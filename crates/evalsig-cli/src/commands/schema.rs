//! `evalsig schema <name>`: print a published JSON Schema.

use serde_json::{Value, json};

use evalsig_project::Error;
use evalsig_project::envelope::{Envelope, Output, Schema};
use evalsig_project::exit::Exit;
use evalsig_project::schemas;

fn lookup(name: &str) -> Result<Value, Error> {
    let text = schemas::by_name(name).ok_or_else(|| {
        let names: Vec<&str> = schemas::ALL.iter().map(|(n, _)| *n).collect();
        Error::new(
            "unknown_schema",
            format!("no schema named {name:?}"),
            Exit::Usage,
        )
        .with_hint(format!("available: {}", names.join(", ")))
    })?;
    Ok(serde_json::from_str(text).expect("embedded schema is JSON"))
}

pub fn run(name: &str) -> Result<Output, Error> {
    let schema = lookup(name)?;
    Ok(Output::success(Envelope::new(
        Schema::Schema,
        json!({ "name": name, "id": schema["$id"], "schema": schema }),
    )))
}

/// The schema document alone, exactly as published, for `--raw`.
pub fn raw(name: &str) -> Result<String, Error> {
    lookup(name)?;
    Ok(schemas::by_name(name).expect("looked up above").to_owned())
}

/// A writer module for `records`, for `--emit`.
pub fn emit(name: &str, lang: &str) -> Result<String, Error> {
    if name != "records" {
        return Err(Error::usage(
            "--emit writes records/1 writers; use `schema records --emit <lang>`",
        ));
    }
    schemas::WRITERS
        .iter()
        .find(|(l, _)| *l == lang)
        .map(|(_, text)| (*text).to_owned())
        .ok_or_else(|| {
            Error::usage(format!("no writer for {lang:?}")).with_hint("python or typescript")
        })
}
