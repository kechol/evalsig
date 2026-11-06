//! `evalsig preregister approve <draft>`: the referee resolves every setting the
//! draft leaves open, pins what the gate will rely on by hash and writes
//! `approved/<hash>.toml`, the only file `gate` reads (SPEC §8.3). It fixes no Ladder
//! step or first best (D-015 is Proposed).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use evalsig_core::bootstrap::required_resamples;
use evalsig_core::config::Config;
use evalsig_core::gate::{Sided, interval_level};
use evalsig_core::power;
use evalsig_core::records::{Judgment, Label, Record};
use evalsig_core::report;
use evalsig_core::rules::Rule;
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{Entry, Kind};
use serde_json::{Value, json};

use crate::data::{self, Data};
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Warning, round_numbers};
use crate::project::{Project, hex, io_error};
use crate::referee::Referee;

#[derive(Debug, Clone, clap::Args)]
pub struct ApproveArgs {
    /// The draft's hash, as `preregister --draft` reported it.
    pub draft: String,
    /// The holdout the gate will use, by its hash.
    #[arg(long, value_name = "HASH")]
    pub holdout: String,
    /// The project whose .evalsig/drafts/ holds the draft (default: --project).
    #[arg(long, value_name = "DIR")]
    pub from: Option<PathBuf>,
    /// The mapping file or converter that makes the candidate's records; its hash is
    /// pinned.
    #[arg(long, value_name = "FILE")]
    pub mapping: Option<PathBuf>,
    /// The referee's home (default: EVALSIG_REFEREE).
    #[arg(long, value_name = "DIR")]
    pub referee: Option<PathBuf>,
}

fn rejected(message: impl Into<String>) -> CliError {
    CliError::new("invalid_conditions", message, Exit::Rejected)
}

pub fn canonical_hash(v: &Value) -> Result<String, CliError> {
    Ok(ContentHash::blake3(&to_canonical_bytes(v).map_err(CliError::store)?).to_string())
}

/// The draft from a project's drafts directory, checked against its hash.
fn draft(dir: &Path, hash: &str) -> Result<Value, CliError> {
    let path = Project::open(dir)?
        .dir
        .join("drafts")
        .join(format!("{}.json", hex(hash)));
    let text = std::fs::read_to_string(&path)
        .map_err(|_| CliError::usage(format!("no draft {hash} in {}", dir.display())))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| CliError::invalid_input(&path, e))?;
    if canonical_hash(&doc)? != hash {
        return Err(rejected(format!(
            "{} does not hash to {hash}",
            path.display()
        )));
    }
    crate::commands::preregister::check(&doc, &path.display().to_string())?;
    Ok(doc)
}

fn last<'e>(entries: &'e [Entry], kind: Kind, holdout: &str) -> Option<&'e Entry> {
    entries.iter().rev().find(|e| {
        e.kind == kind
            && (e.outputs.first().map(String::as_str) == Some(holdout)
                || e.summary.get("holdout").and_then(Value::as_str) == Some(holdout))
    })
}

/// A rule as the draft writes it, or the configuration's.
fn rule(v: &Value, fallback: Option<Rule>) -> Result<Option<Rule>, CliError> {
    match v {
        Value::Null => Ok(fallback),
        other => serde_json::from_value(other.clone())
            .map(Some)
            .map_err(|e| rejected(format!("rule {other}: {e}"))),
    }
}

pub fn approve(root: &Path, args: &ApproveArgs) -> Result<Output, CliError> {
    let (referee, _lock) = Referee::open_exclusive(args.referee.as_deref())?;
    let project = Project::open(root)?;
    let config = project.config()?;
    let mut doc = draft(args.from.as_deref().unwrap_or(root), &args.draft)?;
    let entries = referee.entries()?;
    let split = last(&entries, Kind::Split, &args.holdout).ok_or_else(|| {
        CliError::usage(format!("this referee has cut no holdout {}", args.holdout))
    })?;
    let seal = last(&entries, Kind::Seal, &args.holdout)
        .ok_or_else(|| CliError::usage("seal the holdout before approving conditions on it"))?;
    let held = referee.holdout_items(&args.holdout)?;
    if let Some(m) = doc["min_items"]
        .as_u64()
        .filter(|m| (held.len() as u64) < *m)
    {
        return Err(rejected(format!(
            "min_items is {m} but the holdout has {} items; the gate could never decide",
            held.len()
        )));
    }
    let d = data::load(&project)?;
    let task = doc["task"]
        .as_str()
        .unwrap_or(evalsig_core::records::DEFAULT_TASK)
        .to_owned();
    let baseline = doc["compare"]["baseline"].as_str().map(str::to_owned);
    let budget = budget(&doc) as u32;
    let resolved = resolve(&doc, &config, &d, &task, baseline.as_deref(), &held, budget)?;
    let mapping_hash = match &args.mapping {
        Some(p) => {
            Some(ContentHash::blake3(&std::fs::read(p).map_err(|e| io_error(p, &e))?).to_string())
        }
        None => None,
    };
    let mut r = resolved.settings;
    r["split_entry"] = json!(split.hash);
    r["seal_entry"] = json!(seal.hash);
    r["holdout"] = json!(args.holdout);
    r["holdout_items_hash"] = json!(holdout_items_hash(&d, &held)?);
    r["mapping_hash"] = json!(mapping_hash);
    r["holdout_access_so_far"] = json!(
        entries
            .iter()
            .filter(|e| e.kind == Kind::HoldoutAccess
                && e.summary.get("holdout").and_then(Value::as_str) == Some(args.holdout.as_str()))
            .count()
    );
    doc["resolved"] = r;
    // The gate reads no configuration, so human_parity's epsilon is fixed here.
    for c in doc["conditions"].as_array_mut().into_iter().flatten() {
        if c["metric"] == "human_parity" && c["value"].is_null() {
            c["value"] = json!(config.agree.parity_epsilon);
        }
    }
    // TOML has no null: unset values are left out, before hashing, so the approved
    // file reads back to the same document.
    strip_nulls(&mut doc);
    let approved = canonical_hash(&doc)?;
    let path = referee.file("approved", &approved, "toml");
    let toml = toml::to_string(&doc).map_err(|e| rejected(e.to_string()))?;
    std::fs::write(&path, toml).map_err(|e| io_error(&path, &e))?;

    let provenance = label_provenance(&d, &held);
    let summary = json!({
        "name": doc["name"],
        "draft": args.draft,
        "approved": approved,
        "holdout": args.holdout,
        "budget": budget,
        "per_submission_level": doc["resolved"]["per_submission_level"].as_f64().map(|l| format!("{l:.6}")),
        "power_at_hypothesis": resolved.power.map(|p| format!("{p:.4}")),
    });
    referee.record(
        Kind::PreregisterApprove,
        vec![args.draft.clone(), args.holdout.clone(), seal.hash.clone()],
        vec![approved.clone()],
        summary,
    )?;
    let mut envelope = Envelope::new(
        "evalsig.preregister/1",
        round_numbers(json!({
            "approved": approved,
            "file": path.display().to_string(),
            "resolved": doc["resolved"],
            "power_at_hypothesis": resolved.power,
            "holdout_access_so_far": doc["resolved"]["holdout_access_so_far"],
            "label_provenance": provenance,
        })),
    );
    envelope.provenance.ledger = Some("referee".into());
    envelope.provenance.snapshots = d.snapshot_ids();
    if let Some(p) = resolved.power.filter(|p| *p < 0.5) {
        envelope.warnings.push(Warning {
            code: "low_power_at_hypothesis".into(),
            message: format!(
                "if the hypothesis holds, the gate passes with probability {p:.2} only"
            ),
        });
    }
    Ok(Output::success(envelope))
}

struct Resolved {
    settings: Value,
    power: Option<f64>,
}

/// Every setting the gate needs, from the draft or else the referee's configuration.
#[allow(clippy::too_many_arguments)]
fn resolve(
    doc: &Value,
    config: &Config,
    d: &Data,
    task: &str,
    baseline: Option<&str>,
    held: &BTreeSet<String>,
    budget: u32,
) -> Result<Resolved, CliError> {
    let run = baseline.map(|b| d.run(b)).transpose()?;
    let (all, _) = d.labels(None)?;
    let labels: Vec<&Label> = all.iter().filter(|l| l.task() == task).collect();
    // As the report would: boolean labels make `true` the positive value.
    let positive = report::positive_rule(
        rule(&doc["positive"], config.positive().for_task(task).copied())?,
        &labels,
    );
    if positive.is_none()
        && run
            .is_some_and(|r| r.value_semantics() == evalsig_core::records::ValueSemantics::Decision)
    {
        return Err(rejected(
            "the conditions need a positive definition (positive) for this decision run",
        ));
    }
    let decision = rule(&doc["decision"], config.decision().for_task(task).copied())?;
    let level = doc["conditions"][0]["level"]
        .as_f64()
        .unwrap_or(config.stats.level);
    let per_submission = interval_level(level, budget, Sided::Two);
    // The bootstrap must reach the tail of the strictest condition, at its own level
    // and sidedness, as the gate will judge it.
    let alpha = doc["conditions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            let sided = if c["sided"] == "one" {
                Sided::One
            } else {
                Sided::Two
            };
            1.0 - interval_level(c["level"].as_f64().unwrap_or(level), budget, sided)
        })
        .fold(1.0 - per_submission, f64::min);
    let mode = doc["reference"]["mode"]
        .as_str()
        .map_or_else(|| json!(config.reference.mode), |m| json!(m));
    let design = doc["reference"]["rater_design"]
        .as_str()
        .map_or_else(|| json!(config.reference.rater_design), |m| json!(m));
    let mut settings = json!({
        "task": task,
        "positive": positive,
        "decision": decision,
        "estimand": doc["estimand"].as_str().map_or_else(|| config.stats.estimand.to_string(), str::to_owned),
        "missing_on_holdout": "worst",
        "missing": config.missing_policy().table(),
        "level": level,
        "per_submission_level": per_submission,
        "bootstrap": config.stats.bootstrap.max(required_resamples(alpha)),
        "reference_mode": mode,
        "rater_design": design,
        "seed_derivation": "blake3(approved || candidate snapshot)",
    });
    let mut power = None;
    if let Some(run) = run {
        settings["output_type"] = json!(run.output_type);
        settings["levels"] = json!(run.levels);
        settings["value_semantics"] = json!(run.value_semantics());
        settings["baseline_snapshot"] = json!(baseline_hash(d, &run.run_id)?);
        power = power_at_hypothesis(
            doc,
            config,
            d,
            run,
            held,
            positive,
            decision,
            per_submission,
        )?;
    }
    Ok(Resolved { settings, power })
}

/// The baseline's records, canonical and sorted, by hash.
pub fn baseline_hash(d: &Data, run: &str) -> Result<String, CliError> {
    crate::data::records_hash(d.records.iter().filter(|r| match r {
        Record::Run(x) => x.run_id == run,
        Record::Judgment(j) => j.run_id == run,
        _ => false,
    }))
}

/// The holdout items' IDs and the attributes a candidate could rewrite (SPEC §8.3).
pub fn holdout_items_hash(d: &Data, held: &BTreeSet<String>) -> Result<String, CliError> {
    let items: Vec<Value> = d
        .items()
        .into_iter()
        .filter(|(id, _)| held.contains(*id))
        .map(|(_, i)| {
            json!({
                "item_id": i.item_id,
                "weight": i.weight,
                "cluster": i.cluster,
                "strata": i.strata,
                "label_inclusion_prob": i.label_inclusion_prob,
                "input_ref": i.input_ref,
            })
        })
        .collect();
    canonical_hash(&json!(items))
}

/// Holdout labels by source, so the approver sees whether people or models made them.
fn label_provenance(d: &Data, held: &BTreeSet<String>) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for r in &d.records {
        if let Record::Label(l) = r
            && held.contains(&l.item_id)
        {
            *out.entry(json!(l.source).as_str().unwrap_or("?").to_owned())
                .or_insert(0) += 1;
        }
    }
    out
}

/// The probability of passing the primary difference condition if the candidate is
/// better by its `min_effect`, on the holdout's items, at the per-submission level
/// (metrics spec §11.1, §12). ψ̂ = max(0.05, 4/n, min(1, 2·e_val)) with e_val the
/// baseline's error on the items outside the holdout (§11.3).
#[allow(clippy::too_many_arguments)]
fn power_at_hypothesis(
    doc: &Value,
    config: &Config,
    d: &Data,
    run: &evalsig_core::records::Run,
    held: &BTreeSet<String>,
    positive: Option<Rule>,
    decision: Option<Rule>,
    per_submission: f64,
) -> Result<Option<f64>, CliError> {
    let c = &doc["conditions"][0];
    let delta_rule = matches!(
        c["rule"].as_str(),
        Some("delta_lower_bound_at_least" | "delta_upper_bound_at_most")
    );
    let Some(delta) = c["min_effect"].as_f64().filter(|m| *m > 0.0 && delta_rule) else {
        return Ok(None);
    };
    let outside: Vec<&Judgment> = d
        .judgments(&run.run_id)
        .into_iter()
        .filter(|j| !held.contains(&j.item_id))
        .collect();
    let (all, _) = d.labels(None)?;
    let labels: Vec<&Label> = all.iter().collect();
    let settings = report::Settings {
        task: doc["task"]
            .as_str()
            .unwrap_or(evalsig_core::records::DEFAULT_TASK)
            .to_owned(),
        positive,
        decision,
        estimand: config.stats.estimand,
        missing: config.missing_policy(),
        reference: config.reference.mode,
        rater_design: config.reference.rater_design,
        level: config.stats.level,
        bootstrap: config.stats.bootstrap,
        seed: config.stats.seed.unwrap_or(0),
        threshold: None,
        prevalence: None,
        by: None,
        items: false,
        detail: false,
        features: false,
        exact_rates: false,
        proxy: None,
    };
    let r =
        report::build(run, &outside, &d.items(), &labels, &settings).map_err(CliError::report)?;
    let Some(accuracy) = r
        .metrics
        .iter()
        .find(|m| m.metric == "accuracy")
        .and_then(|m| m.value)
    else {
        return Ok(None);
    };
    let n = held.len() as f64;
    let psi = (2.0 * (1.0 - accuracy)).min(1.0).max(4.0 / n).max(0.05);
    if delta >= psi {
        return Ok(None);
    }
    Ok(Some(power::mcnemar_power(
        held.len(),
        (psi + delta) / 2.0,
        (psi - delta) / 2.0,
        1.0 - per_submission,
        false,
    )))
}

fn strip_nulls(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.retain(|_, x| !x.is_null());
            m.values_mut().for_each(strip_nulls);
        }
        Value::Array(a) => a.iter_mut().for_each(strip_nulls),
        _ => {}
    }
}

/// The holdout budget of approved or draft conditions (SPEC §8.4: 5 by default).
pub fn budget(doc: &Value) -> u64 {
    doc["holdout"]["budget"].as_u64().unwrap_or(5)
}

/// `preregister show <approved>`: the approved conditions as gate reads them, checked
/// against their hash.
pub fn load(referee: &Referee, approved: &str) -> Result<Value, CliError> {
    // The hash names a file, and gate serve takes it from the network: only a well-formed
    // BLAKE3 reference may reach the path.
    if !approved
        .parse::<ContentHash>()
        .is_ok_and(|h| h.to_string().starts_with("blake3:"))
    {
        return Err(CliError::usage(format!(
            "{approved:?} is not an approved hash (blake3:<64 hex>)"
        )));
    }
    let path = referee.file("approved", approved, "toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|_| CliError::usage(format!("no approved conditions {approved}")))?;
    let table: toml::Table =
        toml::from_str(&text).map_err(|e| CliError::invalid_input(&path, e))?;
    let doc = serde_json::to_value(table).expect("TOML converts to JSON");
    if canonical_hash(&doc)? != approved {
        return Err(rejected(format!(
            "{} was changed after approval",
            path.display()
        )));
    }
    Ok(doc)
}
