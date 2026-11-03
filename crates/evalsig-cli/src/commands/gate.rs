//! `evalsig gate`: judge a candidate on the sealed holdout under approved conditions
//! (SPEC §8.3, §8.4; metrics spec §11.1, §11.2). Everything from the candidate's side
//! is a claim the referee checks; only the approved copy, the referee ledger and the
//! sealed labels are trusted. The Ladder (D-015) is not applied yet, so no value is
//! returned, only verdicts.

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
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use evalsig_store::seal;
use serde_json::{Value, json};

use crate::commands::approve;
use crate::data;
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode, Warning};
use crate::project::{Project, io_error};
use crate::referee::Referee;
use crate::validate::RecordValidator;

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The approved conditions, by hash.
    #[arg(long, value_name = "HASH")]
    pub approved: String,
    /// The candidate's run ID.
    #[arg(long, value_name = "RUN")]
    pub candidate: String,
    /// The candidate's records (records/1 JSONL): its run and judgments.
    #[arg(long, value_name = "FILE")]
    pub records: PathBuf,
    /// The hash of the mapping or converter that made the records, as the candidate's
    /// side reports it.
    #[arg(long, value_name = "HASH")]
    pub mapping_hash: Option<String>,
    /// The agent-side ledger, whose head is copied into the gate line.
    #[arg(long, value_name = "FILE")]
    pub agent_ledger: Option<PathBuf>,
    /// The referee's home (default: EVALSIG_REFEREE).
    #[arg(long, value_name = "DIR")]
    pub referee: Option<PathBuf>,
}

fn rejected(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::new(code, message, Exit::Rejected)
}

fn hex(h: &str) -> &str {
    h.strip_prefix("blake3:").unwrap_or(h)
}

/// The answer the candidate's side receives.
struct Answer {
    code: VerdictCode,
    text: String,
    conditions: Vec<Value>,
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    let referee = Referee::open(args.referee.as_deref())?;
    let approved = approve::load(&referee, &args.approved)?;
    let r = &approved["resolved"];
    let holdout = r["holdout"].as_str().unwrap_or_default().to_owned();
    let budget = approved["holdout"]["budget"].as_u64().unwrap_or(5);

    // The candidate's records: checked like any import, named by their canonical hash.
    let text = std::fs::read_to_string(&args.records).map_err(|e| io_error(&args.records, &e))?;
    let validator = RecordValidator::new();
    let mut records = Vec::new();
    let mut lines = Vec::new();
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let rec = validator.check_line(n + 1, line).map_err(|issues| {
            rejected(
                "invalid_records",
                format!("line {}: {}", n + 1, issues[0].message),
            )
        })?;
        lines.push(
            to_canonical_bytes(&serde_json::to_value(&rec).expect("records serialize"))
                .map_err(CliError::store)?,
        );
        records.push(rec);
    }
    lines.sort();
    let candidate_hash = ContentHash::blake3(&lines.concat()).to_string();

    // The same candidate again: the earlier answer, without spending budget.
    let result_path = referee.path("results").join(format!(
        "{}-{}.json",
        hex(&args.approved),
        hex(&candidate_hash)
    ));
    if let Ok(saved) = std::fs::read_to_string(&result_path) {
        let saved: Value =
            serde_json::from_str(&saved).map_err(|e| CliError::invalid_input(&result_path, e))?;
        let code = VerdictCode::ALL
            .into_iter()
            .find(|c| saved["verdict"]["code"] == c.as_str());
        let mut envelope = Envelope::new("evalsig.gate/1", saved["results"].clone());
        envelope.verdict = code.map(|code| Verdict {
            code,
            text: saved["verdict"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        });
        envelope.provenance.ledger = Some("referee".into());
        let exit = exit_for(code);
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
    let held = referee.holdout_items(&holdout)?;

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

    let entries = referee.entries()?;
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
    // From here every outcome spends one submission.
    referee.record(
        Kind::HoldoutAccess,
        vec![args.approved.clone(), candidate_hash.clone()],
        vec![],
        json!({ "holdout": holdout, "approved": args.approved, "candidate": candidate_hash }),
    )?;
    let used = used + 1;

    let project = Project::open(root)?;
    let d = data::load(&project)?;
    let mut recorded: Vec<String> = Vec::new();
    let answer = judge(
        &approved,
        args,
        &referee,
        &d,
        &run,
        &judgments,
        &records,
        &held,
        &mut recorded,
    )?;
    let envelope = answer_envelope(&args.approved, &args.candidate, answer, (budget, used));
    let seal_void = referee.seal_void(&project);
    let head = match &args.agent_ledger {
        Some(p) => evalsig_store::ledger::read(p)
            .ok()
            .and_then(|e| e.last().map(|e| e.hash.clone())),
        None => None,
    };
    referee.record(
        Kind::Gate,
        vec![args.approved.clone(), candidate_hash.clone()],
        vec![],
        json!({
            "approved": args.approved,
            "candidate": candidate_hash,
            "verdict": envelope.verdict.as_ref().map(|v| v.code),
            "conditions": envelope.results["conditions"].as_array().map(|c| c.iter().map(|x| x["passed"].clone()).collect::<Vec<_>>()),
            "agent_ledger_head": head,
            "seal_void": seal_void,
            "withheld_warnings": recorded,
        }),
    )?;
    let mut envelope = envelope;
    if seal_void {
        envelope.warnings.push(Warning {
            code: "seal_void".into(),
            message: "the referee's identity is readable from the agent's side, so the seal protects nothing".into(),
        });
    }
    let exit = exit_for(envelope.verdict.as_ref().map(|v| v.code));
    std::fs::write(
        &result_path,
        serde_json::to_string_pretty(
            &json!({ "verdict": envelope.verdict, "results": envelope.results }),
        )
        .expect("results serialize"),
    )
    .map_err(|e| io_error(&result_path, &e))?;
    Ok(Output { envelope, exit })
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

/// The checks after coverage and the conditions themselves.
#[allow(clippy::too_many_arguments)]
fn judge(
    approved: &Value,
    args: &Args,
    referee: &Referee,
    d: &data::Data,
    run: &Run,
    judgments: &[&Judgment],
    records: &[Record],
    held: &BTreeSet<String>,
    recorded: &mut Vec<String>,
) -> Result<Answer, CliError> {
    let r = &approved["resolved"];
    let refuse = |what: &str| -> Result<Answer, CliError> {
        Err(rejected("does_not_match_approval", what.to_owned()))
    };
    // The candidate must be what was approved (mix-up checks; exit 3).
    let grader = run.grader.as_ref();
    if grader.and_then(|g| g.id.as_deref()) != approved["grader"]["id"].as_str()
        || grader.and_then(|g| g.version.as_deref()) != approved["grader"]["version"].as_str()
    {
        return refuse("the candidate's run.grader differs from the approved grader");
    }
    let levels_match = r["levels"].is_null() || r["levels"].as_u64() == run.levels.map(u64::from);
    if json!(run.output_type) != r["output_type"] || !levels_match {
        return refuse("the candidate's output type or levels differ from the approval");
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
    let items: BTreeMap<&str, &Item> = d
        .items()
        .into_iter()
        .filter(|(id, _)| held.contains(*id))
        .collect();
    if approve::holdout_items_hash(d, held)? != r["holdout_items_hash"].as_str().unwrap_or_default()
    {
        return refuse("the holdout items' attributes changed since approval");
    }
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

    // The sealed labels, in memory only.
    let sealed = std::fs::read(referee.path("sealed").join(format!(
        "{}.age",
        hex(r["holdout"].as_str().unwrap_or_default())
    )))
    .map_err(|e| {
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
    let labels: Vec<&Label> = set
        .latest()
        .into_iter()
        .filter(|l| l.task() == task)
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
    let baseline = approved["compare"]["baseline"].as_str();
    let base_run = baseline.map(|b| d.run(b)).transpose()?;
    if let Some(b) = baseline
        && approve::baseline_hash(d, b)? != r["baseline_snapshot"].as_str().unwrap_or_default()
    {
        return refuse("the baseline's records changed since approval");
    }
    let base_judgments: Vec<&Judgment> = baseline
        .map(|b| {
            d.judgments(b)
                .into_iter()
                .filter(|j| held.contains(&j.item_id))
                .collect()
        })
        .unwrap_or_default();

    // min_items counts paired items, independent of the labels.
    let paired = held
        .iter()
        .filter(|i| base_run.is_none() || base_judgments.iter().any(|j| &j.item_id == *i))
        .count() as u64;
    if paired < approved["min_items"].as_u64().unwrap_or(1) {
        return Ok(stopped(
            VerdictCode::InsufficientData,
            format!("{paired} paired items, fewer than min_items"),
        ));
    }

    let budget = approved["holdout"]["budget"].as_u64().unwrap_or(5) as u32;
    let settings = settings(r, args)?;
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
fn settings(r: &Value, args: &Args) -> Result<Settings, CliError> {
    let rule = |k: &str| -> Option<Rule> { serde_json::from_value(r[k].clone()).ok() };
    let seed_bytes =
        ContentHash::blake3(format!("{}{}", args.approved, args.candidate).as_bytes()).hex();
    Ok(Settings {
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
        proxy: None,
    })
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
            return Ok(None);
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
    Ok(r.metrics
        .iter()
        .find(|m| m.metric == metric)
        .and_then(|m| match (m.value, m.ci) {
            (Some(point), Some((lo, hi))) => Some(Evidence::Value { point, lo, hi }),
            _ => None,
        }))
}
