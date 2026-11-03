//! The validity checks that the `records/1` data model supports (SPEC §7), as pure
//! functions over records. Commands decide which apply: `validate` runs the dataset
//! checks, `compare` adds the arm checks.

use std::collections::{BTreeMap, BTreeSet};

use crate::checks::{Finding, Overrides};
use crate::interval::{Z95, newcombe};
use crate::records::{
    GraderRef, Label, LabelKind, MissingReason, OutputType, Record, Source, Value,
};

/// Checks over one dataset (the records of a project or a snapshot).
pub fn dataset(records: &[Record], overrides: &Overrides) -> Vec<Finding> {
    let mut out = Vec::new();
    out.extend(unexplained_missing(records, overrides));
    out.extend(mixed_output_type(records, overrides));
    out.extend(identical_replicates(records, overrides));
    out.extend(verification_bias_suspected(records, overrides));
    out.extend(model_derived_reference(records, overrides));
    out.extend(secret_in_content(records, overrides));
    out
}

/// A missing value with reason `other` and no detail: the importer could not say
/// why it is missing.
fn unexplained_missing(records: &[Record], overrides: &Overrides) -> Option<Finding> {
    let n = records
        .iter()
        .filter_map(Record::missing)
        .filter(|m| {
            m.reason == MissingReason::Other
                && m.detail.as_deref().is_none_or(|d| d.trim().is_empty())
        })
        .count() as u64;
    (n > 0).then(|| {
        Finding::new(
            "unexplained_missing",
            format!("{n} missing values have reason `other` and no detail"),
            overrides,
        )
        .with_count(n)
    })
}

/// Judgments with a value whose output type differs from their run's or from other
/// judgments of the run, or whose level count differs where both state one. Missing
/// judgments carry no value to type and are not compared.
fn mixed_output_type(records: &[Record], overrides: &Overrides) -> Vec<Finding> {
    let mut shapes: BTreeMap<&str, (BTreeSet<OutputType>, BTreeSet<u32>)> = BTreeMap::new();
    for r in records {
        match r {
            Record::Run(run) => {
                let e = shapes.entry(run.run_id.as_str()).or_default();
                e.0.extend(run.output_type);
                e.1.extend(run.levels);
            }
            Record::Judgment(j) if j.value.is_some() => {
                let e = shapes.entry(j.run_id.as_str()).or_default();
                e.0.insert(j.output_type);
                e.1.extend(j.levels);
            }
            _ => {}
        }
    }
    shapes
        .into_iter()
        .filter(|(_, (types, levels))| types.len() > 1 || levels.len() > 1)
        .map(|(run, (types, levels))| {
            Finding::new(
                "mixed_output_type",
                format!("run {run:?} mixes output types {types:?} or level counts {levels:?}"),
                overrides,
            )
        })
        .collect()
}

/// Items whose generation replicates within one run all have the same output: the
/// outputs may have come from a cache rather than new generations.
fn identical_replicates(records: &[Record], overrides: &Overrides) -> Option<Finding> {
    /// The first output of a (run, item, task) group, how many replicates were seen
    /// and whether all equal the first.
    struct Group<'a> {
        first: Option<&'a str>,
        count: u32,
        same: bool,
    }
    let mut groups: BTreeMap<(&str, &str, &str), Group<'_>> = BTreeMap::new();
    for r in records {
        if let Record::Judgment(j) = r
            && j.grade_replicate() == 0
        {
            let out = j.output_ref.as_deref();
            let g = groups
                .entry((j.run_id.as_str(), j.item_id.as_str(), j.task()))
                .or_insert(Group {
                    first: out,
                    count: 0,
                    same: true,
                });
            g.count += 1;
            g.same &= g.first == out;
        }
    }
    let n = groups
        .values()
        .filter(|g| g.first.is_some() && g.count >= 2 && g.same)
        .count() as u64;
    (n > 0).then(|| {
        Finding::new(
            "identical_replicates",
            format!(
                "{n} items have identical outputs in every generation replicate (cached outputs?)"
            ),
            overrides,
        )
        .with_count(n)
    })
}

/// Shares of labelled items that differ this much between judgment levels suggest
/// that humans checked items selected by the system's output.
const VERIFICATION_SHARE_GAP: f64 = 0.2;
/// Levels with fewer items than this are not compared.
const VERIFICATION_MIN_ITEMS: usize = 10;

/// A judgment value usable as a grouping key (scores are not grouped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Bool(bool),
    Int(i64),
}

/// Human labels on only some judged items, inclusion probabilities missing for some
/// of them, and a labelled share that depends on the system's judgment (metrics spec
/// §9.9). Each item's level is its first generation and grading replicate.
fn verification_bias_suspected(records: &[Record], overrides: &Overrides) -> Vec<Finding> {
    let mut with_prob = BTreeSet::new();
    let mut labelled = BTreeSet::new();
    for r in records {
        match r {
            Record::Item(i) if i.label_inclusion_prob.is_some() => {
                with_prob.insert(i.item_id.as_str());
            }
            Record::Label(l) if l.source == Source::Human => {
                labelled.insert(l.item_id.as_str());
            }
            _ => {}
        }
    }
    if labelled.is_empty() {
        return Vec::new();
    }
    /// (labelled, total) items per level.
    type Shares = BTreeMap<Level, (usize, usize)>;
    let mut levels: BTreeMap<(&str, &str), Shares> = BTreeMap::new();
    let mut unlabelled_without_prob = false;
    for r in records {
        let Record::Judgment(j) = r else { continue };
        if j.gen_replicate != 0 || j.grade_replicate() != 0 {
            continue;
        }
        let level = match j.value {
            Some(Value::Bool(b)) => Level::Bool(b),
            Some(Value::Int(i)) => Level::Int(i),
            _ => continue,
        };
        let is_labelled = labelled.contains(j.item_id.as_str());
        unlabelled_without_prob |= !is_labelled && !with_prob.contains(j.item_id.as_str());
        let e = levels
            .entry((j.run_id.as_str(), j.task()))
            .or_default()
            .entry(level)
            .or_default();
        e.0 += usize::from(is_labelled);
        e.1 += 1;
    }
    if !unlabelled_without_prob {
        return Vec::new();
    }
    levels
        .into_iter()
        .filter_map(|((run, task), by_level)| {
            let shares: Vec<f64> = by_level
                .values()
                .filter(|(_, n)| *n >= VERIFICATION_MIN_ITEMS)
                .map(|(k, n)| *k as f64 / *n as f64)
                .collect();
            let gap = shares.iter().copied().fold(f64::NEG_INFINITY, f64::max)
                - shares.iter().copied().fold(f64::INFINITY, f64::min);
            (shares.len() >= 2 && gap >= VERIFICATION_SHARE_GAP).then(|| {
                Finding::new(
                    "verification_bias_suspected",
                    format!(
                        "run {run:?}, task {task:?}: the share of human-labelled items differs by {gap:.2} between judgment levels and label_inclusion_prob is not given"
                    ),
                    overrides,
                )
            })
        })
        .collect()
}

/// Reference labels that come from a model.
fn model_derived_reference(records: &[Record], overrides: &Overrides) -> Option<Finding> {
    let n = records
        .iter()
        .filter(|r| matches!(r, Record::Label(l) if l.source == Source::Model))
        .count() as u64;
    (n > 0).then(|| {
        Finding::new(
            "model_derived_reference",
            format!("{n} reference labels come from a model, not from people"),
            overrides,
        )
        .with_count(n)
    })
}

/// Secret-shaped strings in the free text of records. Content blobs are scanned by
/// the adapters that import them.
fn secret_in_content(records: &[Record], overrides: &Overrides) -> Option<Finding> {
    let mut n = 0u64;
    let mut kinds = BTreeSet::new();
    for r in records {
        for_each_text(r, &mut |text| {
            if let Some(kind) = secret_shape(text) {
                n += 1;
                kinds.insert(kind);
            }
        });
    }
    (n > 0).then(|| {
        Finding::new(
            "secret_in_content",
            format!(
                "{n} text fields look like secrets ({})",
                kinds.into_iter().collect::<Vec<_>>().join(", ")
            ),
            overrides,
        )
        .with_count(n)
    })
}

fn for_each_text<'a>(r: &'a Record, f: &mut impl FnMut(&'a str)) {
    fn walk<'a>(v: &'a serde_json::Value, f: &mut impl FnMut(&'a str)) {
        match v {
            serde_json::Value::String(s) => f(s),
            serde_json::Value::Array(xs) => xs.iter().for_each(|x| walk(x, f)),
            serde_json::Value::Object(m) => m.values().for_each(|x| walk(x, f)),
            _ => {}
        }
    }
    let mut map = |m: &'a Option<serde_json::Map<String, serde_json::Value>>| {
        m.iter().flat_map(|m| m.values()).for_each(|v| walk(v, f));
    };
    match r {
        Record::Item(i) => {
            map(&i.attrs);
            map(&i.strata);
        }
        Record::Judgment(j) => map(&j.cost),
        Record::Run(run) => {
            map(&run.attrs);
            f(&run.system.version);
        }
        _ => {}
    }
    if let Record::Label(l) = r
        && let Some(reason) = &l.reason
    {
        f(reason);
    }
    if let Some(d) = r.missing().and_then(|m| m.detail.as_deref()) {
        f(d);
    }
    if let Record::Run(run) = r {
        if let Some(h) = &run.harness {
            h.run_ref
                .iter()
                .chain(&h.name)
                .chain(&h.version)
                .for_each(|s| f(s));
        }
        if let Some(g) = &run.grader {
            g.id.iter().chain(&g.version).for_each(|s| f(s));
        }
    }
}

/// Shortest string that can hold a recognized secret; shorter strings are skipped.
const MIN_SECRET_LEN: usize = 20;

/// The kind of secret a string looks like: a private key block, or a token with a
/// well-known prefix followed by a long run of token characters.
pub fn secret_shape(s: &str) -> Option<&'static str> {
    if s.len() < MIN_SECRET_LEN {
        return None;
    }
    if s.contains("-----BEGIN") && s.contains("PRIVATE KEY-----") {
        return Some("private key");
    }
    const PREFIXES: [(&str, usize, &str); 13] = [
        ("AKIA", 16, "AWS access key"),
        ("ghp_", 36, "GitHub token"),
        ("gho_", 36, "GitHub token"),
        ("ghs_", 36, "GitHub token"),
        ("ghu_", 36, "GitHub token"),
        ("ghr_", 36, "GitHub token"),
        ("github_pat_", 22, "GitHub token"),
        ("xoxb-", 20, "Slack token"),
        ("xoxp-", 20, "Slack token"),
        ("xoxa-", 20, "Slack token"),
        ("sk_live_", 20, "Stripe key"),
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

/// Whether two arms were graded alike. A grader recorded for only one arm cannot be
/// shown to match, so it counts as a difference; two arms without one are not
/// compared here.
pub fn mixed_grader(
    a: Option<&GraderRef>,
    b: Option<&GraderRef>,
    overrides: &Overrides,
) -> Option<Finding> {
    let differs = match (a, b) {
        (None, None) => return None,
        (Some(_), None) | (None, Some(_)) => vec!["grader recorded for only one arm"],
        (Some(a), Some(b)) => [
            ("id", a.id != b.id),
            ("version", a.version != b.version),
            ("config_hash", a.config_hash != b.config_hash),
        ]
        .into_iter()
        .filter_map(|(name, d)| d.then_some(name))
        .collect(),
    };
    (!differs.is_empty()).then(|| {
        Finding::new(
            "mixed_grader",
            format!(
                "the compared arms were graded differently ({})",
                differs.join(", ")
            ),
            overrides,
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
                overrides,
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
                overrides,
            )
            .with_count(n),
        );
    }
    (p, findings)
}

/// System-side missing counts out of judged items in two arms; warns when the 95%
/// interval of the difference in missing rates excludes 0.
pub fn missing_rate_differs(
    a: (u64, u64),
    b: (u64, u64),
    overrides: &Overrides,
) -> Option<Finding> {
    let (lo, hi) = newcombe(a.0, a.1, b.0, b.1, Z95);
    (lo > 0.0 || hi < 0.0).then(|| {
        Finding::new(
            "missing_rate_differs",
            format!(
                "system-side missing rates differ: {}/{} vs {}/{} (95% interval of the difference {lo:.3} to {hi:.3})",
                a.0, a.1, b.0, b.1
            ),
            overrides,
        )
    })
}

/// Warn when the run-to-run standard deviation of a metric exceeds the point estimate
/// of the difference being reported (metrics spec §6.3).
pub fn run_variance_exceeds_delta(sd: f64, delta: f64, overrides: &Overrides) -> Option<Finding> {
    (sd > delta.abs()).then(|| {
        Finding::new(
            "run_variance_exceeds_delta",
            format!("the run-to-run standard deviation {sd:.4} exceeds the difference {delta:.4}"),
            overrides,
        )
    })
}
