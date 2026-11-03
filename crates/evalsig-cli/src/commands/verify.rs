//! `evalsig verify <report>`: recompute a saved result from the snapshots and the
//! configuration it names, and list every value that differs.

use std::path::{Path, PathBuf};

use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use serde_json::{Value, json};

use crate::commands::{compare, report};
use crate::data;
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode, Warning};
use crate::project::{Project, io_error};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// A saved result (a file under `.evalsig/reports/`, or one produced with --format json).
    pub report: PathBuf,
}

fn invalid(message: impl Into<String>) -> CliError {
    CliError::new("invalid_report", message, Exit::Rejected)
}

/// Paths where two JSON values differ, with both values.
fn differences(path: &str, saved: &Value, recomputed: &Value, out: &mut Vec<Value>) {
    match (saved, recomputed) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for k in keys {
                differences(
                    &format!("{path}/{k}"),
                    a.get(k).unwrap_or(&Value::Null),
                    b.get(k).unwrap_or(&Value::Null),
                    out,
                );
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                let label = x
                    .get("metric")
                    .and_then(Value::as_str)
                    .map_or_else(|| i.to_string(), str::to_owned);
                differences(&format!("{path}/{label}"), x, y, out);
            }
        }
        _ if saved != recomputed => {
            out.push(json!({ "path": path, "saved": saved, "recomputed": recomputed }))
        }
        _ => {}
    }
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let project = Project::open(root)?;
    let text = std::fs::read_to_string(&args.report).map_err(|e| io_error(&args.report, &e))?;
    let saved: Value = serde_json::from_str(&text)
        .map_err(|e| invalid(format!("{}: {e}", args.report.display())))?;
    let prov = &saved["provenance"];
    let request = &prov["request"];
    let snapshots: Vec<ContentHash> = prov["snapshots"]
        .as_array()
        .ok_or_else(|| invalid("the result names no snapshots"))?
        .iter()
        .map(|s| {
            s.as_str()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| invalid(format!("bad snapshot reference {s}")))
        })
        .collect::<Result<_, _>>()?;
    let config_ref: ContentHash = prov["config"]
        .as_str()
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| invalid("the result names no configuration"))?;
    let config = project.stored_config(&config_ref)?;
    let d = data::load_snapshots(&project, snapshots)?;
    let recomputed = match request["command"].as_str() {
        Some("report") => {
            let a: report::Args = serde_json::from_value(request["args"].clone())
                .map_err(|e| invalid(e.to_string()))?;
            report::compute(&d, &config, &a)?.results
        }
        Some("compare") => {
            let a: compare::Args = serde_json::from_value(request["args"].clone())
                .map_err(|e| invalid(e.to_string()))?;
            compare::compute(&d, &config, &a)?.results
        }
        other => return Err(invalid(format!("cannot recompute a result of {other:?}"))),
    };
    let mut mismatches = Vec::new();
    differences("", &saved["results"], &recomputed, &mut mismatches);
    let saved_hash = crate::provenance::output_hash(&saved["results"])?;
    let recomputed_hash = crate::provenance::output_hash(&recomputed)?;
    let matched = mismatches.is_empty() && saved_hash == recomputed_hash;
    let mut envelope = Envelope::new(
        "evalsig.verify/1",
        json!({
            "report": args.report.display().to_string(),
            "match": matched,
            "saved_output": saved_hash,
            "recomputed_output": recomputed_hash,
            "recorded_output": prov["output"],
            "mismatches": mismatches,
        }),
    );
    if prov["tool"].as_str() != Some(crate::tool_version().as_str()) {
        envelope.warnings.push(Warning {
            code: "tool_version_differs".into(),
            message: format!(
                "the result was produced by {}; differences may come from the version",
                prov["tool"]
            ),
        });
    }
    let summary = json!({ "match": matched, "mismatches": envelope.results["mismatches"].as_array().map_or(0, Vec::len) });
    project.record(
        &mut envelope,
        Kind::Verify,
        vec![saved_hash.to_string()],
        vec![recomputed_hash.to_string()],
        summary.as_object().cloned().unwrap_or_default(),
    )?;
    if matched {
        return Ok(Output::success(envelope));
    }
    envelope.verdict = Some(Verdict {
        code: VerdictCode::Fail,
        text: "Recomputing the result from its snapshots and configuration gives different values."
            .into(),
    });
    Ok(Output {
        envelope,
        exit: Exit::Failed,
    })
}
