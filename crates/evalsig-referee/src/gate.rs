//! `evalsig gate`: judge a candidate on the sealed holdout under approved conditions
//! (SPEC §8.3, §8.4; metrics spec §11.1, §11.2). Everything from the candidate's side
//! is a claim the referee checks; only the approved copy, the referee ledger and the
//! sealed labels are trusted. The answer carries verdicts only, and a Ladder value on
//! a new best (D-015).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use evalsig_core::checks::Overrides;
use evalsig_core::compare::{self, Baseline, Options};
use evalsig_core::config::Estimand;
use evalsig_core::gate::{self, Evidence, Outcome, Rule as GateRule, Sided};
use evalsig_core::labels::LabelSet;
use evalsig_core::missing::MissingPolicy;
use evalsig_core::parity::{self, ParityVerdict};
use evalsig_core::records::{Item, Judgment, Label, Record, Run, Source};
use evalsig_core::report::{self, Fill, Sections, Settings};
use evalsig_core::validity;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use evalsig_store::seal;
use serde_json::{Value, json};

use crate::approve::{self, Approved};
use crate::conditions::Resolved;
use crate::home::Referee;
use evalsig_project::Error;
use evalsig_project::data;
use evalsig_project::envelope::{Envelope, Output, Schema, Verdict, VerdictCode, Warning};
use evalsig_project::exit::Exit;
use evalsig_project::project::{Project, hex, io_error};
use evalsig_project::validate::RecordValidator;

/// One submission as the referee judges it, and as `gate serve` receives it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Request {
    pub approved: String,
    pub candidate: String,
    /// The candidate's records, as records/1 JSONL text.
    pub records: String,
    pub mapping_hash: Option<String>,
    pub agent_ledger_head: Option<String>,
}

/// The answer the candidate's side receives.
struct Answer {
    code: VerdictCode,
    text: String,
    conditions: Vec<Value>,
    /// The Ladder's new best, if the candidate set one.
    ladder: Option<NewBest>,
}

/// A new best for the Ladder: what the answer shows, and the candidate's run and
/// judgments (JSON lines) that the referee keeps as the best.
struct NewBest {
    shown: Value,
    records: String,
}

/// Judge one submission: the checks, the budget, the conditions, the ledger lines and
/// the result file.
pub fn judge_request(root: &Path, referee: &Referee, args: &Request) -> Result<Output, Error> {
    // One submission at a time per referee home, across processes: the budget is
    // read, then written.
    let _lock = referee.lock()?;
    let approved = approve::load(referee, &args.approved)?;
    let holdout = approved.resolved.holdout.clone();
    let budget = u64::from(approved.conditions.budget());

    // The candidate's records: checked like any import, named by their canonical hash.
    let validator = RecordValidator::new();
    let mut records = Vec::new();
    for (n, line) in args
        .records
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        records.push(validator.check_line(n + 1, line).map_err(|issues| {
            Error::rejected(
                "invalid_records",
                format!("line {}: {}", n + 1, issues[0].message),
            )
        })?);
    }
    let candidate_hash = data::records_hash(records.iter())?;

    // The same candidate again: the earlier answer, without spending budget. The
    // records file may hold several runs, so the run is part of the name.
    let submission =
        ContentHash::blake3(format!("{candidate_hash}\n{}", args.candidate).as_bytes());
    let result_path =
        referee
            .path("results")
            .join(format!("{}-{}.json", hex(&args.approved), submission.hex()));
    if let Ok(saved) = std::fs::read_to_string(&result_path) {
        let saved: Value =
            serde_json::from_str(&saved).map_err(|e| Error::invalid_input(&result_path, e))?;
        // A refusal after the submission was counted is an earlier answer too.
        if let Some(refused) = saved.get("error") {
            let mut e = Error::rejected(
                refused["code"].as_str().unwrap_or("refused").to_owned(),
                refused["message"].as_str().unwrap_or_default(),
            );
            e.hint = refused["hint"].as_str().map(str::to_owned);
            return Err(e);
        }
        let mut envelope: Envelope =
            serde_json::from_value(saved).map_err(|e| Error::invalid_input(&result_path, e))?;
        let exit = exit_for(envelope.verdict.as_ref().map(|v| v.code));
        envelope.warnings.push(Warning {
            code: "resubmission".into(),
            message: "this candidate was judged before; the earlier answer is returned and no budget is spent".into(),
        });
        return Ok(Output { envelope, exit });
    }

    let run = records
        .iter()
        .find_map(|x| match x {
            Record::Run(r) if r.run_id == args.candidate => Some(r.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            Error::rejected(
                "invalid_records",
                format!("the records hold no run {}", args.candidate),
            )
        })?;
    let judgments: Vec<&Judgment> = records
        .iter()
        .filter_map(|x| match x {
            Record::Judgment(j) if j.run_id == args.candidate => Some(j),
            _ => None,
        })
        .collect();
    let entries = referee.entries()?;
    let held = referee.holdout_items_in(&holdout, &entries)?;

    // Coverage first, before any label is touched: no budget is spent on a shortfall.
    let judged: BTreeSet<&str> = judgments.iter().map(|j| j.item_id.as_str()).collect();
    let absent = held.iter().filter(|i| !judged.contains(i.as_str())).count();
    if absent > 0 {
        return Err(Error::rejected(
            "incomplete_coverage",
            format!(
                "the candidate's records leave out {absent} holdout items; no budget was spent"
            ),
        ));
    }
    // The referee's own state must still be what was approved; a mismatch is the
    // referee's to fix, so it spends nothing either.
    let project = Project::open(root)?;
    let state = referee_state(
        referee,
        &project,
        (&args.approved, &approved),
        &entries,
        held,
    )?;

    let used = Referee::accesses(&entries, &holdout);
    if used >= budget {
        let envelope = answer_envelope(
            &args.approved,
            &args.candidate,
            Answer {
                code: VerdictCode::BudgetExhausted,
                text: format!("the holdout's budget of {budget} submissions is spent"),
                conditions: Vec::new(),
                ladder: None,
            },
            (budget, used),
        );
        return Ok(Output {
            envelope,
            exit: Exit::BudgetExhausted,
        });
    }
    // From here every outcome spends one submission and leaves a gate line.
    referee.record(
        Kind::HoldoutAccess,
        vec![args.approved.clone(), candidate_hash.clone()],
        vec![],
        json!({ "holdout": holdout, "approved": args.approved, "candidate": candidate_hash, "run": args.candidate }),
    )?;
    let used = used + 1;
    let mut recorded: Vec<String> = Vec::new();
    let answer = mix_ups(&approved, args, &run, &judgments).and_then(|()| {
        judge(
            &Judging {
                approved: &approved,
                request: args,
                candidate_hash: &candidate_hash,
                state: &state,
                run: &run,
                judgments: &judgments,
                records: &records,
            },
            &mut recorded,
        )
    });
    let seal_void = referee.seal_void(&project);
    let gate_line =
        |verdict: Value, conditions: Value, error: Option<&str>, ladder: Option<String>| {
            referee.record(
                Kind::Gate,
                vec![args.approved.clone(), candidate_hash.clone()],
                vec![],
                json!({
                    "approved": args.approved,
                    "candidate": candidate_hash,
                    "run": args.candidate,
                    "verdict": verdict,
                    "conditions": conditions,
                    "error": error,
                    "agent_ledger_head": args.agent_ledger_head,
                    "seal_void": seal_void,
                    "withheld_warnings": recorded,
                    "ladder_value": ladder,
                }),
            )
        };
    let mut answer = match answer {
        Ok(a) => a,
        Err(e) => {
            gate_line(Value::Null, Value::Null, Some(&e.code), None)?;
            if e.exit == Exit::Rejected {
                std::fs::write(&result_path, e.to_json())
                    .map_err(|x| io_error(&result_path, &x))?;
            }
            return Err(e);
        }
    };
    let new_best = answer.ladder.take();
    let ladder_value = new_best
        .as_ref()
        .and_then(|b| b.shown["value"].as_f64())
        .map(|v| format!("{v:.6}"));
    let mut envelope = answer_envelope(&args.approved, &args.candidate, answer, (budget, used));
    let best_records = new_best.map(|b| {
        envelope.results["ladder"] = b.shown;
        b.records
    });
    gate_line(
        json!(envelope.verdict.as_ref().map(|v| v.code)),
        json!(
            envelope.results["conditions"]
                .as_array()
                .map(|c| c.iter().map(|x| x["passed"].clone()).collect::<Vec<_>>())
        ),
        None,
        ladder_value,
    )?;
    if seal_void {
        envelope.warnings.push(Warning {
            code: "seal_void".into(),
            message: "the referee's identity is readable from the agent's side, so the seal protects nothing".into(),
        });
    }
    let exit = exit_for(envelope.verdict.as_ref().map(|v| v.code));
    std::fs::write(&result_path, envelope.to_pretty_json())
        .map_err(|e| io_error(&result_path, &e))?;
    // The best moves only once the gate line and the result are written, so a failure
    // before that leaves the earlier best in place.
    if let Some(records) = best_records {
        crate::home::replace_private(&best_path(referee, &args.approved), records.as_bytes())?;
    }
    Ok(Output { envelope, exit })
}

/// What the referee fixed at approval, read and checked before a submission counts:
/// its project's data, the holdout's attributes, the baseline, the seal and the
/// decrypted labels of the held items.
struct RefereeState {
    d: data::Data,
    held: BTreeSet<String>,
    labels: Vec<Label>,
    /// The Ladder's best candidate so far, when one has beaten the baseline.
    best: Option<(Run, Vec<Judgment>)>,
}

fn referee_state(
    referee: &Referee,
    project: &Project,
    (approved_hash, approved): (&str, &Approved),
    entries: &[evalsig_store::ledger::Entry],
    held: BTreeSet<String>,
) -> Result<RefereeState, Error> {
    let r = &approved.resolved;
    let stale = |what: &str| {
        Error::new(
            "approval_out_of_date",
            format!("{what} since approval; approve the conditions again (no budget was spent)"),
            Exit::Rejected,
        )
    };
    let d = data::load(project)?;
    if approve::holdout_items_hash(&d, &held)? != r.holdout_items_hash {
        return Err(stale("the holdout's items or their attributes changed"));
    }
    if let Some(b) = approved.conditions.compare.baseline.as_deref()
        && Some(approve::baseline_hash(&d, b)?) != r.baseline_snapshot
    {
        return Err(stale("the baseline's records changed"));
    }
    let holdout = r.holdout.as_str();
    let seal = entries.iter().rev().find(|e| {
        e.kind == Kind::Seal && e.summary.get("holdout").and_then(Value::as_str) == Some(holdout)
    });
    if seal.map(|e| e.hash.as_str()) != Some(r.seal_entry.as_str()) {
        return Err(stale("the holdout was sealed again"));
    }
    // The sealed labels, in memory only.
    let sealed = std::fs::read(referee.file("sealed", holdout, "age")).map_err(|e| {
        Error::rejected(
            "seal",
            format!("the holdout's sealed labels cannot be read: {e}"),
        )
    })?;
    let plain = seal::open(&referee.identity()?, &sealed)
        .map_err(|e| Error::rejected("seal", e.to_string()))?;
    let mut set = LabelSet::default();
    for line in String::from_utf8_lossy(&plain).lines() {
        if let Ok(Record::Label(l)) = Record::from_json(line) {
            set.insert(l)
                .map_err(|e| Error::rejected("seal", e.to_string()))?;
        }
    }
    let labels = set
        .latest()
        .into_iter()
        .filter(|l| held.contains(&l.item_id))
        .cloned()
        .collect();
    let best = match std::fs::read_to_string(best_path(referee, approved_hash)) {
        Ok(text) => {
            let mut run = None;
            let mut judgments = Vec::new();
            for line in text.lines() {
                match Record::from_json(line)
                    .map_err(|e| Error::rejected("ladder", e.to_string()))?
                {
                    Record::Run(r) => run = Some(r),
                    Record::Judgment(j) => judgments.push(j),
                    _ => {}
                }
            }
            run.map(|r| (r, judgments))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_error(&best_path(referee, approved_hash), &e)),
    };
    Ok(RefereeState {
        d,
        held,
        labels,
        best,
    })
}

/// The judgments of the holdout's items.
fn on_holdout<'a>(js: &[&'a Judgment], items: &BTreeMap<&str, &Item>) -> Vec<&'a Judgment> {
    js.iter()
        .copied()
        .filter(|j| items.contains_key(j.item_id.as_str()))
        .collect()
}

/// Where the Ladder keeps the best candidate, in the referee's state: per approval, so
/// conditions on another metric or a re-approval start again from the baseline.
fn best_path(referee: &Referee, approved: &str) -> std::path::PathBuf {
    referee.file("state", approved, "best.jsonl")
}

fn exit_for(code: Option<VerdictCode>) -> Exit {
    match code {
        Some(VerdictCode::Pass) => Exit::Success,
        Some(VerdictCode::Fail) => Exit::Failed,
        Some(VerdictCode::BudgetExhausted) => Exit::BudgetExhausted,
        Some(VerdictCode::InsufficientData) => Exit::InsufficientData,
        Some(VerdictCode::Blocked) => Exit::Blocked,
        _ => Exit::Rejected,
    }
}

/// What the candidate's side may see: the verdict, each condition's pass or fail and
/// the budget left. No interval, value, item result or label-dependent warning.
fn answer_envelope(
    approved: &str,
    candidate: &str,
    a: Answer,
    (budget, used): (u64, u64),
) -> Envelope {
    let mut e = Envelope::new(
        Schema::Gate,
        json!({
            "approved": approved,
            "candidate": candidate,
            "preregistered": true,
            "conditions": a.conditions,
            "budget": { "total": budget, "used": used, "remaining": budget.saturating_sub(used) },
        }),
    );
    e.verdict = Some(Verdict {
        code: a.code,
        text: a.text,
    });
    e.provenance.ledger = Some("referee".into());
    e
}

fn stopped(code: VerdictCode, text: String) -> Answer {
    Answer {
        code,
        text,
        conditions: Vec::new(),
        ladder: None,
    }
}

/// The mix-up checks that need no data (exit 3): the candidate must be what was
/// approved. They run after the submission is counted, before the project is loaded.
fn mix_ups(
    approved: &Approved,
    args: &Request,
    run: &Run,
    judgments: &[&Judgment],
) -> Result<(), Error> {
    let (c, r) = (&approved.conditions, &approved.resolved);
    let refuse = |what: &str| -> Result<(), Error> {
        Err(Error::rejected("does_not_match_approval", what.to_owned()))
    };
    let grader = run.grader.as_ref();
    if grader.and_then(|g| g.id.as_deref()) != Some(c.grader.id.as_str())
        || grader.and_then(|g| g.version.as_deref()) != Some(c.grader.version.as_str())
    {
        return refuse("the candidate's run.grader differs from the approved grader");
    }
    if c.grader
        .config_hash
        .as_deref()
        .is_some_and(|h| grader.and_then(|g| g.config_hash.as_deref()) != Some(h))
    {
        return refuse("the candidate's grader configuration differs from the approved one");
    }
    // Pinned from the baseline's run; an approval without a baseline pins none of them.
    let levels_match = r.levels.is_none_or(|l| run.levels == Some(l));
    let type_match = r.output_type.is_none_or(|t| run.output_type == Some(t));
    let semantics_match = r.value_semantics.is_none_or(|v| run.value_semantics() == v);
    if !type_match || !levels_match || !semantics_match {
        return refuse(
            "the candidate's output type, levels or value semantics differ from the approval",
        );
    }
    if let Some(m) = r.mapping_hash.as_deref()
        && args.mapping_hash.as_deref() != Some(m)
    {
        return refuse("the candidate's records were not made with the approved mapping");
    }
    if judgments.iter().any(|j| j.task() != r.task) {
        return refuse("the candidate's judgments are for another task");
    }
    // An @k estimand scores only items with at least k replicates, so k is the floor.
    let floor = match r.estimand {
        Estimand::SingleShot => None,
        Estimand::Majority(k) | Estimand::Any(k) | Estimand::All(k) => Some(u64::from(k)),
    };
    if let Some(want) = c.replicates.max(floor) {
        let mut per_item: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
        for j in judgments {
            per_item
                .entry(j.item_id.as_str())
                .or_default()
                .insert(j.gen_replicate);
        }
        if per_item.values().any(|r| (r.len() as u64) < want) {
            return refuse(&format!(
                "some items have fewer than the approved {want} generation replicates"
            ));
        }
    }
    Ok(())
}

/// One submission being judged: the approval, the referee's state and the candidate.
struct Judging<'a> {
    approved: &'a Approved,
    request: &'a Request,
    candidate_hash: &'a str,
    state: &'a RefereeState,
    run: &'a Run,
    judgments: &'a [&'a Judgment],
    records: &'a [Record],
}

/// The holdout as the conditions read it: its items and labels, the pinned baseline's
/// judgments on it, the settings every condition starts from and the budget.
struct Holdout<'a> {
    items: BTreeMap<&'a str, &'a Item>,
    labels: Vec<&'a Label>,
    base: Option<(&'a Run, Vec<&'a Judgment>)>,
    settings: Settings,
    budget: u32,
}

/// The checks after the submission counts, and the conditions themselves.
fn judge(j: &Judging, recorded: &mut Vec<String>) -> Result<Answer, Error> {
    let (approved, r) = (&j.approved.conditions, &j.approved.resolved);
    let (d, held) = (&j.state.d, &j.state.held);
    // Replicates that are all the same stop the gate (no override).
    let findings = validity::dataset(j.records, &Overrides::default());
    if findings.iter().any(|f| f.code == "identical_replicates") {
        return Ok(stopped(
            VerdictCode::Blocked,
            "every generation replicate of some items is identical".into(),
        ));
    }
    // A value of another type is unreadable and escapes the missing_rate count, so a run
    // that mixes output types or level counts is refused like any other mismatch.
    if findings.iter().any(|f| f.code == "mixed_output_type") {
        return Err(Error::rejected(
            "does_not_match_approval",
            "the candidate's judgments mix output types or level counts",
        ));
    }
    recorded.extend(
        findings
            .iter()
            .filter(|f| f.code != "identical_replicates")
            .map(|f| f.code.to_owned()),
    );
    let labels: Vec<&Label> = j
        .state
        .labels
        .iter()
        .filter(|l| l.task() == r.task)
        .collect();
    // Items whose references come only from models cannot be protected: stop.
    let mut model_only: BTreeMap<&str, bool> = BTreeMap::new();
    for l in &labels {
        *model_only.entry(l.item_id.as_str()).or_insert(true) &= l.source == Source::Model;
    }
    if model_only.values().any(|m| *m) {
        return Ok(stopped(
            VerdictCode::Blocked,
            "some holdout items have model-derived references only".into(),
        ));
    }

    // The baseline from the referee's own records, as pinned.
    let base = approved
        .compare
        .baseline
        .as_deref()
        .map(|b| -> Result<_, Error> {
            let judgments = d
                .judgments(b)
                .into_iter()
                .filter(|x| held.contains(&x.item_id))
                .collect::<Vec<_>>();
            Ok((d.run(b)?, judgments))
        })
        .transpose()?;

    // min_items counts paired items, independent of the labels.
    let base_items: Option<BTreeSet<&str>> = base
        .as_ref()
        .map(|(_, js)| js.iter().map(|x| x.item_id.as_str()).collect());
    let paired = held
        .iter()
        .filter(|i| base_items.as_ref().is_none_or(|b| b.contains(i.as_str())))
        .count() as u64;
    if paired < approved.min_items {
        return Ok(stopped(
            VerdictCode::InsufficientData,
            format!("{paired} paired items, fewer than min_items"),
        ));
    }

    let h = Holdout {
        items: d
            .items()
            .into_iter()
            .filter(|(id, _)| held.contains(*id))
            .collect(),
        labels,
        base,
        settings: settings(r, j.request, j.candidate_hash),
        budget: approved.budget(),
    };
    let mut rows = Vec::new();
    let mut outcomes: Vec<Outcome> = Vec::new();
    for c in &approved.conditions {
        let level = c.level.unwrap_or(r.level);
        let at = Settings {
            level: gate::interval_level(level, h.budget, c.sided),
            threshold: c.threshold,
            ..h.settings.clone()
        };
        let evidence = if c.metric == "human_parity" {
            // The raters are tested together at the submission's level α/B, so the
            // budget keeps its error rate (metrics spec §9.4, §11.1).
            let p = report::prepare(
                j.run,
                &on_holdout(j.judgments, &h.items),
                &h.items,
                &h.labels,
                &at,
            )
            .map_err(Error::report)?;
            // Approval fixes epsilon, since the gate reads no configuration.
            let epsilon = c.value.ok_or_else(|| {
                Error::invalid_conditions(
                    "human_parity has no epsilon; approve the conditions again",
                )
            })?;
            let alpha = (1.0 - level) / f64::from(h.budget);
            match parity::of_run(&p, &h.labels, epsilon, alpha).verdict {
                ParityVerdict::HumanParity => Some(Evidence::Verdict(true)),
                ParityVerdict::BelowHuman => Some(Evidence::Verdict(false)),
                ParityVerdict::InsufficientData => None,
            }
        } else {
            evidence(j, &h, &c.metric, c.rule, c.sided, &at)?
        };
        let o = gate::judge(c.rule, c.value.unwrap_or(0.0), c.min_effect, evidence);
        outcomes.push(o);
        rows.push(
            json!({ "metric": c.metric, "rule": c.rule, "value": c.value, "passed": o.passed }),
        );
    }
    let (code, text) = match gate::overall(&outcomes) {
        Some(true) => (
            VerdictCode::Pass,
            "every approved condition holds on the holdout",
        ),
        Some(false) => (
            VerdictCode::Fail,
            "an approved condition does not hold on the holdout",
        ),
        None => (
            VerdictCode::InsufficientData,
            "a condition could not be decided on the holdout",
        ),
    };
    Ok(Answer {
        code,
        text: text.into(),
        conditions: rows,
        ladder: ladder(j, &h)?,
    })
}

/// The Ladder (metrics spec §11.3, D-015): a value only on a new best, when the lower
/// bound of the candidate's paired difference to the best so far, one-sided at α/B, is
/// above 0 and the difference reaches the approved step; the value is rounded to the
/// step toward the baseline. The first best is the baseline.
fn ladder(j: &Judging, h: &Holdout) -> Result<Option<NewBest>, Error> {
    let (Some(l), Some((base_run, base_judgments))) = (&j.approved.resolved.ladder, &h.base) else {
        return Ok(None);
    };
    let (items, labels) = (&h.items, h.labels.as_slice());
    let primary = &j.approved.conditions.conditions[0];
    let level = primary.level.unwrap_or(j.approved.resolved.level);
    let at = Settings {
        level: gate::interval_level(level, h.budget, Sided::One),
        ..h.settings.clone()
    };
    let cand = report::prepare(j.run, &on_holdout(j.judgments, items), items, labels, &at)
        .map_err(Error::report)?;
    let Some(id) = cand
        .metric_ids()
        .into_iter()
        .find(|id| id.name() == l.metric)
    else {
        return Ok(None);
    };
    let Some(value) = compare::total_ratio(&cand.item_values(id, Fill::Policy)) else {
        return Ok(None);
    };
    let stored: Vec<&Judgment>;
    let best = match &j.state.best {
        Some((best_run, best_judgments)) => {
            stored = best_judgments.iter().collect();
            report::prepare(best_run, &on_holdout(&stored, items), items, labels, &at)
        }
        None => report::prepare(base_run, base_judgments, items, labels, &at),
    }
    .map_err(Error::report)?;
    let options = Options {
        level: at.level,
        bootstrap: at.bootstrap,
        seed: at.seed,
        min_effect: BTreeMap::new(),
        excluded: BTreeSet::new(),
        scope: None,
    };
    // MetricDelta is oriented so that positive is better.
    let Some(m) = compare::compare(&Baseline::new(&best), &cand, &options)
        .metrics
        .into_iter()
        .find(|m| m.metric == l.metric)
    else {
        return Ok(None);
    };
    if !m
        .ci
        .zip(m.delta)
        .is_some_and(|((lo, _), d)| lo > 0.0 && d >= l.step)
    {
        return Ok(None);
    }
    // A difference rule returns the difference to the baseline as the rule reads it,
    // oriented so that positive is better, rounded toward 0.
    let shown = if primary.rule.on_delta() {
        let gap = value - l.first_best;
        let oriented = if m.direction == "higher_is_better" {
            gap
        } else {
            -gap
        };
        gate::round_toward(oriented, 0.0, l.step)
    } else {
        gate::round_toward(value, l.first_best, l.step)
    };
    let records = approve::run_records(j.records, &j.run.run_id)
        .map(|r| format!("{}\n", serde_json::to_string(r).expect("records serialize")))
        .collect();
    Ok(Some(NewBest {
        shown: json!({ "metric": l.metric, "value": shown, "step": l.step }),
        records,
    }))
}

/// The resolved settings, every missing value the worst (SPEC §8.3), and a seed
/// derived from the approval and the candidate.
fn settings(r: &Resolved, args: &Request, candidate_hash: &str) -> Settings {
    let seed_bytes =
        ContentHash::blake3(format!("{}{candidate_hash}", args.approved).as_bytes()).hex();
    Settings {
        task: r.task.clone(),
        positive: r.positive,
        decision: r.decision,
        threshold: None,
        estimand: r.estimand,
        missing: MissingPolicy::holdout(),
        reference: r.reference_mode,
        rater_design: r.rater_design,
        level: r.level,
        bootstrap: r.bootstrap,
        seed: u64::from_str_radix(&seed_bytes[..16], 16).unwrap_or(0),
        prevalence: None,
        sections: Sections::default(),
        exact_rates: true,
        proxy: None,
    }
}

/// The evidence a condition reads, at the condition's interval level.
fn evidence(
    j: &Judging,
    h: &Holdout,
    metric: &str,
    rule: GateRule,
    sided: Sided,
    s: &Settings,
) -> Result<Option<Evidence>, Error> {
    let (run, items, labels) = (j.run, &h.items, h.labels.as_slice());
    let cand_j = on_holdout(j.judgments, items);
    if metric == "missing_rate" {
        // Every missing judgment counts, whatever its reason (SPEC §8.3).
        let (missing, n) = gate::missing_counts(&cand_j, |j| j.missing.is_some());
        return Ok(gate::missing_rate(missing, n, s.level, sided));
    }
    if rule.on_delta() {
        let Some((base_run, base_judgments)) = &h.base else {
            return Ok(None);
        };
        let pa =
            report::prepare(base_run, base_judgments, items, labels, s).map_err(Error::report)?;
        let pb = report::prepare(run, &cand_j, items, labels, s).map_err(Error::report)?;
        let options = Options {
            level: s.level,
            bootstrap: s.bootstrap,
            seed: s.seed,
            min_effect: BTreeMap::new(),
            excluded: BTreeSet::new(),
            scope: None,
        };
        let c = compare::compare(&Baseline::new(&pa), &pb, &options);
        let Some(m) = c.metrics.iter().find(|m| m.metric == metric) else {
            return Err(not_computed(metric));
        };
        // MetricDelta is oriented so that positive is better, as the difference rules read
        // it (D-034).
        return Ok(match (m.delta, m.ci) {
            (Some(point), Some((lo, hi))) => Some(Evidence::Delta { point, lo, hi }),
            _ => None,
        });
    }
    let r = report::build(run, &cand_j, items, labels, s).map_err(Error::report)?;
    let m = r
        .metrics
        .iter()
        .find(|m| m.metric == metric)
        .ok_or_else(|| not_computed(metric))?;
    Ok(match (m.value, m.ci) {
        (Some(point), Some((lo, hi))) => Some(Evidence::Value { point, lo, hi }),
        _ => None,
    })
}

/// A condition on a metric the gate does not compute for this run is an error in the
/// approval, not too little data.
fn not_computed(metric: &str) -> Error {
    Error::rejected(
        "metric_not_computed",
        format!("the gate computes no {metric} for this run; approve conditions on its metrics"),
    )
}
