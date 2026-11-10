//! The dashboard's six screens (SPEC §11, D-019). They render the same results the CLI
//! prints, computed without writing anything: the report and comparison are computed
//! as `report` and `compare` compute them, never recorded or saved. The data in force
//! are read once and kept while the project ledger is unchanged (D-028).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use evalsig_core::checks::Finding;
use evalsig_core::config::{Config, Language};
use evalsig_core::records::{Label, Record, Run};
use serde_json::{Value, json};

use evalsig_project::analysis::{self, Analysis};
use evalsig_project::data::Data;
use evalsig_project::envelope::{Envelope, Output, Schema, Verdict, Warning};
use evalsig_project::project::{Project, io_error};
use evalsig_project::{Error, agree, checks, compare, data, explain, list, report, trend};

use crate::html::{self, Cell, Tab, Tone, esc};
use crate::i18n::{self, T};
use crate::server::{self, Page};
use crate::svg;

/// One screen: its route, its title, the question it answers, and whether it is about
/// one run (and so offered for each run on the overview).
struct ScreenInfo {
    route: &'static str,
    title: T,
    lead: T,
    about_run: bool,
}

const SCREENS: [ScreenInfo; 6] = [
    ScreenInfo {
        route: "",
        title: T::Overview,
        lead: T::LeadOverview,
        about_run: false,
    },
    ScreenInfo {
        route: "compare",
        title: T::Comparison,
        lead: T::LeadComparison,
        about_run: false,
    },
    ScreenInfo {
        route: "trend",
        title: T::Trend,
        lead: T::LeadTrend,
        about_run: true,
    },
    ScreenInfo {
        route: "reproducibility",
        title: T::Reproducibility,
        lead: T::LeadReproducibility,
        about_run: true,
    },
    ScreenInfo {
        route: "reference",
        title: T::ReferenceQuality,
        lead: T::LeadReferenceQuality,
        about_run: true,
    },
    ScreenInfo {
        route: "discrimination",
        title: T::Discrimination,
        lead: T::LeadDiscrimination,
        about_run: true,
    },
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
/// as its candidate. The tab of `current` is marked.
fn nav(focus: &Focus, current: T, lang: Language) -> Vec<Tab> {
    SCREENS
        .iter()
        .map(|sc| {
            let href = match (sc.route, focus) {
                ("", _) => "./".to_owned(),
                ("compare", Focus::Pair(b, c)) => compare_href(b, c),
                ("compare", Focus::Run(r)) => format!("compare?candidate={}", encode(r)),
                (route, Focus::Run(r) | Focus::Pair(_, r)) => run_href(route, r),
                (route, Focus::None) => route.to_owned(),
            };
            Tab {
                href,
                label: sc.title.text(lang),
                current: sc.title == current,
            }
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
    chosen_at(q, k, runs).map_or(default, |i| runs[i].as_str())
}

/// Where in `runs` the run a query names is, if it is one of them.
fn chosen_at(q: &[(String, String)], k: &str, runs: &[String]) -> Option<usize> {
    get(q, k).and_then(|v| runs.iter().position(|r| r == v))
}

/// An interval `[lo, hi]` written as the cells write numbers.
fn interval(ci: &Value) -> String {
    format!("{} – {}", html::number(&ci[0]), html::number(&ci[1]))
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

/// Whether `compare` can pair two runs by what the records say: the same output type and
/// value semantics. `compare` also wants the same decision rule, which the configuration
/// sets per task; a pair it still refuses shows why on the page.
fn comparable(a: &Run, b: &Run) -> bool {
    a.output_type == b.output_type && a.value_semantics() == b.value_semantics()
}

/// The latest run before `c` that `compare` can pair with it.
fn earlier_comparable(runs: &[&Run], c: usize) -> Option<usize> {
    (0..c).rev().find(|&b| comparable(runs[b], runs[c]))
}

/// The candidate shown without a choice: the latest run with an earlier comparable run,
/// else the second run.
fn default_candidate(runs: &[&Run]) -> usize {
    (1..runs.len())
        .rev()
        .find(|&c| earlier_comparable(runs, c).is_some())
        .unwrap_or(1)
}

/// The baseline for a candidate: the latest comparable run before it, else the first one
/// after it, else its neighbour.
fn baseline_for(runs: &[&Run], c: usize) -> usize {
    earlier_comparable(runs, c)
        .or_else(|| (c + 1..runs.len()).find(|&b| comparable(runs[b], runs[c])))
        .unwrap_or(if c == 0 { 1 } else { c - 1 })
}

/// The toolbar choosing the runs a screen is drawn for, as a plain GET form: each field
/// by its query name, label and chosen run, with `extra` (HTML) after its button.
fn picker(
    route: &str,
    runs: &[String],
    fields: &[(&str, T, &str)],
    extra: &str,
    lang: Language,
) -> String {
    let mut s = format!(
        "<form class=\"toolbar\" method=\"get\" action=\"{}\">",
        esc(route)
    );
    for (name, label, chosen) in fields {
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

/// "How to read" for the metrics on a screen, from the embedded glossary.
fn how_to_read<S: AsRef<str>>(names: &[S], lang: Language) -> String {
    let mut items = String::new();
    for n in names.iter().map(AsRef::as_ref) {
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
    html::details(T::HowToRead.text(lang), &format!("<dl>{items}</dl>"))
}

fn warnings_of(v: &[Warning]) -> Vec<String> {
    v.iter()
        .map(|w| format!("{}: {}", w.code, w.message))
        .collect()
}

/// The warnings and verdict of a computation once its validity-check findings are applied
/// as the command line applies them: every finding is a warning and a stopping one makes
/// the verdict `blocked`. `apply` reads neither the schema nor the results.
fn checked(
    verdict: Option<Verdict>,
    warnings: &[Warning],
    findings: &[Finding],
) -> (Vec<String>, Option<Verdict>) {
    let mut envelope = Envelope::new(Schema::Report, Value::Null);
    envelope.verdict = verdict;
    envelope.warnings = warnings.to_vec();
    let out = checks::apply(envelope, findings);
    (warnings_of(&out.envelope.warnings), out.envelope.verdict)
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
            body: html::empty(text),
        }
    }
}

/// A result that could not be computed, as a warning callout led by its message and
/// followed by its hint.
fn error_note(e: &Error) -> String {
    match &e.hint {
        Some(h) => html::callout(Tone::Warning, Some(&e.message), h),
        None => html::callout(Tone::Warning, None, &e.message),
    }
}

/// A panel holding a sentence of what is missing.
fn empty_panel(title: &str, text: &str) -> String {
    html::panel(title, "", &html::empty(text))
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

    /// Each run's `sdt` detail (d′ and c under the decision rule) with the defaults of the
    /// command line, from its prepared counts alone: no metric, interval or check is
    /// computed. `None` for a run without a decision rule or that cannot be prepared.
    fn sdt(&self, runs: &[&str]) -> Vec<Option<Value>> {
        let Ok((labels, _)) = self.d.labels(None) else {
            return vec![None; runs.len()];
        };
        let labels: Vec<&Label> = labels.iter().collect();
        let items = self.d.items();
        let settings = analysis::settings(self.config, &Analysis::default());
        runs.iter()
            .map(|id| {
                let run = self.d.run(id).ok()?;
                let p = evalsig_core::report::prepare(
                    run,
                    &self.d.judgments(id),
                    &items,
                    &labels,
                    &settings,
                )
                .ok()?;
                serde_json::to_value(p.sdt()?).ok()
            })
            .collect()
    }
}

/// Where to go from the run at `i`: its comparison with the latest earlier run it can be
/// compared with as the main action, and the screens about one run beside it.
fn run_actions(records: &[&Run], i: usize, lang: Language) -> String {
    let run = &records[i].run_id;
    let mut buttons = Vec::new();
    if let Some(b) = earlier_comparable(records, i) {
        let before = &records[b].run_id;
        buttons.push(html::button(
            &compare_href(before, run),
            &T::CompareWith.fill(lang, before),
            false,
        ));
    }
    for sc in SCREENS.iter().filter(|sc| sc.about_run) {
        buttons.push(html::button(
            &run_href(sc.route, run),
            sc.title.text(lang),
            true,
        ));
    }
    html::actions(&buttons)
}

fn overview(v: &View) -> Result<Screen, Error> {
    let l = v.lang;
    let runs = list::runs_of(v.d).envelope;
    let rows: Vec<Vec<Cell>> = runs.results["runs"]
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
            .map(|k| Cell::of(&r[*k]))
            .collect()
        })
        .collect();
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
    let records = run_records(v.d);
    // Every run's report first, so that all the plots share one axis.
    let reports: Vec<Result<report::Computed, Error>> = records
        .iter()
        .map(|r| v.report(&report_of(&r.run_id)))
        .collect();
    let all: Vec<svg::Row> = reports
        .iter()
        .flatten()
        .flat_map(|c| svg::metric_rows(&c.results["metrics"], "value"))
        .collect();
    let shared = svg::range(&all, false);
    let mut names = Vec::new();
    for (i, c) in reports.iter().enumerate() {
        // One run that cannot be reported shows its error; the others still render.
        let plot = match c {
            Err(e) => error_note(e),
            Ok(c) => {
                // The dataset checks run over every record, so each run's report repeats them.
                for w in checked(None, &c.warnings, &c.findings).0 {
                    if !warnings.contains(&w) {
                        warnings.push(w);
                    }
                }
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
        body.push_str(&html::panel(
            &records[i].run_id,
            &run_actions(&records, i, l),
            &plot,
        ));
    }
    names.sort();
    names.dedup();
    body.push_str(&how_to_read(&names, l));
    Ok(Screen {
        title: T::Overview,
        focus: Focus::None,
        warnings,
        body,
    })
}

/// How an item's judgment moved from the baseline to the candidate, in the order the
/// comparison lists them: away from its reference share, toward it, or only changed when
/// there is no reference to tell.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Move {
    Further,
    Closer,
    Changed,
}

impl Move {
    fn of(item: &Value) -> Option<Move> {
        let (b, c) = (&item["baseline"], &item["candidate"]);
        if b == c {
            return None;
        }
        match (b.as_f64(), c.as_f64(), item["reference"].as_f64()) {
            (Some(b), Some(c), Some(r)) if (c - r).abs() < (b - r).abs() => Some(Move::Closer),
            (Some(b), Some(c), Some(r)) if (c - r).abs() > (b - r).abs() => Some(Move::Further),
            _ => Some(Move::Changed),
        }
    }

    fn badge(self, lang: Language) -> String {
        let (tone, label) = match self {
            Move::Further => (Tone::Negative, T::Further),
            Move::Closer => (Tone::Positive, T::Closer),
            Move::Changed => (Tone::Neutral, T::Changed),
        };
        html::badge(tone, label.text(lang))
    }
}

fn comparison(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let records = run_records(v.d);
    let runs = run_ids(v.d);
    if runs.len() < 2 {
        return Ok(Screen::note(T::Comparison, T::NeedsTwoRuns.text(v.lang)));
    }
    let at = chosen_at(q, "candidate", &runs).unwrap_or_else(|| default_candidate(&records));
    let before = chosen_at(q, "baseline", &runs).unwrap_or_else(|| baseline_for(&records, at));
    let (base, cand) = (runs[before].clone(), runs[at].clone());
    let mut body = picker(
        "compare",
        &runs,
        &[
            ("baseline", T::Baseline, &base),
            ("candidate", T::Candidate, &cand),
        ],
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
    let warnings = match compare::compute(v.d, v.config, &request, None) {
        Ok(c) => {
            let (warnings, verdict) = checked(c.verdict.clone(), &c.warnings, &c.findings);
            comparison_body(v, &c.results, verdict.as_ref(), (&base, &cand), &mut body);
            warnings
        }
        Err(e) => {
            body.push_str(&error_note(&e));
            vec![]
        }
    };
    Ok(Screen {
        title: T::Comparison,
        focus: Focus::Pair(base, cand),
        warnings,
        body,
    })
}

/// What a computed comparison shows: the verdict, the differences, both runs in ROC
/// space, the transitions, and the items whose judgment changed.
fn comparison_body(
    v: &View,
    r: &Value,
    verdict: Option<&Verdict>,
    (base, cand): (&str, &str),
    body: &mut String,
) {
    let l = v.lang;
    if let Some(vd) = verdict {
        body.push_str(&html::verdict(vd.code.as_str(), &vd.text));
    }
    body.push_str(&html::panel(
        T::Differences.text(l),
        "",
        &svg::metric_plots(&r["metrics"], "delta", true),
    ));
    // Both runs in ROC space: a move along one curve is a changed criterion, a move to a
    // higher curve a better separation.
    let sdt = v.sdt(&[base, cand]);
    let space = svg::roc_space(
        &[
            svg::RocRun {
                label: T::BaselineRun.fill(l, base),
                sdt: sdt[0].as_ref().unwrap_or(&Value::Null),
            },
            svg::RocRun {
                label: T::CandidateRun.fill(l, cand),
                sdt: sdt[1].as_ref().unwrap_or(&Value::Null),
            },
        ],
        l,
    );
    if !space.is_empty() {
        body.push_str(&html::panel(T::RocSpace.text(l), "", &space));
    }
    if r["transition"].is_object() {
        body.push_str(&html::panel(
            T::Transitions.text(l),
            "",
            &svg::transition(&r["transition"], l),
        ));
    }
    // The items whose judgment changed, those moving away from the reference first; the
    // unchanged ones folded.
    let items = r["items"].as_array().map_or(&[][..], Vec::as_slice);
    if items.is_empty() {
        body.push_str(&how_to_read(&metric_names(&r["metrics"]), l));
        return;
    }
    let (mut changed, same): (Vec<_>, Vec<_>) = items
        .iter()
        .map(|i| (i, Move::of(i)))
        .partition(|(_, m)| m.is_some());
    changed.sort_by_key(|(_, m)| *m);
    let row = |i: &Value| -> Vec<Cell> {
        ["item_id", "reference", "baseline", "candidate"]
            .iter()
            .map(|k| Cell::of(&i[*k]))
            .collect()
    };
    let head = [
        T::Item.text(l),
        T::Reference.text(l),
        T::Baseline.text(l),
        T::Candidate.text(l),
    ];
    let mut part = if changed.is_empty() {
        html::empty(T::NoItemChanged.text(l))
    } else {
        let rows: Vec<Vec<Cell>> = changed
            .iter()
            .filter_map(|(i, m)| {
                let mut r = row(i);
                r.push(Cell::Html(m.as_ref()?.badge(l)));
                Some(r)
            })
            .collect();
        html::table(
            &[head[0], head[1], head[2], head[3], T::Change.text(l)],
            &rows,
        )
    };
    if !same.is_empty() {
        let rows: Vec<Vec<Cell>> = same.iter().map(|(i, _)| row(i)).collect();
        part.push_str(&html::details(
            &T::ItemsUnchanged.fill(l, &same.len().to_string()),
            &html::table(&head, &rows),
        ));
    }
    body.push_str(&html::panel(
        &T::ItemsChanged.fill(l, &changed.len().to_string()),
        "",
        &part,
    ));
    body.push_str(&how_to_read(&metric_names(&r["metrics"]), l));
}

fn trend_screen(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen::note(T::Trend, T::NoRuns.text(v.lang)));
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let mut body = picker("trend", &runs, &[("run", T::RunField, &run)], "", v.lang);
    let request = trend::Request {
        run: run.clone(),
        ..trend::Request::default()
    };
    let warnings = match trend::compute(v.d, v.config, &request) {
        Ok(out) => {
            let r = &out.envelope.results;
            if let Some(vd) = &out.envelope.verdict {
                body.push_str(&html::verdict(vd.code.as_str(), &vd.text));
            }
            let primary = r["primary_metric"].as_str().unwrap_or_default();
            body.push_str(&html::panel(
                &T::ByPeriod.fill(v.lang, primary),
                "",
                &svg::trend(&r["periods"], primary),
            ));
            body.push_str(&how_to_read(&[primary, "drifted"], v.lang));
            warnings_of(&out.envelope.warnings)
        }
        Err(e) => {
            body.push_str(&error_note(&e));
            vec![]
        }
    };
    Ok(Screen {
        title: T::Trend,
        focus: Focus::Run(run),
        warnings,
        body,
    })
}

fn reproducibility(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let l = v.lang;
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen::note(T::Reproducibility, T::NoRuns.text(l)));
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let c = v.report(&report::Request {
        k_curve: true,
        ..report_of(&run)
    })?;
    let mut body = picker(
        "reproducibility",
        &runs,
        &[("run", T::RunField, &run)],
        "",
        l,
    );
    // One row per kind of replicate, with the measures of a reproducibility row.
    let rows: Vec<Vec<Cell>> = c.results["reproducibility"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(source, m)| {
            let mut row = vec![Cell::from(source.as_str())];
            row.extend(
                [
                    "items",
                    "items_with_any_disagreement",
                    "pairwise_disagreement",
                    "upper_bound",
                ]
                .iter()
                .map(|k| Cell::of(&m[*k])),
            );
            row
        })
        .collect();
    if rows.is_empty() {
        body.push_str(&empty_panel(
            T::ReplicateAgreement.text(l),
            T::NoReplicates.text(l),
        ));
    } else {
        body.push_str(&html::panel(
            T::ReplicateAgreement.text(l),
            "",
            &html::table(
                &[
                    T::Source.text(l),
                    T::ItemsWithReplicates.text(l),
                    T::ItemsWithAnyDisagreement.text(l),
                    T::PairwiseDisagreement.text(l),
                    T::UpperBound.text(l),
                ],
                &rows,
            ),
        ));
    }
    // The k-curve: each rule and k with the primary metric's value and interval, and
    // the cost per item when the configuration prices a generation.
    if let Some(points) = c.results["k_curve"]["points"].as_array() {
        let rows: Vec<Vec<Cell>> = points
            .iter()
            .map(|p| {
                let m = &p["metrics"][0];
                vec![
                    Cell::of(&p["k"]),
                    Cell::of(&p["rule"]),
                    Cell::of(&m["metric"]),
                    Cell::of(&m["value"]),
                    Cell::from(interval(&m["ci"])),
                    Cell::of(&p["cost_per_item"]),
                ]
            })
            .collect();
        let table = html::table(
            &[
                T::K.text(l),
                T::Rule.text(l),
                T::Metric.text(l),
                T::Value.text(l),
                T::Interval.text(l),
                T::CostPerItem.text(l),
            ],
            &rows,
        );
        // One line per rule over k, drawn when more than one k was judged.
        let mut by_rule: Vec<svg::Line> = Vec::new();
        for p in points {
            let rule = p["rule"].as_str().unwrap_or("–").to_owned();
            let m = &p["metrics"][0];
            let Some(k) = p["k"].as_f64() else { continue };
            match by_rule.iter_mut().find(|line| line.label == rule) {
                Some(line) => line.points.push((k, &m["value"], &m["ci"])),
                None => by_rule.push(svg::Line {
                    label: rule,
                    points: vec![(k, &m["value"], &m["ci"])],
                }),
            }
        }
        let metric = points
            .first()
            .and_then(|p| p["metrics"][0]["metric"].as_str())
            .unwrap_or("value");
        let part = if by_rule.iter().any(|line| line.points.len() > 1) {
            format!(
                "{}{}",
                svg::lines(&by_rule, (T::ReplicatesPerItem.text(l), metric)),
                html::details(T::Values.text(l), &table)
            )
        } else {
            table
        };
        body.push_str(&html::panel(T::PerformanceByK.text(l), "", &part));
    }
    body.push_str(&how_to_read(&["replicate_disagreement", "noise_floor"], l));
    Ok(Screen {
        title: T::Reproducibility,
        focus: Focus::Run(run),
        warnings: checked(None, &c.warnings, &c.findings).0,
        body,
    })
}

fn reference(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let l = v.lang;
    let runs = run_ids(v.d);
    // The system's row joins the human ones when a run is chosen.
    let run = runs
        .first()
        .map(|first| chosen(q, "run", &runs, first).to_owned());
    let mut body = picker(
        "reference",
        &runs,
        &[("run", T::RunField, run.as_deref().unwrap_or(""))],
        "",
        l,
    );
    let request = agree::Request {
        run: run.clone(),
        ..agree::Request::default()
    };
    let warnings = match agree::compute(v.d, v.config, &request) {
        Ok(o) => {
            let r = &o.envelope.results;
            // Everyone against the human raters under the same rule (leave-one-out).
            let same_rule = r["same_rule"].as_array().map_or(&[][..], Vec::as_slice);
            let same: Vec<Vec<Cell>> = same_rule
                .iter()
                .map(|x| {
                    ["who", "kind", "mean_kappa", "humans_compared"]
                        .iter()
                        .map(|k| Cell::of(&x[*k]))
                        .collect()
                })
                .collect();
            let bars: Vec<svg::Bar> = same_rule
                .iter()
                .map(|x| svg::Bar {
                    label: x["who"].as_str().unwrap_or("–").to_owned(),
                    value: &x["mean_kappa"],
                    series: if x["kind"] == "human" { "s0" } else { "s1" },
                })
                .collect();
            let low = bars
                .iter()
                .filter_map(|b| b.value.as_f64())
                .fold(0.0, f64::min);
            body.push_str(&html::panel(
                T::SameRule.text(l),
                "",
                &format!(
                    "{}{}",
                    svg::bars(&bars, (low, 1.0)),
                    html::details(
                        T::Values.text(l),
                        &html::table(
                            &[
                                T::Who.text(l),
                                T::Kind.text(l),
                                T::MeanKappa.text(l),
                                T::HumansCompared.text(l),
                            ],
                            &same,
                        )
                    )
                ),
            ));
            let pairs: Vec<Vec<Cell>> = r["pairs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| {
                    let rater = |i: usize| x["raters"][i].as_str().unwrap_or("–");
                    let m = &x["values"];
                    vec![
                        Cell::from(format!("{} – {}", rater(0), rater(1))),
                        Cell::of(&x["items"]),
                        Cell::of(&m["cohen_kappa"]),
                        Cell::of(&m["gwet_ac1"]),
                        Cell::of(&m["positive_agreement"]),
                        Cell::of(&m["negative_agreement"]),
                    ]
                })
                .collect();
            body.push_str(&html::panel(
                T::RaterPairs.text(l),
                "",
                &html::table(
                    &[
                        T::Pair.text(l),
                        T::Items.text(l),
                        "Cohen κ",
                        "Gwet AC1",
                        T::PositiveAgreement.text(l),
                        T::NegativeAgreement.text(l),
                    ],
                    &pairs,
                ),
            ));
            let variability: Vec<Vec<Cell>> = r["variability"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(task, m)| {
                    vec![
                        Cell::from(task.as_str()),
                        Cell::of(&m["within_rate"]),
                        Cell::of(&m["within_rater"]["pairs"]),
                        Cell::of(&m["between_rate"]),
                        Cell::of(&m["between_raters"]["pairs"]),
                    ]
                })
                .collect();
            body.push_str(&html::panel(
                T::RaterVariability.text(l),
                "",
                &html::table(
                    &[
                        T::Task.text(l),
                        T::WithinRater.text(l),
                        T::Pairs.text(l),
                        T::BetweenRaters.text(l),
                        T::Pairs.text(l),
                    ],
                    &variability,
                ),
            ));
            if r["ceiling"].is_number() {
                let value = format!(
                    "<span data-ceiling=\"{}\">{}</span>",
                    esc(&r["ceiling"].to_string()),
                    html::number(&r["ceiling"])
                );
                body.push_str(&html::panel(
                    T::Ceiling.text(l),
                    "",
                    &format!("<p>{}</p>", T::CeilingNote.fill_html(l, &value)),
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
        l,
    ));
    Ok(Screen {
        title: T::ReferenceQuality,
        focus: run.map_or(Focus::None, Focus::Run),
        warnings,
        body,
    })
}

fn discrimination(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let l = v.lang;
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen::note(T::Discrimination, T::NoRuns.text(l)));
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let c = v.report(&report_of(&run))?;
    let r = &c.results;
    let mut body = picker(
        "discrimination",
        &runs,
        &[("run", T::RunField, &run)],
        "",
        l,
    );
    if r["sdt"].is_object() {
        body.push_str(&html::panel(
            T::EqualVarianceModel.text(l),
            "",
            &format!(
                "{}{}",
                svg::sdt_model(&r["sdt"], l),
                html::table(
                    &[T::Dprime.text(l), T::Criterion.text(l)],
                    &[vec![
                        Cell::of(&r["sdt"]["dprime"]),
                        Cell::of(&r["sdt"]["criterion"]),
                    ]],
                )
            ),
        ));
        body.push_str(&html::panel(
            T::RocSpace.text(l),
            "",
            &svg::roc_space(
                &[svg::RocRun {
                    label: run.clone(),
                    sdt: &r["sdt"],
                }],
                l,
            ),
        ));
    }
    if r["roc"].is_object() {
        let roc = &r["roc"];
        let auc = format!(
            "<p>AUC <span data-auc=\"{}\">{}</span> [{}, {}]</p>",
            esc(&roc["auc"].to_string()),
            html::number(&roc["auc"]),
            html::number(&roc["ci"][0]),
            html::number(&roc["ci"][1])
        );
        body.push_str(&html::panel(
            T::EmpiricalRoc.text(l),
            "",
            &format!("{auc}{}{}", svg::roc(roc, l), svg::zroc(roc, l)),
        ));
    } else {
        body.push_str(&empty_panel(
            T::EmpiricalRoc.text(l),
            &T::NoEmpiricalRoc.fill(l, r["output_type"].as_str().unwrap_or("–")),
        ));
    }
    if r["calibration"].is_object() {
        body.push_str(&html::panel(
            T::Calibration.text(l),
            "",
            &svg::reliability(&r["calibration"], l),
        ));
    }
    body.push_str(&how_to_read(&["auc", "dprime", "criterion"], l));
    Ok(Screen {
        title: T::Discrimination,
        focus: Focus::Run(run),
        warnings: checked(None, &c.warnings, &c.findings).0,
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
            SCREENS
                .iter()
                .find(|sc| sc.title == s.title)
                .map_or("", |sc| sc.lead.text(lang)),
            &nav(&s.focus, s.title, lang),
            &s.warnings,
            &s.body,
        )),
        Err(e) => Page::html(html::page(
            i18n::tag(lang),
            T::Error.text(lang),
            "",
            &nav(&Focus::None, T::Error, lang),
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
