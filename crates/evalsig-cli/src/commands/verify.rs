//! `evalsig verify <report>`: recompute a saved result from the snapshots and the
//! configuration it names, and list every value that differs.

use std::path::{Path, PathBuf};

use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Kind};
use serde_json::{Value, json};

use evalsig_project::Error;
use evalsig_project::envelope::{Envelope, Output, Schema, Verdict, VerdictCode, Warning};
use evalsig_project::exit::Exit;
use evalsig_project::project::{Project, io_error};
use evalsig_project::provenance::{Request, output_hash};
use evalsig_project::{compare, data, report};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// A saved result (a file under `.evalsig/reports/`, or one produced with --format json).
    pub report: PathBuf,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new("invalid_report", message, Exit::Rejected)
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

pub fn run(root: &Path, args: &Args) -> Result<Output, Error> {
    let project = Project::open(root)?;
    let text = std::fs::read_to_string(&args.report).map_err(|e| io_error(&args.report, &e))?;
    let saved: Value = serde_json::from_str(&text)
        .map_err(|e| invalid(format!("{}: {e}", args.report.display())))?;
    let prov = &saved["provenance"];
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
    let request: Request = serde_json::from_value(prov["request"].clone())
        .map_err(|e| invalid(format!("cannot recompute this result: {e}")))?;
    let mut uses_not_recounted = false;
    let recomputed = match &request {
        Request::Report(a) => report::compute(&d, &config, a)?.results,
        Request::Compare(a) => {
            // A comparison on a split names that part by hash; resolve the same one.
            let scope = a
                .split
                .map(|p| {
                    evalsig_project::split::scope(
                        &project,
                        p.part(),
                        saved["results"]["split"].as_str(),
                    )
                })
                .transpose()?;
            let mut c = compare::compute(&d, &config, a, scope.as_ref())?;
            // The use count comes from the ledger lines before the one naming this
            // output. A comparison made with --scratch is in no line, so its count
            // cannot be recounted and is taken as saved.
            let entries = ledger::read(&project.ledger_path()).map_err(Error::store)?;
            let recorded = entries.iter().position(|e| {
                e.outputs
                    .iter()
                    .any(|o| Some(o.as_str()) == prov["output"].as_str())
            });
            if let Some(saved_uses) = saved["results"].get("validation_uses") {
                match recorded {
                    Some(before) => compare::count_uses(
                        &mut c.results,
                        &mut Vec::new(),
                        &entries[..before],
                        config.compare.validation_warn_after,
                    ),
                    None => {
                        c.results["validation_uses"] = saved_uses.clone();
                        uses_not_recounted = true;
                    }
                }
            }
            c.results
        }
    };
    let mut mismatches = Vec::new();
    differences("", &saved["results"], &recomputed, &mut mismatches);
    // `differences` compares every leaf, so no mismatch means equal results.
    let matched = mismatches.is_empty();
    let n_mismatches = mismatches.len();
    let saved_hash = output_hash(&saved["results"])?;
    let recomputed_hash = output_hash(&recomputed)?;
    let mut envelope = Envelope::new(
        Schema::Verify,
        json!({
            "report": args.report.display().to_string(),
            "match": matched,
            "saved_output": saved_hash,
            "recomputed_output": recomputed_hash,
            "recorded_output": prov["output"],
            "mismatches": mismatches,
        }),
    );
    if uses_not_recounted {
        envelope.warnings.push(Warning {
            code: "validation_uses_not_recounted".into(),
            message: "the comparison was not recorded in the ledger, so its validation_uses is taken as saved".into(),
        });
    }
    if prov["spec_version"].as_str() != Some(evalsig_core::SPEC_VERSION) {
        envelope.warnings.push(Warning {
            code: "spec_version_differs".into(),
            message: format!(
                "the result follows metric definitions {}, this evalsig {}; see CHANGELOG.md",
                prov["spec_version"]
                    .as_str()
                    .unwrap_or("of no recorded version"),
                evalsig_core::SPEC_VERSION
            ),
        });
    }
    if prov["tool"].as_str() != Some(evalsig_project::tool_version().as_str()) {
        envelope.warnings.push(Warning {
            code: "tool_version_differs".into(),
            message: format!(
                "the result was produced by {}; differences may come from the version",
                prov["tool"].as_str().unwrap_or("an unrecorded tool")
            ),
        });
    }
    project.record(
        &mut envelope,
        Kind::Verify,
        vec![saved_hash.to_string()],
        vec![recomputed_hash.to_string()],
        json!({ "match": matched, "mismatches": n_mismatches }),
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
