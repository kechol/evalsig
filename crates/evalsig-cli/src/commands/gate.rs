//! `evalsig gate`: judge a candidate on the sealed holdout under approved conditions
//! (SPEC §8.3, §8.4; metrics spec §11.1, §11.2). Everything from the candidate's side
//! is a claim the referee checks; only the approved copy, the referee ledger and the
//! sealed labels are trusted. The answer carries verdicts only, no values (D-015 is
//! Proposed).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use evalsig_core::checks::Overrides;
use evalsig_core::compare::{self, Baseline, Options};
use evalsig_core::config::{Estimand, RaterDesign, ReferenceMode};
use evalsig_core::gate::{self, Evidence, Outcome, Rule as GateRule, Sided};
use evalsig_core::labels::LabelSet;
use evalsig_core::missing::MissingPolicy;
use evalsig_core::records::{Item, Judgment, Label, Record, Run, Source};
use evalsig_core::report::{self, Settings};
use evalsig_core::rules::Rule;
use evalsig_core::validity;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use evalsig_store::seal;
use serde_json::{Value, json};

use crate::commands::{approve, gate_serve};
use crate::data;
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode, Warning};
use crate::project::{Project, hex, io_error};
use crate::referee::Referee;
use crate::validate::RecordValidator;

#[derive(Debug, Clone, clap::Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct Args {
    #[command(subcommand)]
    pub action: Option<Action>,
    #[command(flatten)]
    pub submission: Submission,
    /// The referee's home (default: EVALSIG_REFEREE).
    #[arg(long, value_name = "DIR")]
    pub referee: Option<PathBuf>,
}

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// As the referee: answer submissions on a Unix socket in the referee home and,
    /// with --http, on token HTTP at 127.0.0.1.
    Serve(gate_serve::ServeArgs),
    /// Submit a candidate to a running `gate serve` and print its answer.
    Submit(gate_serve::SubmitArgs),
}

#[derive(Debug, Clone, clap::Args)]
pub struct Submission {
    /// The approved conditions, by hash.
    #[arg(long, value_name = "HASH", required = true)]
    pub approved: Option<String>,
    /// The candidate's run ID.
    #[arg(long, value_name = "RUN", required = true)]
    pub candidate: Option<String>,
    /// The candidate's records (records/1 JSONL): its run and judgments.
    #[arg(long, value_name = "FILE", required = true)]
    pub records: Option<PathBuf>,
    /// The hash of the mapping or converter that made the records, as the candidate's
    /// side reports it.
    #[arg(long, value_name = "HASH")]
    pub mapping_hash: Option<String>,
    /// The agent-side ledger, whose head is copied into the gate line.
    #[arg(long, value_name = "FILE")]
    pub agent_ledger: Option<PathBuf>,
}

/// One submission as the referee judges it.
#[derive(Debug, Clone)]
pub struct Request {
    pub approved: String,
    pub candidate: String,
    /// The candidate's records, as records/1 JSONL text.
    pub records: String,
    pub mapping_hash: Option<String>,
    pub agent_ledger_head: Option<String>,
}

impl Submission {
    /// The fields clap requires when no subcommand is given.
    pub fn parts(&self) -> (&str, &str, &Path) {
        (
            self.approved.as_deref().expect("required by clap"),
            self.candidate.as_deref().expect("required by clap"),
            self.records.as_deref().expect("required by clap"),
        )
    }

    /// The head of the agent-side ledger, if one was named; a named ledger that cannot
    /// be read is an error, not a missing head.
    pub fn agent_ledger_head(&self) -> Result<Option<String>, CliError> {
        let Some(p) = self.agent_ledger.as_ref() else {
            return Ok(None);
        };
        let entries = evalsig_store::ledger::read(p)
            .map_err(|e| CliError::usage(format!("--agent-ledger {}: {e}", p.display())))?;
        Ok(entries.last().map(|e| e.hash.clone()))
    }
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    match &args.action {
        Some(Action::Serve(a)) => gate_serve::serve(root, a),
        Some(Action::Submit(a)) => gate_serve::submit(a),
        None => {
            let s = &args.submission;
            let (approved, candidate, records) = s.parts();
            let referee = Referee::open(args.referee.as_deref())?;
            judge_request(
                root,
                &referee,
                &Request {
                    approved: approved.to_owned(),
                    candidate: candidate.to_owned(),
                    records: std::fs::read_to_string(records).map_err(|e| io_error(records, &e))?,
                    mapping_hash: s.mapping_hash.clone(),
                    agent_ledger_head: s.agent_ledger_head()?,
                },
            )
        }
    }
}

fn rejected(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::new(code, message, Exit::Rejected)
}

/// The answer the candidate's side receives.
struct Answer {
    code: VerdictCode,
    text: String,
    conditions: Vec<Value>,
}

/// Judge one submission: the checks, the budget, the conditions, the ledger lines and
/// the result file.
pub fn judge_request(root: &Path, referee: &Referee, args: &Request) -> Result<Output, CliError> {
    // One submission at a time per referee home, across processes: the budget is
    // read, then written.
    let _lock = referee.lock()?;
    let approved = approve::load(referee, &args.approved)?;
    let r = &approved["resolved"];
    let holdout = r["holdout"].as_str().unwrap_or_default().to_owned();
    let budget = approve::budget(&approved);

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
            rejected(
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
        let mut envelope: Envelope =
            serde_json::from_str(&saved).map_err(|e| CliError::invalid_input(&result_path, e))?;
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
            rejected(
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
        return Err(rejected(
            "incomplete_coverage",
            format!(
                "the candidate's records leave out {absent} holdout items; no budget was spent"
            ),
        ));
    }
    // The referee's own state must still be what was approved; a mismatch is the
    // referee's to fix, so it spends nothing either.
    let project = Project::open(root)?;
    let state = referee_state(referee, &project, &approved, &entries, &held)?;

    let used = entries
        .iter()
        .filter(|e| {
            e.kind == Kind::HoldoutAccess
                && e.summary.get("holdout").and_then(Value::as_str) == Some(holdout.as_str())
        })
        .count() as u64;
    if used >= budget {
        let envelope = answer_envelope(
            &args.approved,
            &args.candidate,
            Answer {
                code: VerdictCode::BudgetExhausted,
                text: format!("the holdout's budget of {budget} submissions is spent"),
                conditions: Vec::new(),
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
            &approved,
            args,
            &candidate_hash,
            &state,
            &run,
            &judgments,
            &records,
            &mut recorded,
        )
    });
    let seal_void = referee.seal_void(&project);
    let gate_line = |verdict: Value, conditions: Value, error: Option<&str>| {
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
            }),
        )
    };
    let answer = match answer {
        Ok(a) => a,
        Err(e) => {
            gate_line(Value::Null, Value::Null, Some(&e.code))?;
            return Err(e);
        }
    };
    let mut envelope = answer_envelope(&args.approved, &args.candidate, answer, (budget, used));
    gate_line(
        json!(envelope.verdict.as_ref().map(|v| v.code)),
        json!(
            envelope.results["conditions"]
                .as_array()
                .map(|c| c.iter().map(|x| x["passed"].clone()).collect::<Vec<_>>())
        ),
        None,
    )?;
    if seal_void {
        envelope.warnings.push(Warning {
            code: "seal_void".into(),
            message: "the referee's identity is readable from the agent's side, so the seal protects nothing".into(),
        });
    }
    let exit = exit_for(envelope.verdict.as_ref().map(|v| v.code));
    std::fs::write(
        &result_path,
        serde_json::to_string_pretty(&envelope).expect("envelope serializes"),
    )
    .map_err(|e| io_error(&result_path, &e))?;
    Ok(Output { envelope, exit })
}

/// What the referee fixed at approval, read and checked before a submission counts:
/// its project's data, the holdout's attributes, the baseline, the seal and the
/// decrypted labels of the held items.
struct RefereeState {
    d: data::Data,
    held: BTreeSet<String>,
    labels: Vec<Label>,
}

fn referee_state(
    referee: &Referee,
    project: &Project,
    approved: &Value,
    entries: &[evalsig_store::ledger::Entry],
    held: &BTreeSet<String>,
) -> Result<RefereeState, CliError> {
    let r = &approved["resolved"];
    let stale = |what: &str| {
        CliError::new(
            "approval_out_of_date",
            format!("{what} since approval; approve the conditions again (no budget was spent)"),
            Exit::Rejected,
        )
    };
    let d = data::load(project)?;
    if approve::holdout_items_hash(&d, held)?
        != r["holdout_items_hash"].as_str().unwrap_or_default()
    {
        return Err(stale("the holdout's items or their attributes changed"));
    }
    if let Some(b) = approved["compare"]["baseline"].as_str()
        && approve::baseline_hash(&d, b)? != r["baseline_snapshot"].as_str().unwrap_or_default()
    {
        return Err(stale("the baseline's records changed"));
    }
    let holdout = r["holdout"].as_str().unwrap_or_default();
    let seal = entries.iter().rev().find(|e| {
        e.kind == Kind::Seal && e.summary.get("holdout").and_then(Value::as_str) == Some(holdout)
    });
    if seal.map(|e| e.hash.as_str()) != r["seal_entry"].as_str() {
        return Err(stale("the holdout was sealed again"));
    }
    // The sealed labels, in memory only.
    let sealed = std::fs::read(referee.file("sealed", holdout, "age")).map_err(|e| {
        rejected(
            "seal",
            format!("the holdout's sealed labels cannot be read: {e}"),
        )
    })?;
    let plain =
        seal::open(&referee.identity()?, &sealed).map_err(|e| rejected("seal", e.to_string()))?;
    let mut set = LabelSet::default();
    for line in String::from_utf8_lossy(&plain).lines() {
        if let Ok(Record::Label(l)) = Record::from_json(line) {
            set.insert(l).map_err(|e| rejected("seal", e.to_string()))?;
        }
    }
    let labels = set
        .latest()
        .into_iter()
        .filter(|l| held.contains(&l.item_id))
        .cloned()
        .collect();
    Ok(RefereeState {
        d,
        held: held.clone(),
        labels,
    })
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
        "evalsig.gate/1",
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
    }
}

/// The mix-up checks that need no data (exit 3): the candidate must be what was
/// approved. They run after the submission is counted, before the project is loaded.
fn mix_ups(
    approved: &Value,
    args: &Request,
    run: &Run,
    judgments: &[&Judgment],
) -> Result<(), CliError> {
    let r = &approved["resolved"];
    let refuse = |what: &str| -> Result<(), CliError> {
        Err(rejected("does_not_match_approval", what.to_owned()))
    };
    let grader = run.grader.as_ref();
    if grader.and_then(|g| g.id.as_deref()) != approved["grader"]["id"].as_str()
        || grader.and_then(|g| g.version.as_deref()) != approved["grader"]["version"].as_str()
    {
        return refuse("the candidate's run.grader differs from the approved grader");
    }
    if approved["grader"]["config_hash"]
        .as_str()
        .is_some_and(|h| grader.and_then(|g| g.config_hash.as_deref()) != Some(h))
    {
        return refuse("the candidate's grader configuration differs from the approved one");
    }
    // Pinned from the baseline's run; an approval without a baseline pins none of them.
    let levels_match = r["levels"].is_null() || r["levels"].as_u64() == run.levels.map(u64::from);
    let type_match = r["output_type"].is_null() || json!(run.output_type) == r["output_type"];
    let semantics_match =
        r["value_semantics"].is_null() || json!(run.value_semantics()) == r["value_semantics"];
    if !type_match || !levels_match || !semantics_match {
        return refuse(
            "the candidate's output type, levels or value semantics differ from the approval",
        );
    }
    if let Some(m) = r["mapping_hash"].as_str()
        && args.mapping_hash.as_deref() != Some(m)
    {
        return refuse("the candidate's records were not made with the approved mapping");
    }
    let task = r["task"]
        .as_str()
        .unwrap_or(evalsig_core::records::DEFAULT_TASK);
    if judgments.iter().any(|j| j.task() != task) {
        return refuse("the candidate's judgments are for another task");
    }
    if let Some(want) = approved["replicates"].as_u64() {
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

/// The checks after the submission counts, and the conditions themselves.
#[allow(clippy::too_many_arguments)]
fn judge(
    approved: &Value,
    args: &Request,
    candidate_hash: &str,
    state: &RefereeState,
    run: &Run,
    judgments: &[&Judgment],
    records: &[Record],
    recorded: &mut Vec<String>,
) -> Result<Answer, CliError> {
    let r = &approved["resolved"];
    let (d, held) = (&state.d, &state.held);
    let task = r["task"]
        .as_str()
        .unwrap_or(evalsig_core::records::DEFAULT_TASK);
    let items: BTreeMap<&str, &Item> = d
        .items()
        .into_iter()
        .filter(|(id, _)| held.contains(*id))
        .collect();
    // Replicates that are all the same stop the gate (no override).
    let findings = validity::dataset(records, &Overrides::default());
    if findings.iter().any(|f| f.code == "identical_replicates") {
        return Ok(stopped(
            VerdictCode::Blocked,
            "every generation replicate of some items is identical".into(),
        ));
    }
    recorded.extend(
        findings
            .iter()
            .filter(|f| f.code != "identical_replicates")
            .map(|f| f.code.to_owned()),
    );
    let labels: Vec<&Label> = state.labels.iter().filter(|l| l.task() == task).collect();
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
    let baseline = approved["compare"]["baseline"].as_str();
    let base_run = baseline.map(|b| d.run(b)).transpose()?;
    let base_judgments: Vec<&Judgment> = baseline
        .map(|b| {
            d.judgments(b)
                .into_iter()
                .filter(|j| held.contains(&j.item_id))
                .collect()
        })
        .unwrap_or_default();

    // min_items counts paired items, independent of the labels.
    let base_items: BTreeSet<&str> = base_judgments.iter().map(|j| j.item_id.as_str()).collect();
    let paired = held
        .iter()
        .filter(|i| base_run.is_none() || base_items.contains(i.as_str()))
        .count() as u64;
    if paired < approved["min_items"].as_u64().unwrap_or(1) {
        return Ok(stopped(
            VerdictCode::InsufficientData,
            format!("{paired} paired items, fewer than min_items"),
        ));
    }

    let budget = approve::budget(approved) as u32;
    let settings = settings(r, args, candidate_hash);
    let mut rows = Vec::new();
    let mut outcomes: Vec<Outcome> = Vec::new();
    for c in approved["conditions"].as_array().into_iter().flatten() {
        let rule: GateRule = serde_json::from_value(c["rule"].clone())
            .map_err(|e| rejected("invalid_conditions", e.to_string()))?;
        let sided: Sided = serde_json::from_value(c["sided"].clone())
            .map_err(|e| rejected("invalid_conditions", e.to_string()))?;
        let level = c["level"]
            .as_f64()
            .unwrap_or(r["level"].as_f64().unwrap_or(0.95));
        let at = Settings {
            level: gate::interval_level(level, budget, sided),
            threshold: c["threshold"].as_f64(),
            ..settings.clone()
        };
        let metric = c["metric"].as_str().unwrap_or_default();
        let evidence = evidence(
            metric,
            rule,
            sided,
            &at,
            run,
            judgments,
            base_run,
            &base_judgments,
            &items,
            &labels,
        )?;
        let o = gate::judge(
            rule,
            c["value"].as_f64().unwrap_or(0.0),
            c["min_effect"].as_f64(),
            evidence,
        );
        outcomes.push(o);
        rows.push(
            json!({ "metric": metric, "rule": c["rule"], "value": c["value"], "passed": o.passed }),
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
    })
}

/// The resolved settings, every missing value the worst (SPEC §8.3), and a seed
/// derived from the approval and the candidate.
fn settings(r: &Value, args: &Request, candidate_hash: &str) -> Settings {
    let rule = |k: &str| -> Option<Rule> { serde_json::from_value(r[k].clone()).ok() };
    let seed_bytes =
        ContentHash::blake3(format!("{}{candidate_hash}", args.approved).as_bytes()).hex();
    Settings {
        task: r["task"]
            .as_str()
            .unwrap_or(evalsig_core::records::DEFAULT_TASK)
            .to_owned(),
        positive: rule("positive"),
        decision: rule("decision"),
        threshold: None,
        estimand: r["estimand"]
            .as_str()
            .and_then(|e| e.parse::<Estimand>().ok())
            .unwrap_or(Estimand::SingleShot),
        missing: MissingPolicy::holdout(),
        reference: serde_json::from_value(r["reference_mode"].clone())
            .unwrap_or(ReferenceMode::Soft),
        rater_design: serde_json::from_value(r["rater_design"].clone())
            .unwrap_or(RaterDesign::Auto),
        level: r["level"].as_f64().unwrap_or(0.95),
        bootstrap: r["bootstrap"].as_u64().unwrap_or(2000) as u32,
        seed: u64::from_str_radix(&seed_bytes[..16], 16).unwrap_or(0),
        prevalence: None,
        by: None,
        items: false,
        detail: false,
        features: false,
        exact_rates: true,
        proxy: None,
    }
}

/// The evidence a condition reads, at the condition's interval level.
#[allow(clippy::too_many_arguments)]
fn evidence(
    metric: &str,
    rule: GateRule,
    sided: Sided,
    s: &Settings,
    run: &Run,
    judgments: &[&Judgment],
    base_run: Option<&Run>,
    base_judgments: &[&Judgment],
    items: &BTreeMap<&str, &Item>,
    labels: &[&Label],
) -> Result<Option<Evidence>, CliError> {
    let cand_j: Vec<&Judgment> = judgments
        .iter()
        .copied()
        .filter(|j| items.contains_key(j.item_id.as_str()))
        .collect();
    if metric == "missing_rate" {
        // Every missing judgment counts, whatever its reason (SPEC §8.3).
        let missing = cand_j.iter().filter(|j| j.missing.is_some()).count() as u64;
        return Ok(gate::missing_rate(
            missing,
            cand_j.len() as u64,
            s.level,
            sided,
        ));
    }
    if rule.on_delta() {
        let Some(base_run) = base_run else {
            return Ok(None);
        };
        let pa = report::prepare(base_run, base_judgments, items, labels, s)
            .map_err(CliError::report)?;
        let pb = report::prepare(run, &cand_j, items, labels, s).map_err(CliError::report)?;
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
        // MetricDelta is oriented so that positive is better; the rules read cand − base.
        let sign = if m.direction == "higher_is_better" {
            1.0
        } else {
            -1.0
        };
        return Ok(match (m.delta, m.ci) {
            (Some(dl), Some((lo, hi))) => Some(if sign > 0.0 {
                Evidence::Delta { point: dl, lo, hi }
            } else {
                Evidence::Delta {
                    point: -dl,
                    lo: -hi,
                    hi: -lo,
                }
            }),
            _ => None,
        });
    }
    let r = report::build(run, &cand_j, items, labels, s).map_err(CliError::report)?;
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
fn not_computed(metric: &str) -> CliError {
    rejected(
        "metric_not_computed",
        format!("the gate computes no {metric} for this run; approve conditions on its metrics"),
    )
}
