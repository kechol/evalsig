//! The validity checks that the `records/1` data model supports (SPEC §7), as pure
//! functions over records. Commands decide which apply: `validate` runs the dataset
//! checks, `compare` adds the arm checks.

use std::collections::{BTreeMap, BTreeSet};

use crate::checks::{Finding, Overrides};
use crate::records::{
    GraderRef, Label, LabelKind, MissingReason, OutputType, Record, Source, Value,
};

/// Checks over one dataset (the records of a project or a snapshot).
pub fn dataset(records: &[Record], overrides: &Overrides) -> Vec<Finding> {
    let mut out = Vec::new();
    out.extend(unexplained_missing(records, overrides));
    out.extend(mixed_output_type(records));
    out.extend(identical_replicates(records));
    out.extend(verification_bias_suspected(records));
    out.extend(model_derived_reference(records));
    out.extend(secret_in_content(records, overrides));
    out
}

/// A missing value with reason `other` and no detail: the importer could not say
/// why it is missing.
fn unexplained_missing(records: &[Record], overrides: &Overrides) -> Option<Finding> {
    let n = records
        .iter()
        .filter_map(|r| match r {
            Record::Judgment(j) => j.missing.as_ref(),
            Record::Label(l) => l.missing.as_ref(),
            Record::Preference(p) => p.missing.as_ref(),
            _ => None,
        })
        .filter(|m| {
            m.reason == MissingReason::Other && m.detail.as_deref().is_none_or(str::is_empty)
        })
        .count() as u64;
    (n > 0).then(|| {
        Finding::new(
            "unexplained_missing",
            format!("{n} missing values have reason `other` and no detail"),
            overrides.lifts("unexplained_missing"),
        )
        .with_count(n)
    })
}

/// Judgments whose output type or level count differs from their run's, or from the
/// other judgments of the same run.
fn mixed_output_type(records: &[Record]) -> Vec<Finding> {
    let declared: BTreeMap<&str, (Option<OutputType>, Option<u32>)> = records
        .iter()
        .filter_map(|r| match r {
            Record::Run(run) => Some((run.run_id.as_str(), (run.output_type, run.levels))),
            _ => None,
        })
        .collect();
    let mut seen: BTreeMap<&str, BTreeSet<(u8, Option<u32>)>> = BTreeMap::new();
    for r in records {
        if let Record::Judgment(j) = r {
            seen.entry(j.run_id.as_str())
                .or_default()
                .insert((j.output_type as u8, j.levels));
        }
    }
    let mut out = Vec::new();
    for (run, shapes) in seen {
        let mut all = shapes.clone();
        if let Some((Some(t), levels)) = declared.get(run) {
            all.insert((*t as u8, *levels));
        }
        if all.len() > 1 {
            out.push(Finding::new(
                "mixed_output_type",
                format!(
                    "run {run:?} mixes {} output types or level counts",
                    all.len()
                ),
                false,
            ));
        }
    }
    out
}

/// Items whose generation replicates within one run all have the same output: the
/// outputs may have come from a cache rather than new generations.
fn identical_replicates(records: &[Record]) -> Option<Finding> {
    let mut outputs: BTreeMap<(&str, &str, &str), Vec<Option<&str>>> = BTreeMap::new();
    for r in records {
        if let Record::Judgment(j) = r
            && j.grade_replicate() == 0
        {
            outputs
                .entry((j.run_id.as_str(), j.item_id.as_str(), j.task()))
                .or_default()
                .push(j.output_ref.as_deref());
        }
    }
    let n = outputs
        .values()
        .filter(|o| o.len() >= 2 && o[0].is_some() && o.iter().all(|x| *x == o[0]))
        .count() as u64;
    (n > 0).then(|| {
        Finding::new(
            "identical_replicates",
            format!(
                "{n} items have identical outputs in every generation replicate (cached outputs?)"
            ),
            false,
        )
        .with_count(n)
    })
}

/// Shares of labelled items that differ this much between judgment levels suggest
/// that humans checked items selected by the system's output.
const VERIFICATION_SHARE_GAP: f64 = 0.2;
/// Levels with fewer items than this are not compared.
const VERIFICATION_MIN_ITEMS: usize = 10;

/// Human labels on only some items, no inclusion probability, and a labelled share
/// that depends on the system's judgment (metrics spec §9.9).
fn verification_bias_suspected(records: &[Record]) -> Vec<Finding> {
    let mut with_prob = BTreeSet::new();
    let mut items = BTreeSet::new();
    for r in records {
        if let Record::Item(i) = r {
            items.insert(i.item_id.as_str());
            if i.label_inclusion_prob.is_some() {
                with_prob.insert(i.item_id.as_str());
            }
        }
    }
    let labelled: BTreeSet<&str> = records
        .iter()
        .filter_map(|r| match r {
            Record::Label(l) if l.source == Source::Human => Some(l.item_id.as_str()),
            _ => None,
        })
        .collect();
    if labelled.is_empty() || labelled.len() == items.len() || !with_prob.is_empty() {
        return Vec::new();
    }
    // The first judgment of each item per run, grouped by its value.
    let mut levels: BTreeMap<&str, BTreeMap<String, (usize, usize)>> = BTreeMap::new();
    let mut counted = BTreeSet::new();
    for r in records {
        if let Record::Judgment(j) = r
            && let Some(v) = j.value
            && !matches!(v, Value::Float(_))
            && counted.insert((j.run_id.as_str(), j.item_id.as_str()))
        {
            let e = levels
                .entry(j.run_id.as_str())
                .or_default()
                .entry(format!("{v:?}"))
                .or_default();
            e.0 += 1;
            e.1 += usize::from(labelled.contains(j.item_id.as_str()));
        }
    }
    levels
        .into_iter()
        .filter_map(|(run, by_level)| {
            let shares: Vec<f64> = by_level
                .values()
                .filter(|(n, _)| *n >= VERIFICATION_MIN_ITEMS)
                .map(|(n, k)| *k as f64 / *n as f64)
                .collect();
            let gap = shares.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                - shares.iter().copied().fold(f64::INFINITY, f64::min);
            (shares.len() >= 2 && gap >= VERIFICATION_SHARE_GAP).then(|| {
                Finding::new(
                    "verification_bias_suspected",
                    format!(
                        "run {run:?}: the share of human-labelled items differs by {gap:.2} between judgment levels and no label_inclusion_prob is given"
                    ),
                    false,
                )
            })
        })
        .collect()
}

/// Reference labels that come from a model.
fn model_derived_reference(records: &[Record]) -> Option<Finding> {
    let n = records
        .iter()
        .filter(|r| matches!(r, Record::Label(l) if l.source == Source::Model))
        .count() as u64;
    (n > 0).then(|| {
        Finding::new(
            "model_derived_reference",
            format!("{n} reference labels come from a model, not from people"),
            false,
        )
        .with_count(n)
    })
}

/// Secret-shaped strings in the free text of records.
fn secret_in_content(records: &[Record], overrides: &Overrides) -> Option<Finding> {
    let mut n = 0u64;
    let mut kinds = BTreeSet::new();
    for r in records {
        for text in free_text(r) {
            if let Some(kind) = secret_shape(&text) {
                n += 1;
                kinds.insert(kind);
            }
        }
    }
    (n > 0).then(|| {
        let mut f = Finding::new(
            "secret_in_content",
            format!(
                "{n} text fields look like secrets ({})",
                kinds.into_iter().collect::<Vec<_>>().join(", ")
            ),
            false,
        )
        .with_count(n);
        if overrides.fail_on_secret {
            f.action = crate::checks::Action::Stop;
        } else {
            f.hint = Some("pass --fail-on-secret to stop instead".into());
        }
        f
    })
}

fn free_text(r: &Record) -> Vec<String> {
    let mut out = Vec::new();
    let json = |m: &Option<serde_json::Map<String, serde_json::Value>>, out: &mut Vec<String>| {
        if let Some(m) = m {
            m.values().for_each(|v| collect_strings(v, out));
        }
    };
    match r {
        Record::Item(i) => {
            json(&i.attrs, &mut out);
            json(&i.strata, &mut out);
        }
        Record::Label(l) => {
            out.extend(l.reason.clone());
            out.extend(l.missing.as_ref().and_then(|m| m.detail.clone()));
        }
        Record::Judgment(j) => {
            out.extend(j.missing.as_ref().and_then(|m| m.detail.clone()));
            json(&j.cost, &mut out);
        }
        Record::Preference(p) => out.extend(p.missing.as_ref().and_then(|m| m.detail.clone())),
        Record::Run(run) => json(&run.attrs, &mut out),
        Record::Content(_) => {}
    }
    out
}

fn collect_strings(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(xs) => xs.iter().for_each(|x| collect_strings(x, out)),
        serde_json::Value::Object(m) => m.values().for_each(|x| collect_strings(x, out)),
        _ => {}
    }
}

/// The kind of secret a string looks like: a private key block, or a token with a
/// well-known prefix followed by a long run of token characters.
pub fn secret_shape(s: &str) -> Option<&'static str> {
    if s.contains("-----BEGIN") && s.contains("PRIVATE KEY-----") {
        return Some("private key");
    }
    const PREFIXES: [(&str, usize, &str); 7] = [
        ("AKIA", 16, "AWS access key"),
        ("ghp_", 36, "GitHub token"),
        ("gho_", 36, "GitHub token"),
        ("github_pat_", 22, "GitHub token"),
        ("xoxb-", 20, "Slack token"),
        ("sk-", 20, "API key"),
        ("AIza", 35, "Google API key"),
    ];
    for (prefix, min, kind) in PREFIXES {
        for (i, _) in s.match_indices(prefix) {
            let boundary = s[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_ascii_alphanumeric());
            let run = s[i + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .count();
            if boundary && run >= min {
                return Some(kind);
            }
        }
    }
    None
}

/// Labels that may enter agreement metrics and the alternative annotator test:
/// adjudications are decided after seeing other labels, so they are never
/// independent (SPEC §7).
pub fn agreement_labels<'a>(labels: impl IntoIterator<Item = &'a Label>) -> Vec<&'a Label> {
    labels
        .into_iter()
        .filter(|l| l.kind() != LabelKind::Adjudication)
        .collect()
}

/// Whether two runs were graded by the same grader (id, version and configuration).
pub fn mixed_grader(
    a: Option<&GraderRef>,
    b: Option<&GraderRef>,
    overrides: &Overrides,
) -> Option<Finding> {
    let key =
        |g: Option<&GraderRef>| g.map(|g| (g.id.clone(), g.version.clone(), g.config_hash.clone()));
    (key(a) != key(b)).then(|| {
        Finding::new(
            "mixed_grader",
            "the compared arms were graded by different grader versions",
            overrides.lifts("mixed_grader"),
        )
    })
}

/// How two arms' items pair up.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Pairing<'a> {
    /// Items compared.
    pub paired: Vec<&'a str>,
    /// Items in only one arm.
    pub n_unpaired: u64,
    /// Items present in both arms whose input differs; excluded unless allowed.
    pub input_changed: Vec<&'a str>,
}

/// Pair two arms by item ID. Each map gives an arm's item IDs and their `input_ref`.
pub fn pair<'a>(
    a: &BTreeMap<&'a str, Option<&'a str>>,
    b: &BTreeMap<&'a str, Option<&'a str>>,
    overrides: &Overrides,
) -> (Pairing<'a>, Vec<Finding>) {
    let mut p = Pairing::default();
    for (item, input) in a {
        match b.get(item) {
            None => p.n_unpaired += 1,
            Some(other) if input.is_some() && other.is_some() && input != other => {
                p.input_changed.push(item);
                if overrides.allow_input_change {
                    p.paired.push(item);
                }
            }
            Some(_) => p.paired.push(item),
        }
    }
    p.n_unpaired += b.keys().filter(|k| !a.contains_key(*k)).count() as u64;
    let mut findings = Vec::new();
    if p.n_unpaired > 0 {
        findings.push(
            Finding::new(
                "unpaired_items",
                format!(
                    "{} items are in only one arm; compared on the {} common items",
                    p.n_unpaired,
                    p.paired.len()
                ),
                false,
            )
            .with_count(p.n_unpaired),
        );
    }
    if !p.input_changed.is_empty() {
        let n = p.input_changed.len() as u64;
        findings.push(
            Finding::new(
                "input_changed",
                format!("{n} items have different inputs in the two arms"),
                overrides.lifts("input_changed"),
            )
            .with_count(n),
        );
    }
    (p, findings)
}

const Z95: f64 = 1.959_963_984_540_054;

fn wilson(k: u64, n: u64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (k, n) = (k as f64, n as f64);
    let p = k / n;
    let z2 = Z95 * Z95;
    let d = 1.0 + z2 / n;
    let c = (p + z2 / (2.0 * n)) / d;
    let h = Z95 * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt() / d;
    ((c - h).max(0.0), (c + h).min(1.0))
}

/// Newcombe's hybrid score interval (method 10) for p_a − p_b.
fn newcombe(ka: u64, na: u64, kb: u64, nb: u64) -> (f64, f64) {
    let (pa, pb) = (ka as f64 / na.max(1) as f64, kb as f64 / nb.max(1) as f64);
    let (la, ua) = wilson(ka, na);
    let (lb, ub) = wilson(kb, nb);
    let d = pa - pb;
    (
        d - ((pa - la).powi(2) + (ub - pb).powi(2)).sqrt(),
        d + ((ua - pa).powi(2) + (pb - lb).powi(2)).sqrt(),
    )
}

/// System-side missing counts out of judged items in two arms; warns when the 95%
/// interval of the difference in missing rates excludes 0.
pub fn missing_rate_differs(a: (u64, u64), b: (u64, u64)) -> Option<Finding> {
    let (lo, hi) = newcombe(a.0, a.1, b.0, b.1);
    (lo > 0.0 || hi < 0.0).then(|| {
        Finding::new(
            "missing_rate_differs",
            format!(
                "system-side missing rates differ: {}/{} vs {}/{} (95% interval of the difference {lo:.3} to {hi:.3})",
                a.0, a.1, b.0, b.1
            ),
            false,
        )
    })
}

/// Warn when the run-to-run standard deviation of a metric exceeds the point estimate
/// of the difference being reported (metrics spec §6.3).
pub fn run_variance_exceeds_delta(sd: f64, delta: f64) -> Option<Finding> {
    (sd > delta.abs()).then(|| {
        Finding::new(
            "run_variance_exceeds_delta",
            format!("the run-to-run standard deviation {sd:.4} exceeds the difference {delta:.4}"),
            false,
        )
    })
}
