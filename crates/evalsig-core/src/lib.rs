//! Data types, metrics and validity checks for evalsig (pure computation, no I/O).

// Published on crates.io: every public item states what it means.
#![warn(missing_docs)]

/// The version of the metric definitions (metrics spec). It changes, with an entry
/// in CHANGELOG.md, whenever a metric's formula or method changes, so a saved result
/// says which definitions produced it (audit spec §5).
pub const SPEC_VERSION: &str = "metrics/1";

pub mod agreement;
pub mod bootstrap;
pub mod capabilities;
pub mod checks;
pub mod compare;
pub mod config;
pub mod cost;
pub mod criteria;
pub mod features;
pub mod gate;
pub mod interval;
pub mod kcurve;
pub mod labels;
pub mod meta;
pub mod missing;
pub mod numeric;
pub mod paired;
pub mod power;
pub mod ppi;
pub mod preference;
pub mod records;
pub mod reference;
pub mod replicates;
pub mod report;
pub mod roc;
pub mod rules;
pub mod score;
pub mod sdt;
pub mod sequential;
pub mod split;
pub mod time;
pub mod validity;
