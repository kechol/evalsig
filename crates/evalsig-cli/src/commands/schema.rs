//! `evalsig schema <name>`: print a published JSON Schema.

use serde_json::{Value, json};

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output};
use crate::schemas;

pub fn run(name: &str) -> Result<Output, CliError> {
    let text = schemas::by_name(name).ok_or_else(|| {
        let names: Vec<&str> = schemas::ALL.iter().map(|(n, _)| *n).collect();
        CliError::new(
            "unknown_schema",
            format!("no schema named {name:?}"),
            Exit::Usage,
        )
        .with_hint(format!("available: {}", names.join(", ")))
    })?;
    let schema: Value = serde_json::from_str(text).expect("embedded schema is JSON");
    Ok(Output::success(Envelope::new(
        "evalsig.schema/1",
        json!({ "name": name, "id": schema["$id"], "schema": schema }),
    )))
}

/// The schema document alone, for `--raw`.
pub fn raw(name: &str) -> Result<String, CliError> {
    run(name).map(|o| {
        let mut s =
            serde_json::to_string_pretty(&o.envelope.results["schema"]).expect("schema serializes");
        s.push('\n');
        s
    })
}
