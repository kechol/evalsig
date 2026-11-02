//! `evalsig validate --records <file>`: check a `records/1` file line by line.

use std::path::Path;

use serde_json::json;

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode};
use crate::project::io_error;
use crate::validate::RecordValidator;

pub fn run(records: &Path) -> Result<Output, CliError> {
    let text = std::fs::read_to_string(records).map_err(|e| io_error(records, &e))?;
    let validator = RecordValidator::new();
    let mut lines = 0;
    let mut invalid_lines = 0;
    let mut issues = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        lines += 1;
        let found = validator.validate_line(i + 1, line);
        if !found.is_empty() {
            invalid_lines += 1;
            issues.extend(found);
        }
    }
    let mut envelope = Envelope::new(
        "evalsig.validate/1",
        json!({
            "file": records.display().to_string(),
            "records": lines,
            "invalid_records": invalid_lines,
            "issues": issues,
        }),
    );
    if invalid_lines == 0 {
        return Ok(Output::success(envelope));
    }
    envelope.verdict = Some(Verdict {
        code: VerdictCode::Blocked,
        text: format!("{invalid_lines} of {lines} records do not conform to records/1."),
    });
    Ok(Output {
        envelope,
        exit: Exit::Blocked,
    })
}
