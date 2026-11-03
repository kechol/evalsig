//! Zstandard frames, decompressed within a limit.
#![no_main]

use evalsig_adapters::limits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = limits::zstd(data, 1 << 20, "fuzz");
});
