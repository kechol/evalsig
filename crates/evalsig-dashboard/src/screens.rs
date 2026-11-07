//! The dashboard's six screens (SPEC §11, D-019). They render the same results the CLI
//! prints, computed without writing anything: the report and comparison are computed
//! as `report` and `compare` compute them, never recorded or saved. The data in force
//! are read once and kept while the project ledger is unchanged (D-028).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use evalsig_core::config::Config;
use evalsig_core::records::{Record, Run};
use serde_json::{Value, json};

use evalsig_project::data::Data;
use evalsig_project::envelope::Output;
use evalsig_project::project::{Project, io_error};
use evalsig_project::{Error, agree, compare, data, explain, list, report, trend};

use crate::html::{self, esc};
use crate::server::{self, Page};
use crate::svg;

/// The screens, by route and title.
pub const SCREENS: [(&str, &str); 6] = [
    ("", "Overview"),
    ("compare", "Comparison"),
    ("trend", "Trend"),
    ("reproducibility", "Reproducibility"),
    ("reference", "Reference quality"),
    ("discrimination", "Discrimination"),
];

/// The screens about one run, as the links offered for it.
const RUN_SCREENS: [(&str, &str); 4] = [
    ("reproducibility", "Reproducibility"),
    ("reference", "Reference quality"),
    ("discrimination", "Discrimination"),
    ("trend", "Trend"),
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
fn nav(focus: &Focus) -> Vec<(String, &'static str)> {
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
            (href, *label)
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
fn picker(route: &str, runs: &[String], fields: &[(&str, &str)], extra: &str) -> String {
    let mut s = format!(
        "<form class=\"toolbar\" method=\"get\" action=\"{}\">",
        esc(route)
    );
    for (name, chosen) in fields {
        s.push_str(&format!(
            "<label>{}<select name=\"{}\">",
            esc(name),
            esc(name)
        ));
        for r in runs {
            let sel = if r == chosen { " selected" } else { "" };
            s.push_str(&format!("<option{sel}>{}</option>", esc(r)));
        }
        s.push_str("</select></label>");
    }
    s.push_str(&format!("<button>Show</button>{extra}</form>"));
    s
}

/// The question each screen answers, under its title.
fn lead(title: &str) -> &'static str {
    match title {
        "Overview" => {
            "How each run performs: every metric with its interval, all runs on one axis."
        }
        "Comparison" => "Whether the candidate differs from the baseline, and on which items.",
        "Trend" => "How a run's primary metric moves from one period to the next.",
        "Reproducibility" => {
            "How often a run's replicates of one item disagree, and what more replicates buy."
        }
        "Reference quality" => {
            "How well the human raters agree with one another, and the system with them."
        }
        "Discrimination" => {
            "How well a run's judgments separate positive items from negative ones."
        }
        _ => "",
    }
}

/// "How to read" for the metrics on a screen, from the embedded glossary.
fn how_to_read(names: &[&str]) -> String {
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
    format!("<details><summary>How to read</summary><dl>{items}</dl></details>")
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
    title: &'static str,
    focus: Focus,
    warnings: Vec<String>,
    body: String,
}

impl Screen {
    /// A screen with nothing to show but a sentence.
    fn note(title: &'static str, text: &str) -> Self {
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
}

impl View<'_> {
    fn report(&self, request: &report::Request) -> Result<report::Computed, Error> {
        report::compute(self.d, self.config, request)
    }
}

/// Where to go from the run at `i`: its comparison with the run imported before it as
/// the main action, and the screens about one run beside it.
fn run_actions(ids: &[String], i: usize) -> String {
    let run = &ids[i];
    let mut buttons = Vec::new();
    if let Some(before) = i.checked_sub(1).map(|j| &ids[j]) {
        buttons.push(html::button(
            &compare_href(before, run),
            &format!("Compare with {before}"),
            false,
        ));
    }
    for (route, label) in RUN_SCREENS {
        buttons.push(html::button(&run_href(route, run), label, true));
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
    let mut body = html::panel(
        "Runs",
        "",
        &html::table(
            &[
                "Run",
                "System",
                "Version",
                "Output type",
                "Items",
                "Judgments",
                "Missing",
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
        body.push_str(&html::panel(id, &run_actions(&ids, i), &plot));
    }
    names.sort();
    names.dedup();
    body.push_str(&how_to_read(
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
    Ok(Screen {
        title: "Overview",
        focus: Focus::None,
        warnings,
        body,
    })
}

/// How an item's judgment moved from the baseline to the candidate: toward its reference
/// share, away from it, or only changed when there is no reference to tell.
fn movement(item: &Value) -> Option<&'static str> {
    let (b, c) = (&item["baseline"], &item["candidate"]);
    if b == c {
        return None;
    }
    match (b.as_f64(), c.as_f64(), item["reference"].as_f64()) {
        (Some(b), Some(c), Some(r)) if (c - r).abs() < (b - r).abs() => {
            Some("closer to the reference")
        }
        (Some(b), Some(c), Some(r)) if (c - r).abs() > (b - r).abs() => {
            Some("further from the reference")
        }
        _ => Some("changed"),
    }
}

fn comparison(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let records = run_records(v.d);
    let runs = run_ids(v.d);
    if runs.len() < 2 {
        return Ok(Screen::note("Comparison", "A comparison needs two runs."));
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
        &html::button(
            &compare_href(&cand, &base),
            "Swap baseline and candidate",
            true,
        ),
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
                title: "Comparison",
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
        "Differences (candidate − baseline)",
        "",
        &svg::intervals(&svg::metric_rows(&r["metrics"], "delta"), true),
    ));
    if r["transition"].is_object() {
        body.push_str(&html::panel(
            "Transitions",
            "",
            &svg::transition(&r["transition"]),
        ));
    }
    // The items whose judgment changed, those moving away from the reference first; the
    // unchanged ones folded.
    let items = r["items"].as_array().map_or(&[][..], Vec::as_slice);
    let mut changed: Vec<(&Value, &str)> = items
        .iter()
        .filter_map(|i| movement(i).map(|m| (i, m)))
        .collect();
    changed.sort_by_key(|(_, m)| match *m {
        "further from the reference" => 0,
        "closer to the reference" => 1,
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
    let head = ["Item", "Reference", "Baseline", "Candidate"];
    if !items.is_empty() {
        let mut part = if changed.is_empty() {
            "<p class=\"empty\">No item's judgment changed.</p>".to_owned()
        } else {
            let rows: Vec<Vec<String>> = changed
                .iter()
                .map(|(i, m)| {
                    let tone = match *m {
                        "further from the reference" => html::Tone::Negative,
                        "closer to the reference" => html::Tone::Positive,
                        _ => html::Tone::Neutral,
                    };
                    let mut r: Vec<String> = row(i).iter().map(|c| html::td(c)).collect();
                    r.push(format!("<td>{}</td>", html::badge(tone, m)));
                    r
                })
                .collect();
            html::table_html(
                &["Item", "Reference", "Baseline", "Candidate", "Change"],
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
                "<details><summary>Unchanged items ({})</summary>{}</details>",
                same.len(),
                html::table(&head, &same)
            ));
        }
        body.push_str(&html::panel(
            &format!("Items that changed ({})", changed.len()),
            "",
            &part,
        ));
    }
    let names = metric_names(&r["metrics"]);
    body.push_str(&how_to_read(
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
    Ok(Screen {
        title: "Comparison",
        focus: Focus::Pair(base, cand),
        warnings: warnings_of(&c.warnings),
        body,
    })
}

fn trend_screen(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen::note("Trend", "No runs."));
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let mut body = picker("trend", &runs, &[("run", &run)], "");
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
                &format!("{primary} by period"),
                "",
                &svg::trend(&r["periods"], primary),
            ));
            body.push_str(&how_to_read(&[primary, "drifted"]));
            Ok(Screen {
                title: "Trend",
                focus: Focus::Run(run),
                warnings: warnings_of(&out.envelope.warnings),
                body,
            })
        }
        Err(e) => {
            body.push_str(&error_note(&e));
            Ok(Screen {
                title: "Trend",
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
        return Ok(Screen::note("Reproducibility", "No runs."));
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let c = v.report(&report::Request {
        k_curve: true,
        ..report_of(&run)
    })?;
    let mut body = picker("reproducibility", &runs, &[("run", &run)], "");
    let rep = &c.results["reproducibility"];
    let rows: Vec<Vec<String>> = rep
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| vec![k.clone(), cell(v)])
        .collect();
    if rows.is_empty() {
        body.push_str(&empty_panel(
            "Replicate agreement",
            "Every item has one judgment in this run, so there are no replicates to \
             compare. Judge each item several times (gen_replicate 0, 1, …) and import \
             the run again.",
        ));
    } else {
        body.push_str(&html::panel(
            "Replicate agreement",
            "",
            &html::table(&["Measure", "Value"], &rows),
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
        body.push_str(&html::panel(
            "Performance and cost by k",
            "",
            &html::table(
                &["k", "Rule", "Metric", "Value", "Interval", "Cost per item"],
                &rows,
            ),
        ));
    }
    body.push_str(&how_to_read(&["replicate_disagreement", "noise_floor"]));
    Ok(Screen {
        title: "Reproducibility",
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
            body.push_str(&html::panel(
                "Same rule: each rater, grader or run against the humans",
                "",
                &html::table(&["Who", "Kind", "Mean κ", "Humans compared"], &same),
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
                "Rater pairs",
                "",
                &html::table(
                    &[
                        "Pair",
                        "Items",
                        "Cohen κ",
                        "Gwet AC1",
                        "Positive agreement",
                        "Negative agreement",
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
                "Rater variability",
                "",
                &html::table(
                    &["Task", "Within a rater", "Pairs", "Between raters", "Pairs"],
                    &variability,
                ),
            ));
            if let Some(ceiling) = r["ceiling"].as_f64() {
                body.push_str(&html::panel(
                    "Ceiling with a soft reference",
                    "",
                    &format!(
                        "<p>No system can exceed accuracy <span data-ceiling=\"{}\">{}</span> against this reference.</p>",
                        esc(&cell(&r["ceiling"])),
                        html::shown(ceiling)
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
    body.push_str(&how_to_read(&[
        "rater_design",
        "same_rule",
        "within_rater",
        "ceiling",
        "alt_test",
        "reference_agreement",
    ]));
    Ok(Screen {
        title: "Reference quality",
        focus: run.map_or(Focus::None, Focus::Run),
        warnings,
        body,
    })
}

fn discrimination(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen::note("Discrimination", "No runs."));
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let c = v.report(&report_of(&run))?;
    let r = &c.results;
    let mut body = picker("discrimination", &runs, &[("run", &run)], "");
    if r["sdt"].is_object() {
        body.push_str(&html::panel(
            "Signal detection",
            "",
            &html::table(
                &["d′ (sensitivity)", "c (criterion)"],
                &[vec![
                    cell(&r["sdt"]["dprime"]),
                    cell(&r["sdt"]["criterion"]),
                ]],
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
            "ROC and z-ROC",
            "",
            &format!("{auc}{}{}", svg::roc(&r["roc"]), svg::zroc(&r["roc"])),
        ));
    } else {
        body.push_str(&empty_panel(
            "ROC and z-ROC",
            &format!(
                "This run's judgments are {}; a curve needs ordered judgments (output type \
                 ordinal or score).",
                r["output_type"].as_str().unwrap_or("not ordered")
            ),
        ));
    }
    if r["calibration"].is_object() {
        body.push_str(&html::panel(
            "Calibration",
            "",
            &svg::reliability(&r["calibration"]),
        ));
    }
    body.push_str(&how_to_read(&["auc", "dprime", "criterion"]));
    Ok(Screen {
        title: "Discrimination",
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
    let shown = current(root, kept).and_then(|(d, config)| {
        screen(
            &View {
                d: &d,
                config: &config,
            },
            &q,
        )
    });
    match shown {
        Ok(s) => Page::html(html::page(
            s.title,
            lead(s.title),
            &nav(&s.focus),
            &s.warnings,
            &s.body,
        )),
        Err(e) => Page::html(html::page(
            "Error",
            "",
            &nav(&Focus::None),
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
