//! Ingestion adapters for evalsig.

/// Where adapters keep the bodies records name (inputs, outputs, references,
/// rationales): the importer stores each under `store.content` and returns its
/// reference.
pub trait BodySink {
    fn body(&mut self, value: &serde_json::Value) -> Result<String, String>;
}

pub mod csv;
pub mod draft;
pub mod inspect;
pub mod limits;
pub mod mapping;
pub mod maptest;
pub mod probe;
pub mod promptfoo;
pub mod source;
pub mod table;

/// Test doubles shared by the adapters' tests.
#[cfg(test)]
pub(crate) mod testing {
    use std::collections::BTreeMap;

    use serde_json::Value as Json;

    /// Keeps bodies in order and names them by position; serves `--arg` values.
    #[derive(Default)]
    pub struct Recorder {
        pub args: BTreeMap<String, String>,
        pub bodies: Vec<Json>,
    }

    impl crate::BodySink for Recorder {
        fn body(&mut self, v: &Json) -> Result<String, String> {
            self.bodies.push(v.clone());
            Ok(format!("blake3:{:064x}", self.bodies.len()))
        }
    }

    impl crate::mapping::Host for Recorder {
        fn arg(&self, name: &str) -> Option<&str> {
            self.args.get(name).map(String::as_str)
        }
        fn read(&mut self, file: &str) -> Result<Vec<u8>, String> {
            Err(format!("{file}: no files in this test"))
        }
    }
}
