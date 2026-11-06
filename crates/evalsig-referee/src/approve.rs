//! `evalsig preregister approve <draft>`: the referee resolves every setting the
//! draft leaves open, pins what the gate will rely on by hash and writes
//! `approved/<hash>.toml`, the only file `gate` reads (SPEC §8.3). It fixes the Ladder
//! step and first best (D-015).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use evalsig_core::bootstrap::required_resamples;
use evalsig_core::config::Config;
use evalsig_core::gate::{self, Sided, interval_level};
use evalsig_core::missing::Policy;
use evalsig_core::power;
use evalsig_core::records::{Judgment, Label, Record, Run};
use evalsig_core::report::{self, Fill, Sections};
use evalsig_core::rules::Rule;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{Entry, Kind};
use serde_json::{Value, json};

use crate::conditions::{self, Conditions, Ladder, Resolved};
use crate::home::Referee;
use evalsig_project::Error;
use evalsig_project::data::{self, Data};
use evalsig_project::envelope::{Envelope, Output, Schema, Warning, round_numbers};
use evalsig_project::project::{Project, hex, io_error};

/// What the referee approves: a draft, for a holdout, with a pinned mapping.
#[derive(Debug, Clone)]
pub struct Request {
    /// The draft's hash, as `preregister --draft` reported it.
    pub draft: String,
    /// The holdout the gate will use, by its hash.
    pub holdout: String,
    /// The project whose .evalsig/drafts/ holds the draft (default: --project).
    pub from: Option<PathBuf>,
    /// The mapping file or converter that makes the candidate's records; its hash is
    /// pinned.
    pub mapping: Option<PathBuf>,
    /// The referee's home (default: EVALSIG_REFEREE).
    pub referee: Option<PathBuf>,
}

fn canonical_hash(v: &Value) -> Result<String, Error> {
    Ok(evalsig_store::canonical::hash(v)
        .map_err(Error::store)?
        .to_string())
}

/// The draft from a project's drafts directory, checked against its hash.
fn draft(dir: &Path, hash: &str) -> Result<Conditions, Error> {
    let path = Project::open(dir)?
        .dir
        .join("drafts")
        .join(format!("{}.json", hex(hash)));
    let text = std::fs::read_to_string(&path)
        .map_err(|_| Error::usage(format!("no draft {hash} in {}", dir.display())))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| Error::invalid_input(&path, e))?;
    if canonical_hash(&doc)? != hash {
        return Err(Error::invalid_conditions(format!(
            "{} does not hash to {hash}",
            path.display()
        )));
    }
    conditions::checked(&doc, &path.display().to_string())
}

fn last<'e>(entries: &'e [Entry], kind: Kind, holdout: &str) -> Option<&'e Entry> {
    entries.iter().rev().find(|e| {
        e.kind == kind
            && (e.outputs.first().map(String::as_str) == Some(holdout)
                || e.summary.get("holdout").and_then(Value::as_str) == Some(holdout))
    })
}

pub fn approve(root: &Path, args: &Request) -> Result<Output, Error> {
    let (referee, _lock) = Referee::open_exclusive(args.referee.as_deref())?;
    let project = Project::open(root)?;
    let config = project.config()?;
    let mut doc = draft(args.from.as_deref().unwrap_or(root), &args.draft)?;
    let entries = referee.entries()?;
    let split = last(&entries, Kind::Split, &args.holdout)
        .ok_or_else(|| Error::usage(format!("this referee has cut no holdout {}", args.holdout)))?;
    let seal = last(&entries, Kind::Seal, &args.holdout)
        .ok_or_else(|| Error::usage("seal the holdout before approving conditions on it"))?;
    let held = referee.holdout_items(&args.holdout)?;
    if (held.len() as u64) < doc.min_items {
        return Err(Error::invalid_conditions(format!(
            "min_items is {} but the holdout has {} items; the gate could never decide",
            doc.min_items,
            held.len()
        )));
    }
    let d = data::load(&project)?;
    let mapping_hash = args
        .mapping
        .as_deref()
        .map(|p| -> Result<String, Error> {
            let bytes = std::fs::read(p).map_err(|e| io_error(p, &e))?;
            Ok(ContentHash::blake3(&bytes).to_string())
        })
        .transpose()?;
    let pins = Pins {
        split_entry: split.hash.clone(),
        seal_entry: seal.hash.clone(),
        holdout: args.holdout.clone(),
        holdout_items_hash: holdout_items_hash(&d, &held)?,
        mapping_hash,
        holdout_access_so_far: Referee::accesses(&entries, &args.holdout),
    };
    let approval = Approval {
        doc: &doc,
        config: &config,
        d: &d,
        held: &held,
    };
    let (resolved, power) = approval.resolve(pins)?;
    doc.resolved = Some(resolved);
    // The gate reads no configuration, so human_parity's epsilon is fixed here.
    for c in &mut doc.conditions {
        if c.metric == "human_parity" && c.value.is_none() {
            c.value = Some(config.agree.parity_epsilon);
        }
    }
    let approved = canonical_hash(&doc.to_json())?;
    let path = referee.file("approved", &approved, "toml");
    let toml = toml::to_string(&doc).map_err(|e| Error::invalid_conditions(e.to_string()))?;
    std::fs::write(&path, toml).map_err(|e| io_error(&path, &e))?;
    let resolved = doc.resolved.as_ref().expect("set above");

    let provenance = label_provenance(&d, &held);
    let summary = json!({
        "name": doc.name,
        "draft": args.draft,
        "approved": approved,
        "holdout": args.holdout,
        "budget": doc.budget(),
        "per_submission_level": format!("{:.6}", resolved.per_submission_level),
        "ladder_step": resolved.ladder.as_ref().map(|l| format!("{:.6}", l.step)),
        "power_at_hypothesis": power.map(|p| format!("{p:.4}")),
    });
    referee.record(
        Kind::PreregisterApprove,
        vec![args.draft.clone(), args.holdout.clone(), seal.hash.clone()],
        vec![approved.clone()],
        summary,
    )?;
    let mut envelope = Envelope::new(
        Schema::Preregister,
        round_numbers(json!({
            "approved": approved,
            "file": path.display().to_string(),
            "resolved": resolved,
            "power_at_hypothesis": power,
            "holdout_access_so_far": resolved.holdout_access_so_far,
            "label_provenance": provenance,
        })),
    );
    envelope.provenance.ledger = Some("referee".into());
    envelope.provenance.snapshots = d.snapshot_ids();
    if resolved.ladder.is_none() {
        envelope.warnings.push(Warning {
            code: "ladder_unavailable".into(),
            message: "no Ladder step could be fixed (no baseline, or the first condition's metric has no value on both sides of the holdout); the gate will return verdicts only".into(),
        });
    }
    if let Some(p) = power.filter(|p| *p < 0.5) {
        envelope.warnings.push(Warning {
            code: "low_power_at_hypothesis".into(),
            message: format!(
                "if the hypothesis holds, the gate passes with probability {p:.2} only"
            ),
        });
    }
    Ok(Output::success(envelope))
}

/// What approval pins of the referee's state, beside the resolved settings.
struct Pins {
    split_entry: String,
    seal_entry: String,
    holdout: String,
    holdout_items_hash: String,
    mapping_hash: Option<String>,
    holdout_access_so_far: u64,
}

/// A draft being approved, with the referee's configuration, its data and the
/// holdout's items.
struct Approval<'a> {
    doc: &'a Conditions,
    config: &'a Config,
    d: &'a Data,
    held: &'a BTreeSet<String>,
}

impl Approval<'_> {
    /// Every setting the gate needs, from the draft or else the referee's
    /// configuration, and the power at the hypothesis when there is a baseline.
    fn resolve(&self, pins: Pins) -> Result<(Resolved, Option<f64>), Error> {
        let (doc, config, d) = (self.doc, self.config, self.d);
        let task = doc.task();
        let run = doc
            .compare
            .baseline
            .as_deref()
            .map(|b| d.run(b))
            .transpose()?;
        let (all, _) = d.labels(None)?;
        let labels: Vec<&Label> = all.iter().filter(|l| l.task() == task).collect();
        // As the report would: boolean labels make `true` the positive value.
        let positive = report::positive_rule(
            doc.positive.or(config.positive().for_task(task).copied()),
            &labels,
        );
        if positive.is_none()
            && run.is_some_and(|r| {
                r.value_semantics() == evalsig_core::records::ValueSemantics::Decision
            })
        {
            return Err(Error::invalid_conditions(
                "the conditions need a positive definition (positive) for this decision run",
            ));
        }
        let decision = doc.decision.or(config.decision().for_task(task).copied());
        let budget = doc.budget();
        let level = doc.conditions[0].level.unwrap_or(config.stats.level);
        let per_submission = interval_level(level, budget, Sided::Two);
        // The bootstrap must reach the tail of the strictest condition, at its own level
        // and sidedness, as the gate will judge it.
        let alpha = doc
            .conditions
            .iter()
            .map(|c| 1.0 - interval_level(c.level.unwrap_or(level), budget, c.sided))
            .fold(1.0 - per_submission, f64::min);
        let reference = doc.reference.as_ref();
        let mut resolved = Resolved {
            task: task.to_owned(),
            positive,
            decision,
            estimand: doc.estimand.unwrap_or(config.stats.estimand),
            missing_on_holdout: Policy::Worst,
            missing: config.missing_policy().table(),
            level,
            per_submission_level: per_submission,
            bootstrap: config.stats.bootstrap.max(required_resamples(alpha)),
            reference_mode: reference
                .and_then(|r| r.mode)
                .unwrap_or(config.reference.mode),
            rater_design: reference
                .and_then(|r| r.rater_design)
                .unwrap_or(config.reference.rater_design),
            seed_derivation: "blake3(approved || candidate snapshot)".into(),
            ladder: None,
            output_type: None,
            levels: None,
            value_semantics: None,
            baseline_snapshot: None,
            split_entry: pins.split_entry,
            seal_entry: pins.seal_entry,
            holdout: pins.holdout,
            holdout_items_hash: pins.holdout_items_hash,
            mapping_hash: pins.mapping_hash,
            holdout_access_so_far: pins.holdout_access_so_far,
        };
        let mut power = None;
        if let Some(run) = run {
            resolved.ladder = self.ladder(run, positive, decision)?;
            resolved.output_type = run.output_type;
            resolved.levels = run.levels;
            resolved.value_semantics = Some(run.value_semantics());
            resolved.baseline_snapshot = Some(baseline_hash(d, &run.run_id)?);
            power = self.power_at_hypothesis(run, positive, decision, per_submission)?;
        }
        Ok((resolved, power))
    }

    /// The probability of passing the primary difference condition if the candidate is
    /// better by its `min_effect`, on the holdout's items, at the per-submission level
    /// (metrics spec §11.1, §12). ψ̂ = max(0.05, 4/n, min(1, 2·e_val)) with e_val the
    /// baseline's error on the items outside the holdout (§11.3).
    fn power_at_hypothesis(
        &self,
        run: &Run,
        positive: Option<Rule>,
        decision: Option<Rule>,
        per_submission: f64,
    ) -> Result<Option<f64>, Error> {
        let (d, held) = (self.d, self.held);
        let c = &self.doc.conditions[0];
        let on_delta = matches!(
            c.rule,
            gate::Rule::DeltaLowerBoundAtLeast | gate::Rule::DeltaUpperBoundAtMost
        );
        let Some(delta) = c.min_effect.filter(|m| *m > 0.0 && on_delta) else {
            return Ok(None);
        };
        let outside: Vec<&Judgment> = d
            .judgments(&run.run_id)
            .into_iter()
            .filter(|j| !held.contains(&j.item_id))
            .collect();
        let (all, _) = d.labels(None)?;
        let labels: Vec<&Label> = all.iter().collect();
        let settings = self.baseline_settings(positive, decision);
        let r =
            report::build(run, &outside, &d.items(), &labels, &settings).map_err(Error::report)?;
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

    /// The settings a baseline is read with at approval, as the gate will read it.
    fn baseline_settings(
        &self,
        positive: Option<Rule>,
        decision: Option<Rule>,
    ) -> report::Settings {
        let config = self.config;
        report::Settings {
            task: self.doc.task().to_owned(),
            positive,
            decision,
            estimand: self.doc.estimand.unwrap_or(config.stats.estimand),
            missing: config.missing_policy(),
            reference: config.reference.mode,
            rater_design: config.reference.rater_design,
            level: config.stats.level,
            bootstrap: config.stats.bootstrap,
            seed: config.stats.seed.unwrap_or(0),
            threshold: None,
            prevalence: None,
            sections: Sections::default(),
            exact_rates: false,
            proxy: None,
        }
    }

    /// The Ladder's step and first best (metrics spec §11.3, D-015), on the primary
    /// condition's metric: η = max(k·√(ψ̂/n), 1/n), n the metric's denominator on the
    /// holdout and ψ̂ = max(0.05, 4/n, min(1, 2·e_val)), e_val the baseline's error outside
    /// the holdout; the first best is the baseline on the holdout. None when there is no
    /// baseline or the metric is not one a report gives.
    fn ladder(
        &self,
        run: &Run,
        positive: Option<Rule>,
        decision: Option<Rule>,
    ) -> Result<Option<Ladder>, Error> {
        let (d, held) = (self.d, self.held);
        let metric = self.doc.conditions[0].metric.as_str();
        let (all, _) = d.labels(None)?;
        let labels: Vec<&Label> = all.iter().collect();
        // On the holdout every missing value is the worst, as the gate will count it.
        let settings = report::Settings {
            missing: evalsig_core::missing::MissingPolicy::holdout(),
            ..self.baseline_settings(positive, decision)
        };
        let items = d.items();
        // The metric's value, its denominator and its direction, on or off the holdout.
        let on = |inside: bool| -> Result<Option<(f64, f64, bool)>, Error> {
            let judgments: Vec<&Judgment> = d
                .judgments(&run.run_id)
                .into_iter()
                .filter(|j| held.contains(&j.item_id) == inside)
                .collect();
            let p = report::prepare(run, &judgments, &items, &labels, &settings)
                .map_err(Error::report)?;
            let Some(id) = p.metric_ids().into_iter().find(|id| id.name() == metric) else {
                return Ok(None);
            };
            let values = p.item_values(id, Fill::Policy);
            let n: f64 = values.values().map(|v| v.denominator).sum();
            Ok(evalsig_core::compare::total_ratio(&values).map(|v| (v, n, id.higher_is_better())))
        };
        let (Some((outside, _, higher)), Some((first_best, n, _))) = (on(false)?, on(true)?) else {
            return Ok(None);
        };
        let e_val = if higher { 1.0 - outside } else { outside };
        let psi = gate::ladder_psi(e_val, n);
        let draft = self.doc.holdout.as_ref();
        let k = draft.and_then(|h| h.ladder_k).unwrap_or(2.0);
        let automatic = gate::ladder_step(k, psi, n);
        let step = match draft.and_then(|h| h.ladder_step) {
            // A step printed at six decimals and copied back is the automatic one.
            Some(s) if s < automatic - 1e-6 => {
                return Err(Error::usage(format!(
                    "ladder_step {s} is below the automatic step {automatic:.6}; a finer step would return more than the holdout can tell"
                )));
            }
            Some(s) => s,
            None => automatic,
        };
        Ok(Some(Ladder {
            reuse_guard: "ladder".into(),
            metric: metric.to_owned(),
            k,
            step,
            denominator: n,
            psi_hat: psi,
            first_best,
        }))
    }
}

/// The baseline's records, canonical and sorted, by hash.
pub fn baseline_hash(d: &Data, run: &str) -> Result<String, Error> {
    evalsig_project::data::records_hash(run_records(&d.records, run))
}

/// The records of one run: the run and its judgments.
pub fn run_records<'a>(
    records: &'a [Record],
    run: &'a str,
) -> impl Iterator<Item = &'a Record> + 'a {
    records.iter().filter(move |r| match r {
        Record::Run(x) => x.run_id == run,
        Record::Judgment(j) => j.run_id == run,
        _ => false,
    })
}

/// The holdout items' IDs and the attributes a candidate could rewrite (SPEC §8.3).
pub fn holdout_items_hash(d: &Data, held: &BTreeSet<String>) -> Result<String, Error> {
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

/// `preregister show <approved>`: the approved conditions as gate reads them, checked
/// against their hash.
pub fn load(referee: &Referee, approved: &str) -> Result<Approved, Error> {
    // The hash names a file, and gate serve takes it from the network: only a well-formed
    // BLAKE3 reference may reach the path.
    if !approved
        .parse::<ContentHash>()
        .is_ok_and(|h| h.to_string().starts_with("blake3:"))
    {
        return Err(Error::usage(format!(
            "{approved:?} is not an approved hash (blake3:<64 hex>)"
        )));
    }
    let path = referee.file("approved", approved, "toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|_| Error::usage(format!("no approved conditions {approved}")))?;
    let mut doc: Conditions = toml::from_str(&text).map_err(|e| Error::invalid_input(&path, e))?;
    if canonical_hash(&doc.to_json())? != approved {
        return Err(Error::invalid_conditions(format!(
            "{} was changed after approval",
            path.display()
        )));
    }
    let resolved = doc.resolved.take().ok_or_else(|| {
        Error::invalid_conditions(format!(
            "{} has no resolved settings; approve the conditions again",
            path.display()
        ))
    })?;
    Ok(Approved {
        conditions: doc,
        resolved,
    })
}

/// Approved conditions, and apart from them the settings approval resolved.
#[derive(Debug, Clone)]
pub struct Approved {
    pub conditions: Conditions,
    pub resolved: Resolved,
}
