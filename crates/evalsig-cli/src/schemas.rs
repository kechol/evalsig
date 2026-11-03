//! JSON Schemas published by `evalsig schema` (D-012).

pub const RECORDS: &str = include_str!("../schema/records.schema.json");
pub const LEDGER_ENTRY: &str = include_str!("../schema/ledger-entry.schema.json");
pub const MAPPING: &str = include_str!("../schema/mapping.schema.json");

/// Every published schema by name.
pub const ALL: [(&str, &str); 3] = [
    ("records", RECORDS),
    ("ledger-entry", LEDGER_ENTRY),
    ("mapping", MAPPING),
];

pub fn by_name(name: &str) -> Option<&'static str> {
    ALL.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}
