//! `evalsig report <run>`: the default-tier metrics of one run.

use std::collections::BTreeMap;
use std::path::Path;

use evalsig_core::config::{Config, Estimand, ReferenceMode};
use evalsig_core::labels::LabelSet;
use evalsig_core::records::{DEFAULT_TASK, Item, Judgment, Label, Record};
use evalsig_core::report::{self, Settings};
use evalsig_core::time::Instant;
use evalsig_core::validity;
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Kind, LedgerSide, NewEntry};
use serde_json::{Value, json};

use crate::checks::{self, ArmArgs, DatasetArgs};
use crate::data;
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output};
use crate::project::{Project, actor, io_error, now};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The run to report.
    pub run: String,
    /// The judgment task.
    #[arg(long, default_value = DEFAULT_TASK)]
    pub task: String,
    /// The estimand: single_shot, majority@k, any@k or all@k (default from the configuration).
    #[arg(long)]
    pub estimand: Option<Estimand>,
    /// Decision threshold for score outputs; for score runs, also reports the share at or above it.
    #[arg(long)]
    pub threshold: Option<f64>,
    /// Use the labels as they were at this RFC 3339 time.
    #[arg(long, value_name = "TIME")]
    pub labels_as_of: Option<String>,
    /// Also report each value of this stratum.
    #[arg(long, value_name = "STRATUM")]
    pub by: Option<String>,
    /// Include one row per item.
    #[arg(long)]
    pub items: bool,
    /// How several raters' labels become the reference (default from the configuration).
    #[arg(long, value_enum)]
    pub reference: Option<ReferenceArg>,
    #[command(flatten)]
    pub checks: DatasetArgs,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ReferenceArg {
    Single,
    Majority,
    Soft,
    Latent,
}

impl From<ReferenceArg> for ReferenceMode {
    fn from(r: ReferenceArg) -> Self {
        match r {
            ReferenceArg::Single => ReferenceMode::Single,
            ReferenceArg::Majority => ReferenceMode::Majority,
            ReferenceArg::Soft => ReferenceMode::Soft,
            ReferenceArg::Latent => ReferenceMode::Latent,
        }
    }
}

/// The hash of the configuration with every default filled in (D-004).
pub fn config_hash(config: &Config) -> ContentHash {
    ContentHash::blake3(
        &to_canonical_bytes(&config.to_json()).expect("configuration JSON is canonical"),
    )
}

/// Round every number to 10 decimal places (metrics spec §0).
pub fn round_numbers(v: Value) -> Value {
    match v {
        Value::Number(n) if n.is_f64() => {
            let x = evalsig_store::canonical::round_output(n.as_f64().expect("f64 number"));
            serde_json::Number::from_f64(x).map_or(Value::Null, Value::Number)
        }
        Value::Array(xs) => Value::Array(xs.into_iter().map(round_numbers).collect()),
        Value::Object(m) => {
            Value::Object(m.into_iter().map(|(k, v)| (k, round_numbers(v))).collect())
        }
        other => other,
    }
}

/// Select the labels in force: the latest revisions, or those as of a time.
pub fn select_labels<'a>(
    records: &'a [Record],
    as_of: Option<&str>,
) -> Result<(Vec<&'a Label>, usize), CliError> {
    let mut set = LabelSet::default();
    for r in records {
        if let Record::Label(l) = r {
            set.insert(l.clone())
                .map_err(|e| CliError::new("invalid_labels", e.to_string(), Exit::Blocked))?;
        }
    }
    let ids: Vec<(String, String, String, u32, u32)> = match as_of {
        None => set.latest().into_iter().map(key).collect(),
        Some(t) => {
            let at = Instant::parse(t).map_err(|e| {
                CliError::usage(format!("--labels-as-of: {e}"))
                    .with_hint("use RFC 3339, e.g. 2026-09-01T00:00:00Z")
            })?;
            let chosen = set.as_of(at);
            let skipped = chosen.undated_skipped;
            let keys: Vec<_> = chosen.labels.into_iter().map(key).collect();
            return Ok((pick(records, &keys), skipped));
        }
    };
    Ok((pick(records, &ids), 0))
}

fn key(l: &Label) -> (String, String, String, u32, u32) {
    (
        l.item_id.clone(),
        l.rater_id.clone(),
        l.task().to_owned(),
        l.label_replicate(),
        l.version,
    )
}

fn pick<'a>(records: &'a [Record], keys: &[(String, String, String, u32, u32)]) -> Vec<&'a Label> {
    let wanted: std::collections::BTreeSet<_> = keys.iter().collect();
    records
        .iter()
        .filter_map(|r| match r {
            Record::Label(l) if wanted.contains(&key(l)) => Some(l),
            _ => None,
        })
        .collect()
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let run = d
        .records
        .iter()
        .find_map(|r| match r {
            Record::Run(run) if run.run_id == args.run => Some(run),
            _ => None,
        })
        .ok_or_else(|| {
            CliError::usage(format!("no run named {:?}", args.run))
                .with_hint("list runs with `evalsig runs`")
        })?;
    let judgments: Vec<&Judgment> = d
        .records
        .iter()
        .filter_map(|r| match r {
            Record::Judgment(j) if j.run_id == args.run => Some(j),
            _ => None,
        })
        .collect();
    let items: BTreeMap<&str, &Item> = d
        .records
        .iter()
        .filter_map(|r| match r {
            Record::Item(i) => Some((i.item_id.as_str(), i)),
            _ => None,
        })
        .collect();
    let (labels, undated_skipped) = select_labels(&d.records, args.labels_as_of.as_deref())?;
    let settings = Settings {
        task: args.task.clone(),
        positive: config.positive().for_task(&args.task).copied(),
        decision: config.decision().for_task(&args.task).copied(),
        threshold: args.threshold,
        estimand: args.estimand.unwrap_or(config.stats.estimand),
        missing: config.missing_policy(),
        reference: args.reference.map_or(config.reference.mode, Into::into),
        level: config.stats.level,
        bootstrap: config.stats.bootstrap,
        seed: config.stats.seed.unwrap_or(0),
        prevalence: config.cost.as_ref().map(|c| c.prevalence),
        by: args.by.clone(),
        items: args.items,
    };
    let report = report::build(run, &judgments, &items, &labels, &settings)
        .map_err(|e| CliError::new("report", e.to_string(), Exit::Rejected))?;
    let findings = validity::dataset(
        &d.records,
        &checks::overrides(args.checks, ArmArgs::default()),
    );
    let results = round_numbers(serde_json::to_value(&report).expect("report serializes"));
    let output_hash = ContentHash::blake3(&to_canonical_bytes(&results).map_err(CliError::store)?);
    let cfg = config_hash(&config);
    let mut envelope = Envelope::new("evalsig.report/1", results);
    if undated_skipped > 0 {
        envelope.warnings.push(crate::output::Warning {
            code: "undated_label_revisions".into(),
            message: format!("{undated_skipped} labels have newer undated revisions that --labels-as-of cannot place in time"),
        });
    }
    envelope.provenance.snapshots = d.snapshots.iter().map(ToString::to_string).collect();
    envelope.provenance.config = Some(cfg.to_string());
    envelope.provenance.seed = Some(settings.seed);
    let mut summary = serde_json::Map::new();
    summary.insert("run".into(), json!(args.run));
    summary.insert("task".into(), json!(args.task));
    summary.insert("estimand".into(), json!(settings.estimand.to_string()));
    summary.insert("seed".into(), json!(settings.seed));
    let entry = ledger::append(
        &project.ledger_path(),
        LedgerSide::Agent,
        Kind::Report,
        NewEntry {
            ts: now(),
            actor: actor(),
            inputs: envelope
                .provenance
                .snapshots
                .iter()
                .cloned()
                .chain([cfg.to_string()])
                .collect(),
            outputs: vec![output_hash.to_string()],
            summary,
            tool: crate::tool_version(),
        },
    )
    .map_err(CliError::store)?;
    envelope.provenance.ledger = Some("agent".into());
    envelope.provenance.ledger_seq = Some(entry.seq);
    let path = project
        .dir
        .join("reports")
        .join(format!("{}.json", output_hash.hex()));
    let mut saved = serde_json::to_string_pretty(&envelope).expect("envelope serializes");
    saved.push('\n');
    std::fs::write(&path, saved).map_err(|e| io_error(&path, &e))?;
    Ok(checks::apply(envelope, &findings))
}
