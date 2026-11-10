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
    // The budget belongs to the sealed holdout: its first approval fixes B and later ones
    // keep it, so every submission is judged at the same alpha/B.
    let fixed = entries
        .iter()
        .find(|e| {
            e.kind == Kind::PreregisterApprove
                && e.summary.get("holdout").and_then(Value::as_str) == Some(args.holdout.as_str())
        })
        .and_then(|e| e.summary.get("budget").and_then(Value::as_u64));
    if let Some(b) = fixed.filter(|b| *b != u64::from(doc.budget())) {
        return Err(Error::invalid_conditions(format!(
            "this holdout's budget was fixed at {b} by its first approval; a new budget needs a new split and seal"
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
        "power": power.iter().map(|p| json!({
            "condition": p.condition,
            "pass_probability": format!("{:.4}", p.pass_probability),
        })).collect::<Vec<_>>(),
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
            "power": power.iter().map(|p| json!({
                "condition": p.condition,
                "metric": p.metric,
                "expected_effect": p.expected_effect,
                "pass_probability": p.pass_probability,
            })).collect::<Vec<_>>(),
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
    for p in power.iter().filter(|p| p.pass_probability < 0.8) {
        envelope.warnings.push(Warning {
            code: "low_power".into(),
            message: format!(
                "condition {} ({}) passes with probability {:.2} only if the candidate is better by the expected {}",
                p.condition, p.metric, p.pass_probability, p.expected_effect
            ),
        });
    }
    Ok(Output::success(envelope))
}

/// The probability that a difference condition passes at its expected effect (metrics
/// spec §11.1).
struct Power {
    condition: usize,
    metric: String,
    expected_effect: f64,
    pass_probability: f64,
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
    fn resolve(&self, pins: Pins) -> Result<(Resolved, Vec<Power>), Error> {
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
        // The level of a condition that names none; the primary condition's own level
        // fixes only the per-submission level.
        let level = config.stats.level;
        let per_submission =
            interval_level(doc.conditions[0].level.unwrap_or(level), budget, Sided::Two);
        // The bootstrap must reach the percentile tail of the strictest condition, at its
        // own level and sidedness, as the gate will judge it: the quantile its interval
        // reads is half of 1 - interval_level.
        let alpha = doc
            .conditions
            .iter()
            .map(|c| (1.0 - interval_level(c.level.unwrap_or(level), budget, c.sided)) / 2.0)
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
        let mut power = Vec::new();
        if let Some(run) = run {
            resolved.ladder = self.ladder(run, positive, decision)?;
            resolved.output_type = run.output_type;
            resolved.levels = run.levels;
            resolved.value_semantics = Some(run.value_semantics());
            resolved.baseline_snapshot = Some(baseline_hash(d, &run.run_id)?);
            power = self.power(run, positive, decision, level, budget)?;
        }
        Ok((resolved, power))
    }

    /// For each difference condition that states an `expected_effect`, the probability
    /// that it passes if the candidate is better by that much (metrics spec §11.1): on the
    /// metric's denominator in the holdout, at the condition's own level and sidedness
    /// and the budget, with the discordance ψ̂ the Ladder uses.
    fn power(
        &self,
        run: &Run,
        positive: Option<Rule>,
        decision: Option<Rule>,
        level: f64,
        budget: u32,
    ) -> Result<Vec<Power>, Error> {
        let mut out = Vec::new();
        for (k, c) in self.doc.conditions.iter().enumerate() {
            let Some(effect) = c.expected_effect.filter(|_| c.rule.on_delta()) else {
                continue;
            };
            let (Some((outside, _, higher)), Some((_, n, _))) = (
                self.metric_on(run, &c.metric, false, positive, decision)?,
                self.metric_on(run, &c.metric, true, positive, decision)?,
            ) else {
                continue;
            };
            let e_val = if higher { 1.0 - outside } else { outside };
            let value = c.value.unwrap_or(0.0);
            let (threshold, min_effect) = match c.rule {
                gate::Rule::NotWorseThan => (-value, f64::NEG_INFINITY),
                _ => (value, c.min_effect.unwrap_or(0.0)),
            };
            let tail = (1.0 - interval_level(c.level.unwrap_or(level), budget, c.sided)) / 2.0;
            if let Some(p) = power::difference_pass_probability(
                n,
                gate::ladder_psi(e_val, n),
                effect,
                threshold,
                min_effect,
                tail,
            ) {
                out.push(Power {
                    condition: k,
                    metric: c.metric.clone(),
                    expected_effect: effect,
                    pass_probability: p,
                });
            }
        }
        Ok(out)
    }

    /// The baseline's value of `metric`, its denominator and whether higher is better, on
    /// the holdout (`inside`) or off it, with every missing value the worst as the gate
    /// counts it.
    fn metric_on(
        &self,
        run: &Run,
        metric: &str,
        inside: bool,
        positive: Option<Rule>,
        decision: Option<Rule>,
    ) -> Result<Option<(f64, f64, bool)>, Error> {
        let (d, held) = (self.d, self.held);
        let (all, _) = d.labels(None)?;
        let labels: Vec<&Label> = all.iter().collect();
        let settings = report::Settings {
            missing: evalsig_core::missing::MissingPolicy::holdout(),
            ..self.baseline_settings(positive, decision)
        };
        let judgments: Vec<&Judgment> = d
            .judgments(&run.run_id)
            .into_iter()
            .filter(|j| held.contains(&j.item_id) == inside)
            .collect();
        let p = report::prepare(run, &judgments, &d.items(), &labels, &settings)
            .map_err(Error::report)?;
        let Some(id) = p.metric_ids().into_iter().find(|id| id.name() == metric) else {
            return Ok(None);
        };
        let values = p.item_values(id, Fill::Policy);
        let n = evalsig_core::numeric::sum(values.values().map(|v| v.denominator));
        Ok(evalsig_core::compare::total_ratio(&values).map(|v| (v, n, id.higher_is_better())))
    }

    /// The settings a baseline is read with at approval, as the gate will read it.
    fn baseline_settings(
        &self,
        positive: Option<Rule>,
        decision: Option<Rule>,
    ) -> report::Settings {
        let config = self.config;
        let reference = self.doc.reference.as_ref();
        report::Settings {
            task: self.doc.task().to_owned(),
            positive,
            decision,
            estimand: self.doc.estimand.unwrap_or(config.stats.estimand),
            missing: config.missing_policy(),
            reference: reference
                .and_then(|r| r.mode)
                .unwrap_or(config.reference.mode),
            rater_design: reference
                .and_then(|r| r.rater_design)
                .unwrap_or(config.reference.rater_design),
            level: config.stats.level,
            bootstrap: config.stats.bootstrap,
            seed: config.stats.seed.unwrap_or(0),
            threshold: None,
            prevalence: None,
            cost: None,
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
        let metric = self.doc.conditions[0].metric.as_str();
        let (Some((outside, _, higher)), Some((first_best, n, _))) = (
            self.metric_on(run, metric, false, positive, decision)?,
            self.metric_on(run, metric, true, positive, decision)?,
        ) else {
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
