//! JSON and JSONL through the depth check, the Inspect and promptfoo readers and the
//! example mapping.
#![no_main]

use std::sync::LazyLock;

use evalsig_adapters::limits::{self, Limits};
use evalsig_adapters::mapping::{self, Compiled, Host};
use evalsig_adapters::{inspect, promptfoo};
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

const LIMITS: Limits = Limits {
    max_bytes: 1 << 20,
    max_entries: 64,
    max_depth: 64,
};

static MAPPING: LazyLock<Compiled> = LazyLock::new(|| {
    let text = include_str!("../../examples/mapping.toml");
    mapping::compile(&mapping::parse(text).unwrap()).unwrap()
});

struct Nothing;

impl Host for Nothing {
    fn arg(&self, _: &str) -> Option<&str> {
        Some("x")
    }
    fn read(&mut self, _: &str) -> Result<Vec<u8>, String> {
        Ok(b"{}".to_vec())
    }
    fn body(&mut self, _: &Value) -> Result<String, String> {
        Ok(format!("blake3:{}", "0".repeat(64)))
    }
}

fuzz_target!(|data: &[u8]| {
    if limits::check_json_depth(data, &LIMITS).is_err() {
        return;
    }
    if let Ok((log, samples)) = inspect::read_log(data, false, &LIMITS) {
        let _ = inspect::records(&log, &samples, &inspect::Options::default(), &mut Nothing);
    }
    if let Ok(eval) = promptfoo::parse(data) {
        let _ = promptfoo::records(&eval, &mut Nothing);
    }
    if let Ok(rows) = mapping::rows(&MAPPING, data) {
        let _ = mapping::apply(&MAPPING, &rows, &mut Nothing);
    }
});
