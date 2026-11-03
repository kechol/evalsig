//! `evalsig import`: read canonical JSONL, flat CSV or mapped records into one
//! snapshot, with its provenance and an `import` line in the agent-side ledger.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use evalsig_adapters::limits::{self, LimitError, Limits};
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
use crate::mapped;
use crate::output::{Envelope, Output, Verdict, VerdictCode, Warning};
use crate::project::{Project, actor, io_error, now};
use crate::validate::{Issue, RecordValidator};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum AdapterArg {
    /// Canonical `records/1` JSONL.
    Records,
    /// A CSV whose columns are record field names.
    Csv,
    /// Inspect logs: `.eval` archives or `.json` logs.
    Inspect,
    /// promptfoo results: `-o out.json` output or the local `.db`.
    Promptfoo,
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
    #[arg(required_unless_present = "from")]
    pub files: Vec<PathBuf>,
    /// Adapter; by default chosen from the file extension (.jsonl/.json, .csv).
    #[arg(long, value_enum)]
    pub adapter: Option<AdapterArg>,
    /// Map the files with a mapping file (`mapping/1`).
    #[arg(long, value_name = "FILE", conflicts_with_all = ["adapter", "from", "converter"])]
    pub mapping: Option<PathBuf>,
    /// A value a mapping reads with `{ arg = "NAME" }`; repeat for several.
    #[arg(long = "arg", value_name = "NAME=VALUE")]
    pub args: Vec<String>,
    /// Apply the `[[import]]` block of this name in the configuration; its `path`
    /// names the files unless files are given.
    #[arg(long, value_name = "NAME", conflicts_with_all = ["adapter", "converter"])]
    pub from: Option<String>,
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
    /// Refuse any file, or decompressed archive, larger than this (bytes, or with a
    /// K, M or G suffix).
    #[arg(long, value_name = "SIZE", default_value = "1G", value_parser = parse_size)]
    pub max_bytes: u64,
}

/// Bytes with an optional binary K, M or G suffix.
pub(crate) fn parse_size(s: &str) -> Result<u64, String> {
    let (digits, shift) = match s.strip_suffix(['K', 'M', 'G']) {
        Some(d) => (
            d,
            match &s[s.len() - 1..] {
                "K" => 10,
                "M" => 20,
                _ => 30,
            },
        ),
        None => (s, 0),
    };
    let n: u64 = digits
        .parse()
        .map_err(|_| format!("{s:?} is not a size such as 1048576, 512M or 2G"))?;
    n.checked_shl(shift)
        .filter(|v| v >> shift == n)
        .ok_or_else(|| format!("{s} is too large"))
}

/// A file read within the limits, JSON checked for depth before parsing.
fn read_within(path: &Path, limits: &Limits, json: bool) -> Result<Vec<u8>, CliError> {
    let refused = |e: LimitError| match e {
        LimitError::Io(io) => io_error(path, &io),
        other => CliError::new(
            "input_limit",
            format!("{}: {other}", path.display()),
            Exit::Rejected,
        )
        .with_hint("raise --max-bytes if the file is trusted"),
    };
    let bytes = limits::read_file(path, limits).map_err(refused)?;
    if json {
        limits::check_json_depth(&bytes, limits).map_err(refused)?;
    }
    Ok(bytes)
}

const MAX_ISSUES: usize = 50;

fn adapter_for(path: &Path, explicit: Option<AdapterArg>) -> Result<AdapterArg, CliError> {
    if let Some(a) = explicit {
        return Ok(a);
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("jsonl" | "json") => Ok(AdapterArg::Records),
        Some("csv") => Ok(AdapterArg::Csv),
        Some("eval") => Ok(AdapterArg::Inspect),
        Some("db") => Ok(AdapterArg::Promptfoo),
        _ => Err(
            CliError::usage(format!("cannot tell the format of {}", path.display()))
                .with_hint("pass --adapter records, csv, inspect or promptfoo"),
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
        AdapterArg::Inspect | AdapterArg::Promptfoo => {
            unreachable!("harness logs are read with their bodies")
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
    let via = mapped::resolve(root, &config, args.mapping.as_deref(), args.from.as_deref())?;
    let store = project.objects();
    let limits = Limits {
        max_bytes: args.max_bytes,
        ..Limits::default()
    };
    let mut bodies = mapped::Bodies::new(
        &args.args,
        (!args.dry_run).then_some(&store),
        &config,
        limits,
    )?;
    let files = match &via {
        Some(v) if args.files.is_empty() => v.files.clone(),
        _ => args.files.clone(),
    };
    let validator = RecordValidator::new();
    let mut records = Vec::new();
    let mut issues: Vec<(String, Issue)> = Vec::new();
    let mut sources = Vec::new();
    let mut adapters = std::collections::BTreeSet::new();
    for path in &files {
        let adapter = match via {
            Some(_) => None,
            None => Some(adapter_for(path, args.adapter)?),
        };
        let json = match (&via, adapter) {
            (Some(v), _) => v.reads_json(),
            (None, a) => a == Some(AdapterArg::Records),
        };
        let bytes = read_within(path, &limits, json)?;
        sources.push(SourceFile {
            path: path.display().to_string(),
            hash: ContentHash::blake3(&bytes),
        });
        if let Some(v) = &via {
            let (mapped, found) = mapped::apply(v, path, &bytes, &mut bodies)?;
            let file = path.display().to_string();
            issues.extend(found.into_iter().map(|i| (file.clone(), i)));
            for (row, value) in mapped {
                match validator.check_value(row, value) {
                    Ok(r) => records.push(r),
                    Err(found) => issues.extend(found.into_iter().map(|i| (file.clone(), i))),
                }
            }
            continue;
        }
        let adapter = adapter.expect("chosen above when there is no mapping");
        adapters.insert(adapter);
        if matches!(adapter, AdapterArg::Inspect | AdapterArg::Promptfoo) {
            let file = path.display().to_string();
            let read = match adapter {
                AdapterArg::Inspect => mapped::inspect_log(
                    path,
                    &bytes,
                    &evalsig_adapters::inspect::Options::default(),
                    &mut bodies,
                )?,
                _ => mapped::promptfoo_results(path, &bytes, &mut bodies)?,
            };
            for (row, value) in read {
                match validator.check_value(row, value) {
                    Ok(r) => records.push(r),
                    Err(found) => issues.extend(found.into_iter().map(|i| (file.clone(), i))),
                }
            }
            continue;
        }
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
    for value in std::mem::take(&mut bodies.contents) {
        match validator.check_value(0, value) {
            Ok(r) => records.push(r),
            Err(found) => issues.extend(found.into_iter().map(|i| ("<content>".into(), i))),
        }
    }
    sources.append(&mut bodies.joined);
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
    let adapter = match &via {
        Some(v) => v.adapter.clone(),
        None => adapters
            .iter()
            .map(|a| a.name())
            .collect::<Vec<_>>()
            .join("+"),
    };
    let mapping = via.as_ref().map(|v| v.hash).or(converter);
    let written = snapshot::write(&store, &mut records, |snapshot| Provenance {
        schema: PROVENANCE_SCHEMA.into(),
        snapshot,
        sources: sources.clone(),
        adapter: adapter.clone(),
        adapter_version: None,
        mapping,
        mapping_decisions: vec![],
        tool: tool.clone(),
        imported_at: imported_at.clone(),
        actor: who.clone(),
        redactions: bodies.redactions.clone(),
    })
    .map_err(CliError::store)?;
    let entries = ledger::read(&project.ledger_path()).map_err(CliError::store)?;
    if snapshot::purged(&entries)
        .map_err(CliError::store)?
        .contains(&written.snapshot)
    {
        // Purge deleted this snapshot, so the object was just written again.
        project
            .objects()
            .remove(&written.snapshot)
            .map_err(CliError::store)?;
        return Err(CliError::new(
            "purged_records",
            "these records were purged from the project; remove the purged text from the source before importing it again",
            Exit::Rejected,
        ));
    }
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
                .chain(mapping.map(|c| c.to_string()))
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
