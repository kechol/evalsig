//! Content-hash references `blake3:<hex>` and `sha256:<hex>` (D-005).
//!
//! evalsig writes BLAKE3. sha256 is accepted everywhere a reference is read, so that a
//! third party can verify with nothing but a standard library.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::Digest as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Algorithm {
    Blake3,
    Sha256,
}

impl Algorithm {
    pub fn prefix(self) -> &'static str {
        match self {
            Algorithm::Blake3 => "blake3",
            Algorithm::Sha256 => "sha256",
        }
    }
}

/// A content hash with its algorithm.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentHash {
    algorithm: Algorithm,
    digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a content hash reference (blake3:<64 hex> or sha256:<64 hex>): {0:?}")]
pub struct HashRefError(pub String);

impl ContentHash {
    /// The BLAKE3 reference written by evalsig.
    pub fn blake3(bytes: &[u8]) -> Self {
        ContentHash {
            algorithm: Algorithm::Blake3,
            digest: *blake3::hash(bytes).as_bytes(),
        }
    }

    pub fn sha256(bytes: &[u8]) -> Self {
        ContentHash {
            algorithm: Algorithm::Sha256,
            digest: sha2::Sha256::digest(bytes).into(),
        }
    }

    pub fn of(algorithm: Algorithm, bytes: &[u8]) -> Self {
        match algorithm {
            Algorithm::Blake3 => ContentHash::blake3(bytes),
            Algorithm::Sha256 => ContentHash::sha256(bytes),
        }
    }

    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    pub fn hex(&self) -> String {
        self.digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Whether `bytes` hash to this reference under its own algorithm.
    pub fn matches(&self, bytes: &[u8]) -> bool {
        ContentHash::of(self.algorithm, bytes) == *self
    }
}

/// BLAKE3 of canonical lines joined with LF, without a trailing newline: the hash of
/// a snapshot file (audit spec §1).
pub fn file_hash<I, L>(lines: I) -> ContentHash
where
    I: IntoIterator<Item = L>,
    L: AsRef<[u8]>,
{
    let mut h = blake3::Hasher::new();
    for (i, line) in lines.into_iter().enumerate() {
        if i > 0 {
            h.update(b"\n");
        }
        h.update(line.as_ref());
    }
    ContentHash {
        algorithm: Algorithm::Blake3,
        digest: *h.finalize().as_bytes(),
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.algorithm.prefix(), self.hex())
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for ContentHash {
    type Err = HashRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || HashRefError(s.to_owned());
        let (prefix, hex) = s.split_once(':').ok_or_else(err)?;
        let algorithm = match prefix {
            "blake3" => Algorithm::Blake3,
            "sha256" => Algorithm::Sha256,
            _ => return Err(err()),
        };
        if hex.len() != 64 || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(err());
        }
        let mut digest = [0u8; 32];
        for (i, d) in digest.iter_mut().enumerate() {
            *d = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| err())?;
        }
        Ok(ContentHash { algorithm, digest })
    }
}

impl Serialize for ContentHash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ContentHash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        assert_eq!(
            ContentHash::sha256(b"abc").to_string(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            ContentHash::blake3(b"").to_string(),
            "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn parse_round_trips_and_rejects_bad_references() {
        let h = ContentHash::blake3(b"x");
        assert_eq!(h.to_string().parse::<ContentHash>().unwrap(), h);
        assert!(h.matches(b"x"));
        assert!(!h.matches(b"y"));
        for bad in [
            "blake3:ABC",
            "md5:00",
            "sha256:",
            "blake3:".to_owned().as_str(),
        ] {
            assert!(bad.parse::<ContentHash>().is_err(), "{bad}");
        }
        let upper = format!("sha256:{}", "A".repeat(64));
        assert!(upper.parse::<ContentHash>().is_err());
    }
}
