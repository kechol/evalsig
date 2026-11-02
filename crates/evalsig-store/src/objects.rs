//! Content-addressed immutable objects under `.evalsig/objects/` (D-010).
//!
//! An object is named by the BLAKE3 hash of its uncompressed bytes and stored
//! zstd-compressed. Writing an object that already exists is a no-op, so importing the
//! same data twice stores it once.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::hash::{Algorithm, ContentHash};

#[derive(Debug, thiserror::Error)]
pub enum ObjectError {
    #[error("object {0} not found")]
    NotFound(ContentHash),
    #[error("object {0} is corrupt: its content does not match its name")]
    Corrupt(ContentHash),
    #[error("object {hash} cannot be decompressed: {message}")]
    Decompress { hash: ContentHash, message: String },
    #[error("evalsig stores objects under blake3 names; {0} cannot be looked up")]
    UnsupportedAlgorithm(ContentHash),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone)]
pub struct ObjectStore {
    root: PathBuf,
}

impl ObjectStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ObjectStore { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `objects/blake3/ab/cdef….zst`.
    fn path(&self, hash: &ContentHash) -> Result<PathBuf, ObjectError> {
        if hash.algorithm() != Algorithm::Blake3 {
            return Err(ObjectError::UnsupportedAlgorithm(*hash));
        }
        let hex = hash.hex();
        Ok(self
            .root
            .join("blake3")
            .join(&hex[..2])
            .join(format!("{}.zst", &hex[2..])))
    }

    pub fn contains(&self, hash: &ContentHash) -> bool {
        self.path(hash).is_ok_and(|p| p.is_file())
    }

    /// Store bytes and return their name; `true` if the object was new.
    pub fn put(&self, bytes: &[u8]) -> Result<(ContentHash, bool), ObjectError> {
        let hash = ContentHash::blake3(bytes);
        let path = self.path(&hash)?;
        if path.is_file() {
            return Ok((hash, false));
        }
        let dir = path.parent().expect("object paths have a parent");
        fs::create_dir_all(dir)?;
        let compressed =
            ruzstd::encoding::compress_to_vec(bytes, ruzstd::encoding::CompressionLevel::Fastest);
        // Write to a temporary file in the same directory and rename, so a crash never
        // leaves a truncated object under its final name.
        let mut tmp = tempfile_in(dir)?;
        tmp.1.write_all(&compressed)?;
        tmp.1.sync_all()?;
        drop(tmp.1);
        match fs::rename(&tmp.0, &path) {
            Ok(()) => Ok((hash, true)),
            Err(e) => {
                let _ = fs::remove_file(&tmp.0);
                if path.is_file() {
                    Ok((hash, false))
                } else {
                    Err(e.into())
                }
            }
        }
    }

    /// Read an object and check that its content matches its name.
    pub fn get(&self, hash: &ContentHash) -> Result<Vec<u8>, ObjectError> {
        let path = self.path(hash)?;
        let compressed = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(ObjectError::NotFound(*hash));
            }
            Err(e) => return Err(e.into()),
        };
        let mut decoder =
            ruzstd::decoding::StreamingDecoder::new(&compressed[..]).map_err(|e| {
                ObjectError::Decompress {
                    hash: *hash,
                    message: e.to_string(),
                }
            })?;
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|e| ObjectError::Decompress {
                hash: *hash,
                message: e.to_string(),
            })?;
        if !hash.matches(&out) {
            return Err(ObjectError::Corrupt(*hash));
        }
        Ok(out)
    }

    /// Every stored object name.
    pub fn list(&self) -> Result<Vec<ContentHash>, ObjectError> {
        let mut out = Vec::new();
        let base = self.root.join("blake3");
        if !base.is_dir() {
            return Ok(out);
        }
        for prefix in fs::read_dir(&base)? {
            let prefix = prefix?;
            let p = prefix.file_name().to_string_lossy().into_owned();
            for entry in fs::read_dir(prefix.path())? {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if let Some(rest) = name.strip_suffix(".zst")
                    && let Ok(h) = format!("blake3:{p}{rest}").parse()
                {
                    out.push(h);
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

fn tempfile_in(dir: &Path) -> io::Result<(PathBuf, fs::File)> {
    for attempt in 0u32..100 {
        let path = dir.join(format!(".tmp-{}-{attempt}", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temporary file name",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_get_and_dedupe() {
        let dir = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(dir.path());
        let (h, new) = store.put(b"hello").unwrap();
        assert!(new);
        let (h2, new2) = store.put(b"hello").unwrap();
        assert_eq!(h, h2);
        assert!(!new2);
        assert_eq!(store.get(&h).unwrap(), b"hello");
        assert_eq!(store.list().unwrap(), [h]);
        assert!(store.contains(&h));
    }

    #[test]
    fn corrupt_and_missing_objects_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(dir.path());
        let (h, _) = store.put(b"hello").unwrap();
        let other = ruzstd::encoding::compress_to_vec(
            &b"bye"[..],
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        fs::write(store.path(&h).unwrap(), other).unwrap();
        assert!(matches!(store.get(&h), Err(ObjectError::Corrupt(_))));
        fs::write(store.path(&h).unwrap(), b"not zstd").unwrap();
        assert!(matches!(store.get(&h), Err(ObjectError::Decompress { .. })));
        let absent = ContentHash::blake3(b"absent");
        assert!(matches!(store.get(&absent), Err(ObjectError::NotFound(_))));
        assert!(matches!(
            store.get(&ContentHash::sha256(b"x")),
            Err(ObjectError::UnsupportedAlgorithm(_))
        ));
    }
}
