//! The dashboard's six screens (SPEC §11, D-019). They render the same results the CLI
//! prints, computed without writing anything: the report and comparison are computed
//! as `report` and `compare` compute them, never recorded or saved. The data in force
//! are read once and kept while the project ledger is unchanged (D-028).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use evalsig_core::config::{Config, Language};
use evalsig_core::records::{Record, Run};
use serde_json::{Value, json};

use evalsig_project::data::Data;
use evalsig_project::envelope::Output;
use evalsig_project::project::{Project, io_error};
use evalsig_project::{Error, agree, compare, data, explain, list, report, trend};

use crate::html::{self, esc};
use crate::i18n::{self, T};
use crate::server::{self, Page};
use crate::svg;

/// The screens, by route and title.
pub const SCREENS: [(&str, T); 6] = [
    ("", T::Overview),
    ("compare", T::Comparison),
    ("trend", T::Trend),
    ("reproducibility", T::Reproducibility),
    ("reference", T::ReferenceQuality),
    ("discrimination", T::Discrimination),
];

/// The screens about one run, as the links offered for it.
const RUN_SCREENS: [(&str, T); 4] = [
    ("reproducibility", T::Reproducibility),
    ("reference", T::ReferenceQuality),
    ("discrimination", T::Discrimination),
    ("trend", T::Trend),
];

/// The runs a screen shows, which the navigation carries to the other screens so that
/// moving between them keeps the choice.
enum Focus {
    None,
    Run(String),
    Pair(String, String),
}

/// A query value with every byte outside the unreserved set percent-encoded.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn run_href(route: &str, run: &str) -> String {
    format!("{route}?run={}", encode(run))
}

fn compare_href(baseline: &str, candidate: &str) -> String {
    format!(
        "compare?baseline={}&candidate={}",
        encode(baseline),
        encode(candidate)
    )
}

/// The navigation, each link carrying the runs of `focus`: a run screen opens on the
/// focused run (the candidate of a pair), the comparison on the pair or with the run
/// as its candidate.
fn nav(focus: &Focus, lang: Language) -> Vec<(String, &'static str)> {
    SCREENS
        .iter()
        .map(|(route, label)| {
            let href = match (*route, focus) {
                ("", _) => "./".to_owned(),
                ("compare", Focus::Pair(b, c)) => compare_href(b, c),
                ("compare", Focus::Run(r)) => format!("compare?candidate={}", encode(r)),
                (route, Focus::Run(r) | Focus::Pair(_, r)) => run_href(route, r),
                (route, Focus::None) => route.to_owned(),
            };
            (href, label.text(lang))
        })
        .collect()
}

fn query(path: &str) -> (&str, Vec<(String, String)>) {
    let (route, q) = path.split_once('?').unwrap_or((path, ""));
    let pairs = q
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (decode(k), decode(v)))
        .collect();
    (route, pairs)
}

/// `%XX` and `+` in a query value.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            // Hex digits are ASCII, so they are read as bytes, never as a slice of the
            // string that could split a character.
            b'%' if i + 2 < bytes.len() => match std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                Some(b) => {
                    out.push(b);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn get<'a>(q: &'a [(String, String)], k: &str) -> Option<&'a str> {
    q.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
}

/// The run a query names, if it is one of the project's runs; otherwise `default`.
/// Only known run IDs are used, so a query cannot name anything else.
fn chosen<'a>(q: &[(String, String)], k: &str, runs: &'a [String], default: &'a str) -> &'a str {
    get(q, k)
        .and_then(|v| runs.iter().find(|r| r.as_str() == v))
        .map_or(default, String::as_str)
}

fn cell(v: &Value) -> String {
    match v {
        Value::Null => "–".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// An interval `[lo, hi]` written as the cells write numbers.
fn interval(ci: &Value) -> String {
    let end = |v: &Value| v.as_f64().map_or_else(|| cell(v), html::shown);
    format!("{} – {}", end(&ci[0]), end(&ci[1]))
}

/// The run records, in import order.
fn run_records(d: &Data) -> Vec<&Run> {
    d.records
        .iter()
        .filter_map(|r| match r {
            Record::Run(run) => Some(run),
            _ => None,
        })
        .collect()
}

/// The runs, in import order.
fn run_ids(d: &Data) -> Vec<String> {
    run_records(d)
        .into_iter()
        .map(|r| r.run_id.clone())
        .collect()
}

/// Whether `compare` can pair two runs: the same output type and value semantics.
fn comparable(a: &Run, b: &Run) -> bool {
    a.output_type == b.output_type && a.value_semantics == b.value_semantics
}

/// The comparison shown without a choice: the latest run that has an earlier comparable
/// run, against the latest such run; the first two runs when no run has one.
fn default_pair(runs: &[&Run]) -> (usize, usize) {
    (1..runs.len())
        .rev()
        .find_map(|c| {
            (0..c)
                .rev()
                .find(|&b| comparable(runs[b], runs[c]))
                .map(|b| (b, c))
        })
        .unwrap_or((0, 1))
}

/// The baseline for a chosen candidate: the latest comparable run before it, else the
/// first one after it, else its neighbour.
fn baseline_for(runs: &[&Run], c: usize) -> usize {
    (0..c)
        .rev()
        .chain(c + 1..runs.len())
        .find(|&b| comparable(runs[b], runs[c]))
        .unwrap_or(if c == 0 { 1 } else { c - 1 })
}

/// The toolbar choosing the runs a screen is drawn for, as a plain GET form, with
/// `extra` (HTML) after its button.
fn picker(
    route: &str,
    runs: &[String],
    fields: &[(&str, &str)],
    extra: &str,
    lang: Language,
) -> String {
    let mut s = format!(
        "<form class=\"toolbar\" method=\"get\" action=\"{}\">",
        esc(route)
    );
    for (name, chosen) in fields {
        let label = match *name {
            "baseline" => T::Baseline,
            "candidate" => T::Candidate,
            _ => T::RunField,
        };
        s.push_str(&format!(
            "<label>{}<select name=\"{}\">",
            esc(label.text(lang)),
            esc(name)
        ));
        for r in runs {
            let sel = if r == chosen { " selected" } else { "" };
            s.push_str(&format!("<option{sel}>{}</option>", esc(r)));
        }
        s.push_str("</select></label>");
    }
    s.push_str(&format!(
        "<button>{}</button>{extra}</form>",
        esc(T::Show.text(lang))
    ));
    s
}

/// The question each screen answers, under its title.
fn lead(title: T) -> Option<T> {
    match title {
        T::Overview => Some(T::LeadOverview),
        T::Comparison => Some(T::LeadComparison),
        T::Trend => Some(T::LeadTrend),
        T::Reproducibility => Some(T::LeadReproducibility),
        T::ReferenceQuality => Some(T::LeadReferenceQuality),
        T::Discrimination => Some(T::LeadDiscrimination),
        _ => None,
    }
}

/// "How to read" for the metrics on a screen, from the embedded glossary.
fn how_to_read(names: &[&str], lang: Language) -> String {
    let mut items = String::new();
    for n in names {
        if let Ok(o) = explain::run(n)
            && let Some(m) = o.envelope.results["matches"]
                .as_array()
                .and_then(|m| m.first())
        {
            items.push_str(&format!(
                "<dt>{}</dt><dd>{}</dd>",
                esc(n),
                esc(m["meaning"].as_str().unwrap_or_default())
            ));
        }
    }
    if items.is_empty() {
        return String::new();
    }
    format!(
        "<details><summary>{}</summary><dl>{items}</dl></details>",
        esc(T::HowToRead.text(lang))
    )
}

fn warnings_of(v: &[evalsig_project::envelope::Warning]) -> Vec<String> {
    v.iter()
        .map(|w| format!("{}: {}", w.code, w.message))
        .collect()
}

fn metric_names(metrics: &Value) -> Vec<String> {
    metrics
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["metric"].as_str().map(str::to_owned))
        .collect()
}

struct Screen {
    title: T,
    focus: Focus,
    warnings: Vec<String>,
    body: String,
}

impl Screen {
    /// A screen with nothing to show but a sentence.
    fn note(title: T, text: &str) -> Self {
        Screen {
            title,
            focus: Focus::None,
            warnings: vec![],
            body: format!("<p class=\"empty\">{}</p>", esc(text)),
        }
    }
}

/// A result that could not be computed, as a warning callout with its hint.
fn error_note(e: &Error) -> String {
    match &e.hint {
        Some(h) => format!(
            "<div class=\"callout warning\"><strong>{}</strong>{}</div>",
            esc(&e.message),
            esc(h)
        ),
        None => format!("<div class=\"callout warning\">{}</div>", esc(&e.message)),
    }
}

/// A panel holding a sentence of what is missing.
fn empty_panel(title: &str, text: &str) -> String {
    html::panel(title, "", &format!("<p class=\"empty\">{}</p>", esc(text)))
}

/// A run's report as `report` computes it, with the defaults of the command line.
fn report_of(run: &str) -> report::Request {
    report::Request {
        run: run.to_owned(),
        ..report::Request::default()
    }
}

/// The data in force and the configuration a request is answered from.
struct View<'a> {
    d: &'a Data,
    config: &'a Config,
    lang: Language,
}

impl View<'_> {
    fn report(&self, request: &report::Request) -> Result<report::Computed, Error> {
        report::compute(self.d, self.config, request)
    }
}

/// Where to go from the run at `i`: its comparison with the run imported before it as
/// the main action, and the screens about one run beside it.
fn run_actions(ids: &[String], i: usize, lang: Language) -> String {
    let run = &ids[i];
    let mut buttons = Vec::new();
    if let Some(before) = i.checked_sub(1).map(|j| &ids[j]) {
        buttons.push(html::button(
            &compare_href(before, run),
            &T::CompareWith.fill(lang, before),
            false,
        ));
    }
    for (route, label) in RUN_SCREENS {
        buttons.push(html::button(&run_href(route, run), label.text(lang), true));
    }
    html::actions(&buttons)
}

fn overview(v: &View) -> Result<Screen, Error> {
    let runs = list::runs_of(v.d).envelope;
    let rows: Vec<Vec<String>> = runs.results["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            [
                "run_id",
                "system",
                "version",
                "output_type",
                "items",
                "judgments",
                "missing",
            ]
            .iter()
            .map(|k| cell(&r[*k]))
            .collect()
        })
        .collect();
    let l = v.lang;
    let mut body = html::panel(
        T::Runs.text(l),
        "",
        &html::table(
            &[
                T::Run.text(l),
                T::System.text(l),
                T::Version.text(l),
                T::OutputType.text(l),
                T::Items.text(l),
                T::Judgments.text(l),
                T::Missing.text(l),
            ],
            &rows,
        ),
    );
    let mut warnings = warnings_of(&runs.warnings);
    let ids: Vec<String> = runs.results["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["run_id"].as_str().map(str::to_owned))
        .collect();
    // Every run's report first, so that all the plots share one axis.
    let reports: Vec<Result<report::Computed, Error>> =
        ids.iter().map(|id| v.report(&report_of(id))).collect();
    let all: Vec<svg::Row> = reports
        .iter()
        .flatten()
        .flat_map(|c| svg::metric_rows(&c.results["metrics"], "value"))
        .collect();
    let shared = svg::range(&all, false);
    let mut names = Vec::new();
    for (i, (id, c)) in ids.iter().zip(&reports).enumerate() {
        // One run that cannot be reported shows its error; the others still render.
        let plot = match c {
            Err(e) => error_note(e),
            Ok(c) => {
                warnings.extend(warnings_of(&c.warnings));
                names.extend(metric_names(&c.results["metrics"]));
                shared.map_or_else(String::new, |range| {
                    svg::intervals_in(
                        &svg::metric_rows(&c.results["metrics"], "value"),
                        false,
                        range,
                    )
                })
            }
        };
        body.push_str(&html::panel(id, &run_actions(&ids, i, l), &plot));
    }
    names.sort();
    names.dedup();
    body.push_str(&how_to_read(
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
        v.lang,
    ));
    Ok(Screen {
        title: T::Overview,
        focus: Focus::None,
        warnings,
        body,
    })
}

/// How an item's judgment moved from the baseline to the candidate: toward its reference
/// share, away from it, or only changed when there is no reference to tell.
fn movement(item: &Value) -> Option<T> {
    let (b, c) = (&item["baseline"], &item["candidate"]);
    if b == c {
        return None;
    }
    match (b.as_f64(), c.as_f64(), item["reference"].as_f64()) {
        (Some(b), Some(c), Some(r)) if (c - r).abs() < (b - r).abs() => Some(T::Closer),
        (Some(b), Some(c), Some(r)) if (c - r).abs() > (b - r).abs() => Some(T::Further),
        _ => Some(T::Changed),
    }
}

fn comparison(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let records = run_records(v.d);
    let runs = run_ids(v.d);
    if runs.len() < 2 {
        return Ok(Screen::note(T::Comparison, T::NeedsTwoRuns.text(v.lang)));
    }
    let (b0, c0) = default_pair(&records);
    let cand = chosen(q, "candidate", &runs, &runs[c0]).to_owned();
    let at = runs.iter().position(|r| *r == cand).unwrap_or(c0);
    let before = if at == c0 {
        b0
    } else {
        baseline_for(&records, at)
    };
    let base = chosen(q, "baseline", &runs, &runs[before]).to_owned();
    let mut body = picker(
        "compare",
        &runs,
        &[("baseline", &base), ("candidate", &cand)],
        &html::button(&compare_href(&cand, &base), T::Swap.text(v.lang), true),
        v.lang,
    );
    let request = compare::Request {
        baseline: base.clone(),
        candidates: vec![cand.clone()],
        items: true,
        ..compare::Request::default()
    };
    // Two runs that cannot be compared leave the choice on the page to change.
    let c = match compare::compute(v.d, v.config, &request, None) {
        Ok(c) => c,
        Err(e) => {
            body.push_str(&error_note(&e));
            return Ok(Screen {
                title: T::Comparison,
                focus: Focus::Pair(base, cand),
                warnings: vec![],
                body,
            });
        }
    };
    let r = &c.results;
    if let Some(v) = &c.verdict {
        body.push_str(&html::verdict(v.code.as_str(), &v.text));
    }
    body.push_str(&html::panel(
        T::Differences.text(v.lang),
        "",
        &svg::intervals(&svg::metric_rows(&r["metrics"], "delta"), true),
    ));
    // Both runs in ROC space: a move along one curve is a changed criterion, a move to a
    // higher curve a better separation.
    if let (Ok(rb), Ok(rc)) = (v.report(&report_of(&base)), v.report(&report_of(&cand))) {
        let space = svg::roc_space(
            &[
                svg::RocRun {
                    label: T::BaselineRun.fill(v.lang, &base),
                    sdt: &rb.results["sdt"],
                },
                svg::RocRun {
                    label: T::CandidateRun.fill(v.lang, &cand),
                    sdt: &rc.results["sdt"],
                },
            ],
            v.lang,
        );
        if !space.is_empty() {
            body.push_str(&html::panel(T::RocSpace.text(v.lang), "", &space));
        }
    }
    if r["transition"].is_object() {
        body.push_str(&html::panel(
            T::Transitions.text(v.lang),
            "",
            &svg::transition(&r["transition"], v.lang),
        ));
    }
    // The items whose judgment changed, those moving away from the reference first; the
    // unchanged ones folded.
    let items = r["items"].as_array().map_or(&[][..], Vec::as_slice);
    let mut changed: Vec<(&Value, T)> = items
        .iter()
        .filter_map(|i| movement(i).map(|m| (i, m)))
        .collect();
    changed.sort_by_key(|(_, m)| match m {
        T::Further => 0,
        T::Closer => 1,
        _ => 2,
    });
    let row = |i: &Value| {
        vec![
            cell(&i["item_id"]),
            cell(&i["reference"]),
            cell(&i["baseline"]),
            cell(&i["candidate"]),
        ]
    };
    let head = [
        T::Item.text(v.lang),
        T::Reference.text(v.lang),
        T::Baseline.text(v.lang),
        T::Candidate.text(v.lang),
    ];
    if !items.is_empty() {
        let mut part = if changed.is_empty() {
            format!(
                "<p class=\"empty\">{}</p>",
                esc(T::NoItemChanged.text(v.lang))
            )
        } else {
            let rows: Vec<Vec<String>> = changed
                .iter()
                .map(|(i, m)| {
                    let tone = match m {
                        T::Further => html::Tone::Negative,
                        T::Closer => html::Tone::Positive,
                        _ => html::Tone::Neutral,
                    };
                    let mut r: Vec<String> = row(i).iter().map(|c| html::td(c)).collect();
                    r.push(format!("<td>{}</td>", html::badge(tone, m.text(v.lang))));
                    r
                })
                .collect();
            html::table_html(
                &[head[0], head[1], head[2], head[3], T::Change.text(v.lang)],
                &rows,
            )
        };
        let same: Vec<Vec<String>> = items
            .iter()
            .filter(|i| movement(i).is_none())
            .map(row)
            .collect();
        if !same.is_empty() {
            part.push_str(&format!(
                "<details><summary>{}</summary>{}</details>",
                esc(&T::ItemsUnchanged.fill(v.lang, &same.len().to_string())),
                html::table(&head, &same)
            ));
        }
        body.push_str(&html::panel(
            &T::ItemsChanged.fill(v.lang, &changed.len().to_string()),
            "",
            &part,
        ));
    }
    let names = metric_names(&r["metrics"]);
    body.push_str(&how_to_read(
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
        v.lang,
    ));
    Ok(Screen {
        title: T::Comparison,
        focus: Focus::Pair(base, cand),
        warnings: warnings_of(&c.warnings),
        body,
    })
}

fn trend_screen(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen::note(T::Trend, T::NoRuns.text(v.lang)));
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let mut body = picker("trend", &runs, &[("run", &run)], "", v.lang);
    let request = trend::Request {
        run: run.clone(),
        ..trend::Request::default()
    };
    match trend::compute(v.d, v.config, &request) {
        Ok(out) => {
            let r = &out.envelope.results;
            if let Some(v) = &out.envelope.verdict {
                body.push_str(&html::verdict(v.code.as_str(), &v.text));
            }
            let primary = r["primary_metric"].as_str().unwrap_or_default();
            body.push_str(&html::panel(
                &T::ByPeriod.fill(v.lang, primary),
                "",
                &svg::trend(&r["periods"], primary),
            ));
            body.push_str(&how_to_read(&[primary, "drifted"], v.lang));
            Ok(Screen {
                title: T::Trend,
                focus: Focus::Run(run),
                warnings: warnings_of(&out.envelope.warnings),
                body,
            })
        }
        Err(e) => {
            body.push_str(&error_note(&e));
            Ok(Screen {
                title: T::Trend,
                focus: Focus::Run(run),
                warnings: vec![],
                body,
            })
        }
    }
}

fn reproducibility(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen::note(T::Reproducibility, T::NoRuns.text(v.lang)));
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let c = v.report(&report::Request {
        k_curve: true,
        ..report_of(&run)
    })?;
    let mut body = picker("reproducibility", &runs, &[("run", &run)], "", v.lang);
    // One row per source of variation, its measures as columns in the order they appear.
    let rep = &c.results["reproducibility"];
    let sources: Vec<(&String, &Value)> = rep.as_object().into_iter().flatten().collect();
    let mut keys: Vec<&str> = Vec::new();
    for (_, v) in &sources {
        for k in v.as_object().into_iter().flatten().map(|(k, _)| k.as_str()) {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
    }
    let rows: Vec<Vec<String>> = sources
        .iter()
        .map(|(source, v)| {
            let mut row = vec![(*source).clone()];
            row.extend(keys.iter().map(|k| cell(&v[*k])));
            row
        })
        .collect();
    // The measures are named by their keys; in English with spaces for underscores.
    let head: Vec<String> = std::iter::once(T::Source.text(v.lang).to_owned())
        .chain(keys.iter().map(|k| match v.lang {
            Language::En => k.replace('_', " "),
            Language::Ja => (*k).to_owned(),
        }))
        .collect();
    if rows.is_empty() {
        body.push_str(&empty_panel(
            T::ReplicateAgreement.text(v.lang),
            T::NoReplicates.text(v.lang),
        ));
    } else {
        body.push_str(&html::panel(
            T::ReplicateAgreement.text(v.lang),
            "",
            &html::table(&head.iter().map(String::as_str).collect::<Vec<_>>(), &rows),
        ));
    }
    // The k-curve: each rule and k with the primary metric's value and interval, and
    // the cost per item when the configuration prices a generation.
    let curve = &c.results["k_curve"];
    if let Some(points) = curve["points"].as_array() {
        let rows: Vec<Vec<String>> = points
            .iter()
            .map(|p| {
                let m = &p["metrics"][0];
                vec![
                    cell(&p["k"]),
                    cell(&p["rule"]),
                    cell(&m["metric"]),
                    cell(&m["value"]),
                    interval(&m["ci"]),
                    cell(&p["cost_per_item"]),
                ]
            })
            .collect();
        let table = html::table(
            &[
                T::K.text(v.lang),
                T::Rule.text(v.lang),
                T::Metric.text(v.lang),
                T::Value.text(v.lang),
                T::Interval.text(v.lang),
                T::CostPerItem.text(v.lang),
            ],
            &rows,
        );
        // One line per rule over k, drawn when more than one k was judged.
        let mut by_rule: Vec<svg::Line> = Vec::new();
        for p in points {
            let rule = cell(&p["rule"]);
            let m = &p["metrics"][0];
            let Some(k) = p["k"].as_f64() else { continue };
            match by_rule.iter_mut().find(|l| l.label == rule) {
                Some(l) => l.points.push((k, &m["value"], &m["ci"])),
                None => by_rule.push(svg::Line {
                    label: rule,
                    points: vec![(k, &m["value"], &m["ci"])],
                }),
            }
        }
        let several = by_rule.iter().any(|l| l.points.len() > 1);
        let metric = points
            .first()
            .and_then(|p| p["metrics"][0]["metric"].as_str())
            .unwrap_or("value");
        let part = if several {
            format!(
                "{}<details><summary>{}</summary>{table}</details>",
                svg::lines(&by_rule, (T::ReplicatesPerItem.text(v.lang), metric)),
                esc(T::Values.text(v.lang))
            )
        } else {
            table
        };
        body.push_str(&html::panel(T::PerformanceByK.text(v.lang), "", &part));
    }
    body.push_str(&how_to_read(
        &["replicate_disagreement", "noise_floor"],
        v.lang,
    ));
    Ok(Screen {
        title: T::Reproducibility,
        focus: Focus::Run(run),
        warnings: warnings_of(&c.warnings),
        body,
    })
}

fn reference(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    // The system's row joins the human ones when a run is chosen.
    let run = runs
        .first()
        .map(|first| chosen(q, "run", &runs, first).to_owned());
    let mut body = picker(
        "reference",
        &runs,
        &[("run", run.as_deref().unwrap_or(""))],
        "",
        v.lang,
    );
    let request = agree::Request {
        run: run.clone(),
        ..agree::Request::default()
    };
    let warnings = match agree::compute(v.d, v.config, &request) {
        Ok(o) => {
            let r = &o.envelope.results;
            // Everyone against the human raters under the same rule (leave-one-out).
            let same: Vec<Vec<String>> = r["same_rule"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| {
                    vec![
                        cell(&x["who"]),
                        cell(&x["kind"]),
                        cell(&x["mean_kappa"]),
                        cell(&x["humans_compared"]),
                    ]
                })
                .collect();
            let bars: Vec<svg::Bar> = r["same_rule"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| svg::Bar {
                    label: cell(&x["who"]),
                    value: &x["mean_kappa"],
                    series: if x["kind"] == "human" { "s0" } else { "s1" },
                })
                .collect();
            let low = bars
                .iter()
                .filter_map(|b| b.value.as_f64())
                .fold(0.0, f64::min);
            body.push_str(&html::panel(
                T::SameRule.text(v.lang),
                "",
                &format!(
                    "{}<details><summary>{}</summary>{}</details>",
                    svg::bars(&bars, (low, 1.0), None),
                    esc(T::Values.text(v.lang)),
                    html::table(
                        &[
                            T::Who.text(v.lang),
                            T::Kind.text(v.lang),
                            T::MeanKappa.text(v.lang),
                            T::HumansCompared.text(v.lang),
                        ],
                        &same
                    )
                ),
            ));
            let pairs: Vec<Vec<String>> = r["pairs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| {
                    let v = &x["values"];
                    vec![
                        format!("{} – {}", cell(&x["raters"][0]), cell(&x["raters"][1])),
                        cell(&x["items"]),
                        cell(&v["cohen_kappa"]),
                        cell(&v["gwet_ac1"]),
                        cell(&v["positive_agreement"]),
                        cell(&v["negative_agreement"]),
                    ]
                })
                .collect();
            body.push_str(&html::panel(
                T::RaterPairs.text(v.lang),
                "",
                &html::table(
                    &[
                        T::Pair.text(v.lang),
                        T::Items.text(v.lang),
                        "Cohen κ",
                        "Gwet AC1",
                        T::PositiveAgreement.text(v.lang),
                        T::NegativeAgreement.text(v.lang),
                    ],
                    &pairs,
                ),
            ));
            let variability: Vec<Vec<String>> = r["variability"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(task, v)| {
                    vec![
                        task.clone(),
                        cell(&v["within_rate"]),
                        cell(&v["within_rater"]["pairs"]),
                        cell(&v["between_rate"]),
                        cell(&v["between_raters"]["pairs"]),
                    ]
                })
                .collect();
            body.push_str(&html::panel(
                T::RaterVariability.text(v.lang),
                "",
                &html::table(
                    &[
                        T::Task.text(v.lang),
                        T::WithinRater.text(v.lang),
                        T::Pairs.text(v.lang),
                        T::BetweenRaters.text(v.lang),
                        T::Pairs.text(v.lang),
                    ],
                    &variability,
                ),
            ));
            if let Some(ceiling) = r["ceiling"].as_f64() {
                body.push_str(&html::panel(
                    T::Ceiling.text(v.lang),
                    "",
                    // The sentence is escaped first; the value goes in as markup.
                    &format!(
                        "<p>{}</p>",
                        esc(T::CeilingNote.text(v.lang)).replacen(
                            "{}",
                            &format!(
                                "<span data-ceiling=\"{}\">{}</span>",
                                esc(&cell(&r["ceiling"])),
                                html::shown(ceiling)
                            ),
                            1
                        )
                    ),
                ));
            }
            warnings_of(&o.envelope.warnings)
        }
        Err(e) => {
            body.push_str(&error_note(&e));
            vec![]
        }
    };
    body.push_str(&how_to_read(
        &[
            "rater_design",
            "same_rule",
            "within_rater",
            "ceiling",
            "alt_test",
            "reference_agreement",
        ],
        v.lang,
    ));
    Ok(Screen {
        title: T::ReferenceQuality,
        focus: run.map_or(Focus::None, Focus::Run),
        warnings,
        body,
    })
}

fn discrimination(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen::note(T::Discrimination, T::NoRuns.text(v.lang)));
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let c = v.report(&report_of(&run))?;
    let r = &c.results;
    let mut body = picker("discrimination", &runs, &[("run", &run)], "", v.lang);
    if r["sdt"].is_object() {
        body.push_str(&html::panel(
            T::EqualVarianceModel.text(v.lang),
            "",
            &format!(
                "{}{}",
                svg::sdt_model(&r["sdt"], v.lang),
                html::table(
                    &[T::Dprime.text(v.lang), T::Criterion.text(v.lang)],
                    &[vec![
                        cell(&r["sdt"]["dprime"]),
                        cell(&r["sdt"]["criterion"]),
                    ]],
                )
            ),
        ));
        body.push_str(&html::panel(
            T::RocSpace.text(v.lang),
            "",
            &svg::roc_space(
                &[svg::RocRun {
                    label: run.clone(),
                    sdt: &r["sdt"],
                }],
                v.lang,
            ),
        ));
    }
    if r["roc"].is_object() {
        let auc = format!(
            "<p>AUC <span data-auc=\"{}\">{}</span> [{}]</p>",
            esc(&r["roc"]["auc"].to_string()),
            r["roc"]["auc"]
                .as_f64()
                .map_or_else(|| cell(&r["roc"]["auc"]), html::shown),
            interval(&r["roc"]["ci"]).replace(" – ", ", ")
        );
        body.push_str(&html::panel(
            T::EmpiricalRoc.text(v.lang),
            "",
            &format!(
                "{auc}{}{}",
                svg::roc(&r["roc"], v.lang),
                svg::zroc(&r["roc"], v.lang)
            ),
        ));
    } else {
        body.push_str(&empty_panel(
            T::EmpiricalRoc.text(v.lang),
            &T::NoEmpiricalRoc.fill(v.lang, r["output_type"].as_str().unwrap_or("–")),
        ));
    }
    if r["calibration"].is_object() {
        body.push_str(&html::panel(
            T::Calibration.text(v.lang),
            "",
            &svg::reliability(&r["calibration"], v.lang),
        ));
    }
    body.push_str(&how_to_read(&["auc", "dprime", "criterion"], v.lang));
    Ok(Screen {
        title: T::Discrimination,
        focus: Focus::Run(run),
        warnings: warnings_of(&c.warnings),
        body,
    })
}

/// The data in force, kept with the ledger's last line when they were read.
struct Loaded {
    last_line: String,
    data: Arc<Data>,
}

/// The ledger's last line. The ledger is append-only and every change to the data in
/// force appends a line, so the line names the data without parsing the ledger.
fn last_line(path: &Path) -> Result<String, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .unwrap_or("")
            .to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io_error(path, &e)),
    }
}

/// The data in force: those already read while the ledger's last line is unchanged, else
/// read again. The configuration is read on every request, since editing it writes no
/// ledger line.
fn current(root: &Path, kept: &Mutex<Option<Loaded>>) -> Result<(Arc<Data>, Config), Error> {
    let project = Project::open(root)?;
    let config = project.config()?;
    let last = last_line(&project.ledger_path())?;
    let mut kept = kept.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(k) = kept.as_ref().filter(|k| k.last_line == last) {
        return Ok((k.data.clone(), config));
    }
    let data = Arc::new(data::load(&project)?);
    *kept = Some(Loaded {
        last_line: last,
        data: data.clone(),
    });
    Ok((data, config))
}

type ScreenFn = fn(&View, &[(String, String)]) -> Result<Screen, Error>;

fn route(root: &Path, kept: &Mutex<Option<Loaded>>, path: &str) -> Page {
    let (route, q) = query(path);
    let screen: ScreenFn = match route {
        "" => |v, _| overview(v),
        "compare" => comparison,
        "trend" => trend_screen,
        "reproducibility" => reproducibility,
        "reference" => reference,
        "discrimination" => discrimination,
        _ => return Page::not_found(),
    };
    // English until the configuration is read, so an error reading it is still shown.
    let mut lang = Language::default();
    let shown = current(root, kept).and_then(|(d, config)| {
        lang = config.dashboard.language;
        screen(
            &View {
                d: &d,
                config: &config,
                lang,
            },
            &q,
        )
    });
    match shown {
        Ok(s) => Page::html(html::page(
            i18n::tag(lang),
            s.title.text(lang),
            lead(s.title).map_or("", |t| t.text(lang)),
            &nav(&s.focus, lang),
            &s.warnings,
            &s.body,
        )),
        Err(e) => Page::html(html::page(
            i18n::tag(lang),
            T::Error.text(lang),
            "",
            &nav(&Focus::None, lang),
            &[e.message],
            "",
        )),
    }
}

/// Serve the dashboard on 127.0.0.1 at `port` until the process ends. The first line
/// on stdout names the URL, token included.
pub fn run(root: &Path, port: u16) -> Result<Output, Error> {
    Project::open(root)?;
    let token = evalsig_store::seal::random_token();
    let srv = server::bind(port, token)
        .map_err(|e| Error::usage(format!("cannot listen on 127.0.0.1:{port}: {e}")))?;
    use std::io::Write as _;
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{}", json!({ "url": srv.url() }));
    let _ = out.flush();
    let root: PathBuf = root.to_path_buf();
    let kept = Mutex::new(None);
    srv.serve(move |path| route(&root, &kept, path));
    Err(Error::usage("the dashboard stopped"))
}
