//! Gate conditions (`gate.toml`, SPEC §8.3) as a draft and as the referee checks them
//! again at approval, since the agent can write the stored copy.

use std::path::Path;

use evalsig_project::Error;
use evalsig_project::exit::Exit;
use evalsig_project::project::io_error;
use evalsig_project::schemas;
use serde_json::Value;

fn invalid(message: impl Into<String>) -> Error {
    Error::new("invalid_conditions", message, Exit::Rejected)
}

/// The conditions as JSON, checked against `gate/1`; refuses the `resolved` section,
/// which only approval writes.
pub fn parse(path: &Path) -> Result<Value, Error> {
    let text = std::fs::read_to_string(path).map_err(|e| io_error(path, &e))?;
    let toml: toml::Table = toml::from_str(&text)
        .map_err(|e: toml::de::Error| invalid(format!("{}: {e}", path.display())))?;
    let doc = serde_json::to_value(toml).expect("TOML converts to JSON");
    check(&doc, &path.display().to_string())?;
    Ok(doc)
}

/// The checks a draft passes when it is written and again when the referee approves
/// it, since the agent can write the stored copy: no resolved section, the gate.toml
/// schema, and a threshold for every condition that compares against one.
pub fn check(doc: &Value, origin: &str) -> Result<(), Error> {
    if doc.get("resolved").is_some() {
        return Err(invalid(
            "a draft has no resolved section; approval writes it",
        ));
    }
    let schema: Value = serde_json::from_str(schemas::GATE).expect("embedded schema is JSON");
    let validator = jsonschema::options()
        .build(&schema)
        .expect("embedded schema compiles");
    let mut errors: Vec<String> = validator
        .iter_errors(doc)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    for (k, c) in doc["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        if c["rule"] != "pass" && !c["value"].is_number() {
            errors.push(format!(
                "/conditions/{k}: rule {} needs a numeric value",
                c["rule"]
            ));
        }
        // human_parity's value is the alternative annotator test's epsilon (D-017).
        if c["metric"] == "human_parity"
            && c["value"].as_f64().is_some_and(|e| !(e > 0.0 && e < 1.0))
        {
            errors.push(format!(
                "/conditions/{k}: the epsilon of human_parity must lie strictly between 0 and 1"
            ));
        }
        // The gate computes intervals by metrics spec §1.1 only; a named method it would
        // not use is refused rather than ignored.
        if c["interval"].as_str().is_some_and(|m| m != "auto") {
            errors.push(format!(
                "/conditions/{k}: interval {} is not available; use auto",
                c["interval"]
            ));
        }
    }
    if !errors.is_empty() {
        return Err(invalid(format!("{origin}: {}", errors.join("; "))));
    }
    Ok(())
}
