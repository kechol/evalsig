//! `evalsig import`: read canonical JSONL or flat CSV into one snapshot, with its
//! provenance and an `import` line in the agent-side ledger.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use evalsig_adapters::{csv, table};
use evalsig_core::config::ContentMode;
use evalsig_core::records::{GraderRef, Record};
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Kind, LedgerSide, NewEntry};
use evalsig_store::snapshot::{self, PROVENANCE_SCHEMA, Provenance, SourceFile};
use evalsig_store::{content, rows};
use serde_json::{Value, json};

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode, Warning};
use crate::project::{Project, actor, io_error, now};
use crate::validate::{Issue, RecordValidator};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum AdapterArg {
    /// Canonical `records/1` JSONL.
    Records,
    /// A CSV whose columns are record field names.
    Csv,
}

impl AdapterArg {
    fn name(self) -> String {
        self.to_possible_value()
            .expect("no skipped variants")
            .get_name()
            .to_owned()
    }
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

/// Where each candidate record came from, before validation.
enum Candidate<'a> {
    Line(&'a str),
    Row(Value),
}

/// Split a file into numbered candidate records for its adapter.
fn candidates<'a>(
    adapter: AdapterArg,
    text: &'a str,
    path: &Path,
    row: AsArg,
) -> Result<Vec<(usize, Candidate<'a>)>, CliError> {
    match adapter {
        AdapterArg::Records => Ok(text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, l)| (i + 1, Candidate::Line(l)))
            .collect()),
        AdapterArg::Csv => {
            let parsed = csv::parse(text).map_err(|e| CliError::invalid_input(path, e))?;
            let kind = match row {
                AsArg::Judgment => table::RowKind::Judgment,
                AsArg::Label => table::RowKind::Label,
            };
            let rows =
                table::to_records(&parsed, kind).map_err(|e| CliError::invalid_input(path, e))?;
            Ok(rows
                .into_iter()
                .map(|(line, v)| (line, Candidate::Row(v)))
                .collect())
        }
    }
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let grader = parse_grader(args.grader.as_deref())?;
    if args.system_version.as_deref() == Some("") {
        return Err(CliError::usage("--system-version cannot be empty"));
    }
    let project = Project::open(root)?;
    let config = project.config()?;
    let redactor = (config.store.content == ContentMode::Redact)
        .then(|| content::Redactor::new(&config.store.redact_patterns))
        .transpose()
        .map_err(|e| CliError::new("invalid_config", e.to_string(), Exit::Rejected))?;
    let validator = RecordValidator::new();
    let mut records = Vec::new();
    let mut issues: Vec<(String, Issue)> = Vec::new();
    let mut sources = Vec::new();
    let mut adapters = std::collections::BTreeSet::new();
    for path in &args.files {
        let adapter = adapter_for(path, args.adapter)?;
        adapters.insert(adapter);
        let bytes = std::fs::read(path).map_err(|e| io_error(path, &e))?;
        sources.push(SourceFile {
            path: path.display().to_string(),
            hash: ContentHash::blake3(&bytes),
        });
        let text =
            String::from_utf8(bytes).map_err(|_| CliError::invalid_input(path, "not UTF-8"))?;
        for (line, candidate) in candidates(adapter, &text, path, args.row)? {
            let checked = match candidate {
                Candidate::Line(l) => validator.check_line(line, l),
                Candidate::Row(v) => validator.check_value(line, v),
            };
            match checked {
                Ok(r) => records.push(r),
                Err(found) => {
                    issues.extend(found.into_iter().map(|f| (path.display().to_string(), f)))
                }
            }
        }
    }
    let mut warnings = Vec::new();
    apply_overrides(
        &mut records,
        args.system_version.as_deref(),
        grader.as_ref(),
        &mut warnings,
    );
    let redacted = redactor
        .as_ref()
        .map(|r| content::redact_records(&mut records, r));
    for key in rows::dedup_and_conflicts(&mut records) {
        issues.push((
            "<snapshot>".into(),
            Issue {
                line: 0,
                code: "record.duplicate_key".into(),
                path: String::new(),
                message: format!("two different records share a primary key: {key}"),
            },
        ));
    }
    if !issues.is_empty() {
        return Ok(blocked(issues));
    }
    let counts = counts(&records);
    let converter = args
        .converter
        .as_deref()
        .map(|p| {
            std::fs::read(p)
                .map(|b| ContentHash::blake3(&b))
                .map_err(|e| io_error(p, &e))
        })
        .transpose()?;
    let mut envelope = Envelope::new("evalsig.import/1", Value::Null);
    envelope.warnings = warnings;
    if args.dry_run {
        let (_, hash) = rows::canonical_lines(&mut records).map_err(CliError::store)?;
        envelope.results =
            json!({ "dry_run": true, "snapshot": hash, "records": counts, "sources": sources });
        return Ok(Output::success(envelope));
    }
    let tool = crate::tool_version();
    let imported_at = now();
    let who = actor();
    let adapter = adapters
        .iter()
        .map(|a| a.name())
        .collect::<Vec<_>>()
        .join("+");
    let written = snapshot::write(&project.objects(), records, |snapshot| Provenance {
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
        redactions: vec![],
    })
    .map_err(CliError::store)?;
    let mut summary = serde_json::Map::new();
    summary.insert("adapter".into(), json!(adapter));
    if let Some(n) = redacted {
        summary.insert("redacted".into(), json!(n));
    }
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
    .map_err(CliError::store)?;
    envelope.results = json!({
        "snapshot": written.snapshot,
        "provenance": written.provenance,
        "new_objects": written.new_objects,
        "records": counts,
        "sources": sources,
    });
    if let Some(n) = redacted {
        envelope.results["redacted"] = json!(n);
    }
    envelope.provenance.snapshots = vec![written.snapshot.to_string()];
    envelope.provenance.ledger = Some("agent".into());
    envelope.provenance.ledger_seq = Some(entry.seq);
    Ok(Output::success(envelope))
}

/// `<id>[@<version>]`, both parts non-empty when present.
fn parse_grader(arg: Option<&str>) -> Result<Option<(String, Option<String>)>, CliError> {
    let Some(g) = arg else { return Ok(None) };
    let (id, version) = match g.split_once('@') {
        Some((i, v)) => (i, Some(v)),
        None => (g, None),
    };
    if id.is_empty() || version == Some("") {
        return Err(CliError::usage(
            "--grader takes <id>[@<version>] with non-empty parts",
        ));
    }
    Ok(Some((id.to_owned(), version.map(str::to_owned))))
}

fn apply_overrides(
    records: &mut [Record],
    system_version: Option<&str>,
    grader: Option<&(String, Option<String>)>,
    warnings: &mut Vec<Warning>,
) {
    let mut runs = 0;
    for r in records.iter_mut() {
        if let Record::Run(run) = r {
            runs += 1;
            if let Some(v) = system_version {
                run.system.version = v.to_owned();
            }
            if let Some((id, version)) = grader {
                let g = run.grader.get_or_insert_with(GraderRef::default);
                g.id = Some(id.clone());
                g.version = version.clone();
            }
        }
    }
    if runs == 0 && (system_version.is_some() || grader.is_some()) {
        warnings.push(Warning {
            code: "override_without_run".into(),
            message: "--system-version and --grader apply to run records, and none were imported"
                .into(),
        });
    }
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
