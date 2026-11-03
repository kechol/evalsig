//! `evalsig map`: onboarding records whose shape evalsig does not know (SPEC §5.3).

use std::path::{Path, PathBuf};

use clap::Subcommand;
use evalsig_adapters::limits::Limits;
use evalsig_adapters::probe::{self, Shape};
use evalsig_adapters::{draft, mapping, maptest};
use evalsig_core::capabilities;
use evalsig_core::config::Config;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Kind};
use serde_json::json;

use crate::commands::import::{parse_size, read_within};
use crate::error::CliError;
use crate::exit::Exit;
use crate::mapped::{self, Bodies};
use crate::output::{Envelope, Next, Output, Verdict, VerdictCode};
use crate::project::{Project, io_error};
use crate::validate::RecordValidator;

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
    /// Show what a mapping loses on a file: coverage, unused paths, key integrity,
    /// value distribution, capabilities, and `<mapping>.tests.jsonl` expectations.
    Test {
        mapping: PathBuf,
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// A value the mapping reads with `{ arg = "NAME" }`; repeat for several.
        #[arg(long = "arg", value_name = "NAME=VALUE")]
        args: Vec<String>,
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

fn probe_file(file: &Path, max_bytes: u64, no_values: bool) -> Result<probe::Probe, CliError> {
    probe_rows(file, max_bytes, no_values).map(|(p, _)| p)
}

/// The probe of a file and the rows it was taken from.
fn probe_rows(
    file: &Path,
    max_bytes: u64,
    no_values: bool,
) -> Result<(probe::Probe, Vec<serde_json::Value>), CliError> {
    let shape = Shape::from_extension(file).ok_or_else(|| {
        CliError::usage(format!("cannot tell the format of {}", file.display()))
            .with_hint("name the file .json, .jsonl, .ndjson, .csv, .eval or .zip")
    })?;
    let limits = Limits {
        max_bytes,
        ..Limits::default()
    };
    let bytes = read_within(file, &limits)?;
    let rows = probe::rows(shape, &bytes, &limits).map_err(|e| CliError::invalid_input(file, e))?;
    Ok((probe::probe(shape, &rows, no_values), rows))
}

fn probe_command(file: &Path, no_values: bool, max_bytes: u64) -> Result<Output, CliError> {
    let p = probe_file(file, max_bytes, no_values)?;
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

/// Whether `path` lies in a `.evalsig` directory, following symbolic links of the
/// parts that exist: the store, the ledger and the configuration are never a mapping's
/// destination.
fn inside_store(path: &Path) -> bool {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let real = abs
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.join(abs.file_name().unwrap_or_default()));
    [Some(abs), real]
        .into_iter()
        .flatten()
        .any(|p| p.components().any(|c| c.as_os_str() == ".evalsig"))
}

fn draft_command(file: &Path, out: &Path, force: bool, max_bytes: u64) -> Result<Output, CliError> {
    if inside_store(out) {
        return Err(CliError::usage(format!(
            "{} is inside .evalsig/, which holds the store, the ledger and the configuration",
            out.display()
        ))
        .with_hint("write the mapping next to the records, for example --out mapping.toml"));
    }
    if out.exists() && !force {
        return Err(CliError::usage(format!("{} exists", out.display()))
            .with_hint("pass --force to replace it"));
    }
    if Shape::from_extension(file) == Some(Shape::Zip) {
        return Err(
            CliError::usage("archives are read by harness adapters, not mappings")
                .with_hint("for an Inspect log: evalsig import --adapter inspect <file>"),
        );
    }
    let (mut p, rows) = probe_rows(file, max_bytes, false)?;
    // A JSON document whose rows sit in one array: draft for the array's elements.
    let mut records_path = None;
    if p.shape == Shape::Json
        && p.rows == 1
        && let Some(rp) = p.records_path_candidates.first().cloned()
        && let Ok(path) = mapping::path::Path::parse(&rp)
    {
        let vars = std::collections::BTreeMap::new();
        let elements = path.eval(mapping::path::Scope {
            row: &rows[0],
            current: None,
            vars: &vars,
        });
        p = probe::probe(Shape::Json, elements, false);
        records_path = Some(rp);
    }
    let name = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("records")
        .to_owned();
    let d = draft::draft(&name, &p, records_path.as_deref());
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

pub fn run(root: &Path, cmd: &MapCommand) -> Result<Output, CliError> {
    match cmd {
        MapCommand::Probe {
            file,
            no_values,
            max_bytes,
        } => probe_command(file, *no_values, *max_bytes),
        MapCommand::Draft {
            file,
            out,
            force,
            max_bytes,
        } => draft_command(file, out, *force, *max_bytes),
        MapCommand::Decide {
            id,
            choice,
            by,
            mapping,
        } => decide(root, id, choice, by, mapping),
        MapCommand::Test {
            mapping,
            files,
            args,
            max_bytes,
        } => test(mapping, files, args, *max_bytes),
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
    let remaining = mapped::compile_text(path, &after)?.undecided;
    // Written beside the file and renamed after the ledger line, so the file never
    // holds a half-written mapping or a choice the ledger lacks.
    let staged = path.with_extension("toml.tmp");
    std::fs::write(&staged, &after).map_err(|e| io_error(&staged, &e))?;
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
    std::fs::rename(&staged, path).map_err(|e| io_error(path, &e))?;
    if id == "positive" {
        let rules: serde_json::Value = serde_json::from_str(choice).expect("checked by decide");
        let line = |k: &str| {
            rules
                .get(k)
                .and_then(|r| toml::Value::try_from(r).ok())
                .map(|v| format!("{k} = {v}"))
        };
        let lines: Vec<String> = ["positive", "decision"]
            .into_iter()
            .filter_map(line)
            .collect();
        envelope.next = vec![Next {
            command: format!(
                "add to [project] in .evalsig/config.toml: {}",
                lines.join("; ")
            ),
            why: "the positive definition and the decision rule live in the configuration".into(),
        }];
    }
    Ok(Output::success(envelope))
}

fn test(
    path: &Path,
    files: &[PathBuf],
    args: &[String],
    max_bytes: u64,
) -> Result<Output, CliError> {
    let text = std::fs::read_to_string(path).map_err(|e| io_error(path, &e))?;
    let compiled = mapped::compile_text(path, &text)?;
    mapped::require_decided(&compiled)?;
    let limits = Limits {
        max_bytes,
        ..Limits::default()
    };
    let config = Config::default();
    let mut host = Bodies::new(args, &config, limits)?;
    let (mut rows, mut records, mut issues) = (Vec::new(), Vec::new(), Vec::new());
    for file in files {
        let bytes = read_within(file, &limits)?;
        // Rows are numbered across the files, after the last number used so far.
        let base = rows.last().map_or(0, |(n, _)| *n);
        let these = mapping::rows(&compiled, &bytes, &limits)
            .map_err(|e| CliError::invalid_input(file, e))?;
        let (r, i) = mapping::apply(&compiled, &these, &mut host)
            .map_err(|e| CliError::invalid_input(file, e))?;
        records.extend(r.into_iter().map(|(n, v)| (base + n, v)));
        issues.extend(i.into_iter().map(|mut x| {
            x.row += base;
            x
        }));
        rows.extend(these.into_iter().map(|(n, v)| (base + n, v)));
    }
    let report = maptest::report(&rows, &records, &issues, &compiled);
    drop(rows);
    let validator = RecordValidator::new();
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for (n, v) in records {
        match validator.check_value(n, v) {
            Ok(r) => valid.push(r),
            Err(found) => invalid.extend(found),
        }
    }
    let n_invalid = invalid.len();
    let invalid: Vec<_> = invalid
        .into_iter()
        .take(20)
        .map(|i| json!({"row": i.line, "code": i.code, "path": i.path, "message": i.message}))
        .collect();
    let tests_path = path.with_extension("tests.jsonl");
    let cases = match std::fs::read_to_string(&tests_path) {
        Ok(t) => Some(
            maptest::cases(&compiled, &t, &mut host)
                .map_err(|e| CliError::invalid_input(&tests_path, e))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_error(&tests_path, &e)),
    };
    let failed = cases.as_ref().is_some_and(|c| !c.failures.is_empty());
    let mut envelope = Envelope::new(
        "evalsig.map.test/1",
        json!({
            "mapping": path.display().to_string(),
            "coverage": report.coverage,
            "unused_paths": report.unused_paths,
            "integrity": report.integrity,
            "values": report.values,
            "invalid_records": n_invalid,
            "invalid_examples": invalid,
            "capabilities": capabilities::of(&valid, None),
            "tests": cases,
        }),
    );
    if !failed {
        return Ok(Output::success(envelope));
    }
    envelope.verdict = Some(Verdict {
        code: VerdictCode::Fail,
        text: format!("Some expectations in {} do not hold.", tests_path.display()),
    });
    Ok(Output {
        envelope,
        exit: Exit::Failed,
    })
}
