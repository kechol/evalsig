//! JSON Schemas published by `evalsig schema` (D-012).

pub const RECORDS: &str = include_str!("../schema/records.schema.json");
pub const LEDGER_ENTRY: &str = include_str!("../schema/ledger-entry.schema.json");
pub const MAPPING: &str = include_str!("../schema/mapping.schema.json");
pub const GATE: &str = include_str!("../schema/gate.schema.json");
pub const ENVELOPE: &str = include_str!("../schema/envelope.schema.json");

/// Every published schema by name.
pub const ALL: [(&str, &str); 5] = [
    ("records", RECORDS),
    ("ledger-entry", LEDGER_ENTRY),
    ("mapping", MAPPING),
    ("gate", GATE),
    ("envelope", ENVELOPE),
];

pub fn by_name(name: &str) -> Option<&'static str> {
    ALL.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// Standard-library-only writers of `records/1`, by language (`schema records --emit`).
pub const WRITERS: [(&str, &str); 2] = [
    (
        "python",
        include_str!("../schema/writers/evalsig_records.py"),
    ),
    (
        "typescript",
        include_str!("../schema/writers/evalsig_records.ts"),
    ),
];

#[cfg(test)]
mod tests {
    use evalsig_core::records::MissingReason;

    use super::*;

    #[test]
    fn writers_list_every_missing_reason() {
        for (lang, text) in WRITERS {
            for r in MissingReason::ALL {
                let name = serde_json::to_value(r).unwrap();
                assert!(
                    text.contains(&format!("\"{}\"", name.as_str().unwrap())),
                    "{lang}: {name}"
                );
            }
        }
    }
}
