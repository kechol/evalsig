//! The evalsig project (D-029): the `.evalsig/` directory and the data in force, the
//! analyses each command computes, provenance and ledger lines, import, and validation
//! against the published JSON Schemas. It takes typed requests, never command-line
//! words, so the command line and the dashboard compute the same results.

pub mod agree;
pub mod analysis;
pub mod checks;
pub mod compare;
pub mod data;
pub mod envelope;
pub mod error;
pub mod exit;
pub mod explain;
pub mod http;
pub mod list;
pub mod mapped;
pub mod project;
pub mod provenance;
pub mod report;
pub mod schemas;
pub mod split;
pub mod trend;
pub mod validate;

pub use error::Error;

/// The tool and version written into provenance and ledger lines.
pub fn tool_version() -> String {
    format!("evalsig {}", env!("CARGO_PKG_VERSION"))
}
