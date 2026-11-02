//! `evalsig validate --records <file>`: check a `records/1` file line by line.

use std::io::BufRead;
use std::path::Path;

use serde_json::json;

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode};
use crate::project::io_error;
use crate::validate::{Issue, RecordValidator};

/// Issues kept in the output; the total is always reported.
const MAX_ISSUES: usize = 1000;

pub fn run(records: &Path) -> Result<Output, CliError> {
    let file = std::fs::File::open(records).map_err(|e| io_error(records, &e))?;
    let mut reader = std::io::BufReader::new(file);
    let validator = RecordValidator::new();
    let (mut lines, mut invalid_lines, mut issues_total) = (0usize, 0usize, 0usize);
    let mut issues = Vec::new();
    let mut buf = Vec::new();
    let mut line_no = 0;
    loop {
        buf.clear();
        if reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| io_error(records, &e))?
            == 0
        {
            break;
        }
        line_no += 1;
        let found = match std::str::from_utf8(&buf) {
            Ok(text) if text.trim().is_empty() => continue,
            Ok(text) => validator
                .check_line(line_no, text.trim_end_matches(['\n', '\r']))
                .err()
                .unwrap_or_default(),
            Err(e) => vec![Issue {
                line: line_no,
                code: "json.invalid".into(),
                path: String::new(),
                message: format!("line is not UTF-8: {e}"),
            }],
        };
        lines += 1;
        if !found.is_empty() {
            invalid_lines += 1;
            issues_total += found.len();
            let room = MAX_ISSUES.saturating_sub(issues.len());
            issues.extend(found.into_iter().take(room));
        }
    }
    let mut envelope = Envelope::new(
        "evalsig.validate/1",
        json!({
            "file": records.display().to_string(),
            "records": lines,
            "invalid_records": invalid_lines,
            "issues": issues,
            "issues_total": issues_total,
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
