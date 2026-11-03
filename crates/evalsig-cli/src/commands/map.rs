//! `evalsig map`: onboarding records whose shape evalsig does not know (SPEC §5.3).

use std::path::{Path, PathBuf};

use clap::Subcommand;
use evalsig_adapters::draft;
use evalsig_adapters::limits::Limits;
use evalsig_adapters::mapping;
use evalsig_adapters::probe::{self, Shape};
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Kind};
use serde_json::json;

use crate::commands::import::parse_size;
use crate::error::CliError;
use crate::output::{Envelope, Next, Output, Verdict, VerdictCode};
use crate::project::{Project, io_error};

#[derive(Debug, Subcommand)]
pub enum MapCommand {
    /// Show the shape of a file: per path, types, counts and distinct values, with key
    /// and replicate candidates. Free text and secret-like values are never printed.
    Probe {
        file: PathBuf,
        /// List no values at all, not even short categorical ones.
        #[arg(long)]
        no_values: bool,
        /// Refuse a file, or decompressed archive, larger than this.
        #[arg(long, value_name = "SIZE", default_value = "1G", value_parser = parse_size)]
        max_bytes: u64,
    },
    /// Draft a mapping for a file. Choices that change results stay `DECIDE`, listed
    /// with their options and effects under `decisions_needed`.
    Draft {
        file: PathBuf,
        /// Where to write the mapping.
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Replace an existing file.
        #[arg(long)]
        force: bool,
        #[arg(long, value_name = "SIZE", default_value = "1G", value_parser = parse_size)]
        max_bytes: u64,
    },
    /// Record a person's choice for one decision of a mapping, in the file and as a
    /// `mapping_decision` ledger line.
    Decide {
        /// The decision: item_key, value_semantics, value, output_type, value.none,
        /// replicate_kind, missing.reason or positive.
        id: String,
        /// The chosen option, as `decisions_needed` describes it.
        choice: String,
        /// Who decided (for example user:alice); kept in the file and the ledger.
        #[arg(long, value_name = "WHO")]
        by: String,
        /// The mapping file.
        #[arg(long, value_name = "FILE", default_value = "mapping.toml")]
        mapping: PathBuf,
    },
}

fn shape_of(path: &Path) -> Result<Shape, CliError> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("eval" | "zip") => Ok(Shape::Zip),
        Some("json") => Ok(Shape::Json),
        Some("jsonl" | "ndjson") => Ok(Shape::Jsonl),
        Some("csv") => Ok(Shape::Csv),
        _ => Err(
            CliError::usage(format!("cannot tell the format of {}", path.display()))
                .with_hint("name the file .json, .jsonl, .csv or .eval"),
        ),
    }
}

fn probe_file(file: &Path, max_bytes: u64, no_values: bool) -> Result<probe::Probe, CliError> {
    let shape = shape_of(file)?;
    let limits = Limits {
        max_bytes,
        ..Limits::default()
    };
    let bytes = evalsig_adapters::limits::read_file(file, &limits)
        .map_err(|e| CliError::invalid_input(file, e))?;
    let rows = probe::rows(shape, &bytes, &limits).map_err(|e| CliError::invalid_input(file, e))?;
    Ok(probe::probe(shape, &rows, no_values))
}

pub fn run(root: &Path, cmd: &MapCommand) -> Result<Output, CliError> {
    match cmd {
        MapCommand::Probe {
            file,
            no_values,
            max_bytes,
        } => {
            let p = probe_file(file, *max_bytes, *no_values)?;
            let mut envelope = Envelope::new(
                "evalsig.map.probe/1",
                json!({ "file": file.display().to_string(), "probe": p }),
            );
            envelope.next = vec![
                match p.known.as_ref().and_then(|k| k.adapter.map(|a| (a, k))) {
                    Some((a, k)) => Next {
                        command: format!("evalsig import --adapter {a} {}", file.display()),
                        why: k.hint.into(),
                    },
                    None => Next {
                        command: format!("evalsig map draft {} --out mapping.toml", file.display()),
                        why: "draft a mapping; its choices are left to a person".into(),
                    },
                },
            ];
            Ok(Output::success(envelope))
        }
        MapCommand::Draft {
            file,
            out,
            force,
            max_bytes,
        } => {
            if out.exists() && !force {
                return Err(CliError::usage(format!("{} exists", out.display()))
                    .with_hint("pass --force to replace it"));
            }
            let p = probe_file(file, *max_bytes, false)?;
            let name = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("records")
                .to_owned();
            let d = draft::draft(&name, &p);
            let text = toml::to_string(&d.mapping).expect("tables serialize");
            std::fs::write(out, text).map_err(|e| io_error(out, &e))?;
            let n = d.decisions_needed.len();
            let mut envelope = Envelope::new(
                "evalsig.map.draft/1",
                json!({
                    "source": {"path": file.display().to_string(), "format": p.shape, "rows": p.rows},
                    "detected_format": p.known,
                    "mapping_written": out.display().to_string(),
                    "candidates": {
                        "item_key": p.key_candidates,
                        "replicate": p.replicate_candidates,
                        "records_path": p.records_path_candidates,
                    },
                    "decisions_needed": d.decisions_needed,
                }),
            );
            envelope.verdict = Some(Verdict {
                code: VerdictCode::NeedsDecisions,
                text: format!(
                    "Drafted {}. {n} choices that change results are left to a person: show them the options and effects instead of choosing.",
                    out.display()
                ),
            });
            envelope.next = vec![Next {
                command: format!(
                    "evalsig map decide item_key '<path or template>' --by user:<name> --mapping {}",
                    out.display()
                ),
                why: "record the answer a person chose; an agent does not choose".into(),
            }];
            Ok(Output::success(envelope))
        }
        MapCommand::Decide {
            id,
            choice,
            by,
            mapping,
        } => decide(root, id, choice, by, mapping),
    }
}

fn decide(root: &Path, id: &str, choice: &str, by: &str, path: &Path) -> Result<Output, CliError> {
    if by.trim().is_empty() {
        return Err(CliError::usage("--by names who decided"));
    }
    let project = Project::open(root)?;
    let before = std::fs::read(path).map_err(|e| io_error(path, &e))?;
    let text =
        std::str::from_utf8(&before).map_err(|_| CliError::invalid_input(path, "not UTF-8"))?;
    let mut table: toml::Table =
        toml::from_str(text).map_err(|e| CliError::invalid_input(path, e))?;
    let seq = ledger::read(&project.ledger_path())
        .map_err(CliError::store)?
        .len() as u64;
    draft::decide(&mut table, id, choice, by, seq).map_err(CliError::usage)?;
    let after = toml::to_string(&table).expect("tables serialize");
    let remaining = mapping::parse(&after)
        .and_then(|m| mapping::compile(&m))
        .map(|c| c.undecided)
        .map_err(|e| CliError::invalid_input(path, e))?;
    let mut envelope = Envelope::new(
        "evalsig.map.decide/1",
        json!({
            "mapping": path.display().to_string(),
            "id": id,
            "choice": choice,
            "decided_by": by,
            "remaining": remaining,
        }),
    );
    project.record(
        &mut envelope,
        Kind::MappingDecision,
        vec![ContentHash::blake3(&before).to_string()],
        vec![ContentHash::blake3(after.as_bytes()).to_string()],
        json!({"mapping": path.display().to_string(), "id": id, "choice": choice, "decided_by": by}),
    )?;
    std::fs::write(path, after).map_err(|e| io_error(path, &e))?;
    if id == "positive" {
        envelope.next = vec![Next {
            command: "edit [project] positive and decision in .evalsig/config.toml".into(),
            why: "the positive definition and the decision rule live in the configuration".into(),
        }];
    }
    Ok(Output::success(envelope))
}
