use evalsig_store::canonical::{CanonicalError, canonicalize};
use evalsig_store::hash::{ContentHash, file_hash};
use serde_json::Value;

const VECTORS: &str = include_str!("../../../tests/fixtures/canonicalization-vectors.json");

fn doc() -> Value {
    serde_json::from_str(VECTORS).unwrap()
}

#[test]
fn every_valid_vector_matches_exactly() {
    let doc = doc();
    let vectors = doc["vectors"].as_array().unwrap();
    assert!(vectors.len() >= 13);
    for v in vectors {
        let name = v["name"].as_str().unwrap();
        let bytes =
            canonicalize(v["input"].as_str().unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            v["canonical"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            ContentHash::sha256(&bytes).to_string(),
            v["sha256"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            ContentHash::blake3(&bytes).to_string(),
            v["blake3"].as_str().unwrap(),
            "{name}"
        );
    }
}

#[test]
fn every_invalid_vector_is_rejected_for_its_reason() {
    let doc = doc();
    for v in doc["invalid"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        let err = canonicalize(v["input"].as_str().unwrap()).expect_err(name);
        let ok = match name {
            "duplicate_key" => matches!(err, CanonicalError::DuplicateKey(_)),
            "duplicate_key_after_nfc" => matches!(err, CanonicalError::DuplicateKeyAfterNfc(_)),
            "integer_outside_exact_range" => matches!(err, CanonicalError::IntegerOutOfRange(_)),
            "nan_literal" | "infinity_literal" | "lone_surrogate" => {
                matches!(err, CanonicalError::Json(_))
            }
            other => panic!("unknown invalid vector {other}"),
        };
        assert!(ok, "{name}: {err:?}");
    }
}

#[test]
fn file_hashes_match() {
    let doc = doc();
    for f in doc["files"].as_array().unwrap() {
        let lines: Vec<&str> = f["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l.as_str().unwrap())
            .collect();
        assert_eq!(file_hash(&lines).to_string(), f["blake3"].as_str().unwrap());
        assert_eq!(
            ContentHash::sha256(lines.join("\n").as_bytes()).to_string(),
            f["sha256"].as_str().unwrap()
        );
    }
}
