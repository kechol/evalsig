//! `evalsig import`: read canonical JSONL or flat CSV into one snapshot, with its
//! provenance and an `import` line in the agent-side ledger.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use evalsig_adapters::{csv, table};
use evalsig_core::records::{GraderRef, Record};
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Kind, LedgerSide, NewEntry};
use evalsig_store::rows;
use evalsig_store::snapshot::{self, PROVENANCE_SCHEMA, Provenance, SourceFile};
use serde_json::{Value, json};

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode, Warning};
use crate::project::{Project, actor, io_error, now};
use crate::validate::{Issue, RecordValidator};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AdapterArg {
    /// Canonical `records/1` JSONL.
    Records,
    /// A CSV whose columns are record field names.
    Csv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AsArg {
    Judgment,
    Label,
}

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Files to import into one snapshot.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// Adapter; by default chosen from the file extension (.jsonl/.json, .csv).
    #[arg(long, value_enum)]
    pub adapter: Option<AdapterArg>,
    /// What each CSV row is.
    #[arg(long = "as", value_enum, default_value_t = AsArg::Judgment)]
    pub row: AsArg,
    /// Override `run.system.version` of the imported runs.
    #[arg(long)]
    pub system_version: Option<String>,
    /// Override `run.grader` as `<id>[@<version>]`.
    #[arg(long)]
    pub grader: Option<String>,
    /// A converter script that produced the files; its hash is kept as provenance.
    #[arg(long, value_name = "PATH")]
    pub converter: Option<PathBuf>,
    /// Show what would be imported without writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

const MAX_ISSUES: usize = 50;

fn adapter_for(path: &Path, explicit: Option<AdapterArg>) -> Result<AdapterArg, CliError> {
    if let Some(a) = explicit {
        return Ok(a);
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("jsonl" | "json") => Ok(AdapterArg::Records),
        Some("csv") => Ok(AdapterArg::Csv),
        _ => Err(
            CliError::usage(format!("cannot tell the format of {}", path.display()))
                .with_hint("pass --adapter records or --adapter csv"),
        ),
    }
}

fn read_file(path: &Path) -> Result<(String, ContentHash), CliError> {
    let bytes = std::fs::read(path).map_err(|e| io_error(path, &e))?;
    let hash = ContentHash::blake3(&bytes);
    let text = String::from_utf8(bytes).map_err(|_| {
        CliError::new(
            "invalid_input",
            format!("{} is not UTF-8", path.display()),
            Exit::Blocked,
        )
    })?;
    Ok((text, hash))
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let project = Project::open(root)?;
    project.config()?;
    let validator = RecordValidator::new();
    let mut records = Vec::new();
    let mut issues: Vec<(String, Issue)> = Vec::new();
    let mut sources = Vec::new();
    let mut adapters = Vec::new();
    for path in &args.files {
        let adapter = adapter_for(path, args.adapter)?;
        let (text, hash) = read_file(path)?;
        sources.push(SourceFile {
            path: path.display().to_string(),
            hash,
        });
        let name = match adapter {
            AdapterArg::Records => "records",
            AdapterArg::Csv => "csv",
        };
        if !adapters.contains(&name) {
            adapters.push(name);
        }
        match adapter {
            AdapterArg::Records => {
                for (i, line) in text.lines().enumerate() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let found = validator.validate_line(i + 1, line);
                    if found.is_empty() {
                        records.push(Record::from_json(line).expect("validated line parses"));
                    } else {
                        issues.extend(found.into_iter().map(|f| (path.display().to_string(), f)));
                    }
                }
            }
            AdapterArg::Csv => {
                let parsed = csv::parse(&text).map_err(|e| {
                    CliError::new(
                        "invalid_input",
                        format!("{}: {e}", path.display()),
                        Exit::Blocked,
                    )
                })?;
                let kind = match args.row {
                    AsArg::Judgment => table::RowKind::Judgment,
                    AsArg::Label => table::RowKind::Label,
                };
                let rows = table::to_records(&parsed, kind).map_err(|e| {
                    CliError::new(
                        "invalid_input",
                        format!("{}: {e}", path.display()),
                        Exit::Blocked,
                    )
                })?;
                // Rows pass the same schema and record checks as canonical records.
                for (line, value) in rows {
                    let text = value.to_string();
                    let found = validator.validate_line(line, &text);
                    if found.is_empty() {
                        records.push(Record::from_value(value).expect("validated row parses"));
                    } else {
                        issues.extend(found.into_iter().map(|f| (path.display().to_string(), f)));
                    }
                }
            }
        }
    }
    let mut warnings = Vec::new();
    apply_overrides(&mut records, args, &mut warnings)?;
    rows::sort(&mut records);
    records.dedup();
    for w in records.windows(2) {
        if rows::compare(&w[0], &w[1]).is_eq() {
            issues.push((
                "<snapshot>".into(),
                Issue {
                    line: 0,
                    code: "record.duplicate_key".into(),
                    path: String::new(),
                    message: format!(
                        "two different records share a primary key: {}",
                        key_text(&w[1])
                    ),
                },
            ));
        }
    }
    if !issues.is_empty() {
        return Ok(blocked(issues));
    }
    let counts = counts(&records);
    let converter = match &args.converter {
        Some(p) => Some(ContentHash::blake3(
            &std::fs::read(p).map_err(|e| io_error(p, &e))?,
        )),
        None => None,
    };
    let mut envelope = Envelope::new("evalsig.import/1", Value::Null);
    envelope.warnings = warnings;
    if args.dry_run {
        let (_, hash) = rows::canonical_lines(&mut records.clone())
            .map_err(|e| CliError::new("invalid_input", e.to_string(), Exit::Blocked))?;
        envelope.results =
            json!({ "dry_run": true, "snapshot": hash, "records": counts, "sources": sources });
        return Ok(Output::success(envelope));
    }
    let tool = crate::tool_version();
    let imported_at = now();
    let who = actor();
    let adapter = adapters.join("+");
    let store = project.objects();
    let written = snapshot::write(&store, records, |snapshot| Provenance {
        schema: PROVENANCE_SCHEMA.into(),
        snapshot,
        sources: sources.clone(),
        adapter: adapter.clone(),
        adapter_version: None,
        mapping: converter,
        mapping_decisions: vec![],
        tool: tool.clone(),
        imported_at: imported_at.clone(),
        actor: who.clone(),
    })
    .map_err(|e| CliError::new("store", e.to_string(), Exit::Blocked))?;
    let mut summary = serde_json::Map::new();
    summary.insert("adapter".into(), json!(adapter));
    for (k, v) in &counts {
        summary.insert((*k).to_owned(), json!(v));
    }
    let entry = ledger::append(
        &project.ledger_path(),
        LedgerSide::Agent,
        Kind::Import,
        NewEntry {
            ts: imported_at,
            actor: who,
            inputs: sources
                .iter()
                .map(|s| s.hash.to_string())
                .chain(converter.map(|c| c.to_string()))
                .collect(),
            outputs: vec![written.snapshot.to_string(), written.provenance.to_string()],
            summary,
            tool,
        },
    )
    .map_err(|e| CliError::new("ledger", e.to_string(), Exit::Blocked))?;
    envelope.results = json!({
        "snapshot": written.snapshot,
        "provenance": written.provenance,
        "new_objects": written.new_objects,
        "records": counts,
        "sources": sources,
    });
    envelope.provenance.snapshots = vec![written.snapshot.to_string()];
    envelope.provenance.ledger = Some("agent".into());
    envelope.provenance.ledger_seq = Some(entry.seq);
    Ok(Output::success(envelope))
}

fn apply_overrides(
    records: &mut [Record],
    args: &Args,
    warnings: &mut Vec<Warning>,
) -> Result<(), CliError> {
    let grader = match &args.grader {
        Some(g) => {
            let (id, version) = match g.split_once('@') {
                Some((i, v)) => (i, Some(v.to_owned())),
                None => (g.as_str(), None),
            };
            if id.is_empty() {
                return Err(CliError::usage("--grader needs an id: <id>[@<version>]"));
            }
            Some((id.to_owned(), version))
        }
        None => None,
    };
    let mut runs = 0;
    for r in records.iter_mut() {
        if let Record::Run(run) = r {
            runs += 1;
            if let Some(v) = &args.system_version {
                run.system.version = v.clone();
            }
            if let Some((id, version)) = &grader {
                let g = run.grader.get_or_insert_with(GraderRef::default);
                g.id = Some(id.clone());
                g.version = version.clone();
            }
        }
    }
    if runs == 0 && (args.system_version.is_some() || grader.is_some()) {
        warnings.push(Warning {
            code: "override_without_run".into(),
            message: "--system-version and --grader apply to run records, and none were imported"
                .into(),
        });
    }
    Ok(())
}

fn key_text(r: &Record) -> String {
    let mut v = serde_json::to_value(r).expect("records serialize");
    if let Value::Object(m) = &mut v {
        m.retain(|k, _| {
            matches!(
                k.as_str(),
                "record"
                    | "item_id"
                    | "run_id"
                    | "rater_id"
                    | "task"
                    | "version"
                    | "gen_replicate"
                    | "grade_replicate"
            )
        });
    }
    v.to_string()
}

fn counts(records: &[Record]) -> BTreeMap<&'static str, u64> {
    let mut c = BTreeMap::new();
    for r in records {
        let k = match r {
            Record::Item(_) => "items",
            Record::Label(_) => "labels",
            Record::Judgment(_) => "judgments",
            Record::Preference(_) => "preferences",
            Record::Run(_) => "runs",
            Record::Content(_) => "contents",
        };
        *c.entry(k).or_insert(0) += 1;
    }
    c
}

fn blocked(issues: Vec<(String, Issue)>) -> Output {
    let total = issues.len();
    let shown: Vec<Value> = issues
        .iter()
        .take(MAX_ISSUES)
        .map(|(file, i)| json!({"file": file, "line": i.line, "code": i.code, "path": i.path, "message": i.message}))
        .collect();
    let mut envelope = Envelope::new(
        "evalsig.import/1",
        json!({ "issues": shown, "issues_total": total }),
    );
    envelope.verdict = Some(Verdict {
        code: VerdictCode::Blocked,
        text: format!("{total} problems in the input; nothing was imported."),
    });
    Output {
        envelope,
        exit: Exit::Blocked,
    }
}
