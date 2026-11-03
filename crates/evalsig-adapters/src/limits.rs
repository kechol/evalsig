//! Limits for reading files evalsig does not trust (SPEC §13.1): the bytes read or
//! decompressed, the entries of an archive and the nesting depth of JSON. Readers stop
//! at the limit, so an archive bomb costs at most the limit in memory.

use std::io::Read;
use std::path::Path;

/// Default ceiling on the bytes of one file, decompressed archive entries included.
pub const MAX_BYTES: u64 = 1 << 30;
/// Default ceiling on the entries of one archive.
pub const MAX_ENTRIES: usize = 100_000;
/// Default ceiling on the nesting of JSON arrays and objects (below serde_json's own).
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_bytes: u64,
    pub max_entries: usize,
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_bytes: MAX_BYTES,
            max_entries: MAX_ENTRIES,
            max_depth: MAX_DEPTH,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LimitError {
    #[error("{what} is larger than {limit} bytes")]
    TooLarge { what: String, limit: u64 },
    #[error("the archive has more than {limit} entries")]
    TooManyEntries { limit: usize },
    #[error("line {line}: JSON nested deeper than {limit} levels")]
    TooDeep { line: usize, limit: usize },
    #[error("not a readable zip archive: {0}")]
    Zip(String),
    #[error("{what}: compression method {method} is not supported")]
    Method { what: String, method: String },
    #[error("{what}: cannot decompress: {message}")]
    Decompress { what: String, message: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Read at most `limit` bytes; more is an error.
fn bounded(mut r: impl Read, limit: u64, what: &str) -> Result<Vec<u8>, LimitError> {
    let mut out = Vec::new();
    r.by_ref()
        .take(limit.saturating_add(1))
        .read_to_end(&mut out)?;
    if out.len() as u64 > limit {
        return Err(LimitError::TooLarge {
            what: what.to_owned(),
            limit,
        });
    }
    Ok(out)
}

/// A whole file, refused before reading if it is larger than the limit.
pub fn read_file(path: &Path, limits: &Limits) -> Result<Vec<u8>, LimitError> {
    let what = path.display().to_string();
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > limits.max_bytes {
        return Err(LimitError::TooLarge {
            what,
            limit: limits.max_bytes,
        });
    }
    bounded(file, limits.max_bytes, &what)
}

/// Check that a JSON document nests no deeper than the limit, before parsing.
pub fn check_json_depth(bytes: &[u8], limits: &Limits) -> Result<(), LimitError> {
    depth_scan(bytes, limits, false)
}

/// Check that no line of JSON Lines nests deeper than the limit, before parsing. Each
/// line is counted on its own, so one malformed line does not affect the next.
pub fn check_jsonl_depth(bytes: &[u8], limits: &Limits) -> Result<(), LimitError> {
    depth_scan(bytes, limits, true)
}

fn depth_scan(bytes: &[u8], limits: &Limits, per_line: bool) -> Result<(), LimitError> {
    let (mut depth, mut line, mut in_string, mut escaped) = (0usize, 1usize, false, false);
    for &b in bytes {
        // JSON strings cannot hold a raw newline, so one always ends string state.
        if b == b'\n' {
            line += 1;
            in_string = false;
            escaped = false;
            if per_line {
                depth = 0;
            }
            continue;
        }
        if in_string {
            match b {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' | b'{' => {
                depth += 1;
                if depth > limits.max_depth {
                    return Err(LimitError::TooDeep {
                        line,
                        limit: limits.max_depth,
                    });
                }
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

/// Decompress a zstd frame of at most `limit` bytes.
pub fn zstd(bytes: &[u8], limit: u64, what: &str) -> Result<Vec<u8>, LimitError> {
    let decoder =
        ruzstd::decoding::StreamingDecoder::new(bytes).map_err(|e| LimitError::Decompress {
            what: what.to_owned(),
            message: e.to_string(),
        })?;
    bounded(decoder, limit, what).map_err(|e| match e {
        LimitError::Io(io) => LimitError::Decompress {
            what: what.to_owned(),
            message: io.to_string(),
        },
        other => other,
    })
}

/// The entries of a zip archive whose names `want` accepts, decompressed, in archive
/// order; the others are not decompressed. Stored, Deflate and Zstandard entries are
/// read; the entries read together may not exceed `max_bytes`.
pub fn zip_entries(
    bytes: &[u8],
    limits: &Limits,
    want: impl Fn(&str) -> bool,
) -> Result<Vec<(String, Vec<u8>)>, LimitError> {
    let zip_err = |e: zip::result::ZipError| LimitError::Zip(e.to_string());
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(zip_err)?;
    if archive.len() > limits.max_entries {
        return Err(LimitError::TooManyEntries {
            limit: limits.max_entries,
        });
    }
    let mut left = limits.max_bytes;
    let mut out = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let raw = archive.by_index_raw(i).map_err(zip_err)?;
        let name = raw.name().map_err(zip_err)?.into_owned();
        if !want(&name) {
            continue;
        }
        let method = raw.compression();
        let too_large = || LimitError::TooLarge {
            what: "the archive's entries".into(),
            limit: limits.max_bytes,
        };
        // The declared size can lie; it only lets an honest bomb fail early.
        if raw.size() > left {
            return Err(too_large());
        }
        let data = match method {
            zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated => {
                drop(raw);
                let entry = archive.by_index(i).map_err(zip_err)?;
                bounded(entry, left, &name)?
            }
            // Zstandard (method 93): the `zip` crate reads it only with C code, so the
            // raw entry goes to ruzstd.
            m if m == zip::CompressionMethod::ZSTD => {
                let compressed = bounded(raw, limits.max_bytes, &name)?;
                zstd(&compressed, left, &name)?
            }
            _ => {
                return Err(LimitError::Method {
                    what: name,
                    method: format!("{method:?}"),
                });
            }
        };
        left -= data.len() as u64;
        out.push((name, data));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn small() -> Limits {
        Limits {
            max_bytes: 1 << 20,
            max_entries: 4,
            max_depth: 8,
        }
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn a_deflate_bomb_stops_at_the_limit() {
        let zeros = vec![0u8; 16 << 20];
        let bomb = zip_of(&[("big.json", &zeros)]);
        assert!(bomb.len() < 100_000, "{}", bomb.len());
        assert!(matches!(
            zip_entries(&bomb, &small(), |_| true),
            Err(LimitError::TooLarge { .. })
        ));
        let fine = zip_of(&[("a.json", b"{}"), ("b.json", b"[]")]);
        let entries = zip_entries(&fine, &small(), |_| true).unwrap();
        assert_eq!(entries[1], ("b.json".to_owned(), b"[]".to_vec()));
    }

    #[test]
    fn too_many_entries_are_refused() {
        let names: Vec<String> = (0..5).map(|i| format!("{i}.json")).collect();
        let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"{}"[..])).collect();
        assert!(matches!(
            zip_entries(&zip_of(&entries), &small(), |_| true),
            Err(LimitError::TooManyEntries { limit: 4 })
        ));
    }

    #[test]
    fn a_zstd_bomb_stops_at_the_limit() {
        let zeros = vec![0u8; 16 << 20];
        let bomb = ruzstd::encoding::compress_to_vec(
            &zeros[..],
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        assert!(matches!(
            zstd(&bomb, 1 << 20, "x"),
            Err(LimitError::TooLarge { .. })
        ));
        assert_eq!(zstd(&bomb, 16 << 20, "x").unwrap().len(), 16 << 20);
    }

    #[test]
    fn deep_nesting_is_refused_before_parsing() {
        let deep = format!(
            "{{\"a\":1}}\n{}{}",
            "[".repeat(100_000),
            "]".repeat(100_000)
        );
        assert!(matches!(
            check_jsonl_depth(deep.as_bytes(), &small()),
            Err(LimitError::TooDeep { line: 2, limit: 8 })
        ));
        // An unclosed string or bracket on one line does not carry to the next.
        let broken = "{\"a\": \"open\n[[[[[[[[\n{\"b\": 1}\n";
        check_jsonl_depth(broken.as_bytes(), &small()).unwrap();
        let quoted = r#"{"s": "[[[[[[[[[[[[[[[[[[[["}"#;
        check_json_depth(quoted.as_bytes(), &small()).unwrap();
    }

    #[test]
    fn large_files_are_refused_unread() {
        let dir = std::env::temp_dir().join(format!("evalsig-limits-{}", std::process::id()));
        std::fs::write(&dir, vec![b' '; 2 << 20]).unwrap();
        let r = read_file(&dir, &small());
        std::fs::remove_file(&dir).unwrap();
        assert!(matches!(r, Err(LimitError::TooLarge { .. })));
    }
}
