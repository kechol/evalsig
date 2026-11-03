//! Zip archives as `.eval` logs: entries are read within the limits, and a readable
//! archive goes on through the Inspect reader.
#![no_main]

use evalsig_adapters::inspect;
use evalsig_adapters::limits::{self, Limits};
use libfuzzer_sys::fuzz_target;

const LIMITS: Limits = Limits {
    max_bytes: 1 << 20,
    max_entries: 64,
    max_depth: 64,
};

fuzz_target!(|data: &[u8]| {
    let _ = limits::zip_entries(data, &LIMITS);
    let _ = inspect::read_log(data, true, &LIMITS);
});
