//! JSON Schemas published by `evalsig schema` (D-012).

pub const RECORDS: &str = include_str!("../schema/records.schema.json");
pub const LEDGER_ENTRY: &str = include_str!("../schema/ledger-entry.schema.json");

/// Every published schema by name.
pub const ALL: [(&str, &str); 2] = [("records", RECORDS), ("ledger-entry", LEDGER_ENTRY)];

pub fn by_name(name: &str) -> Option<&'static str> {
    ALL.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}
