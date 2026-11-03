//! `evalsig dashboard`: the local web dashboard (SPEC §11, D-019). Its six screens
//! render the same results the CLI prints, computed without writing anything: the
//! report and comparison are computed as `report` and `compare` compute them, never
//! recorded or saved.

use std::path::{Path, PathBuf};

use clap::Parser;
use evalsig_dashboard::html::{self, esc};
use evalsig_dashboard::server::{self, Page};
use evalsig_dashboard::svg;
use serde_json::{Value, json};

use crate::commands::{agree, compare, list, report, trend};
use crate::data;
use crate::error::CliError;
use crate::output::Output;
use crate::project::Project;

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The port on 127.0.0.1 (0 picks a free one).
    #[arg(long, default_value_t = 7777)]
    pub port: u16,
}

pub const NAV: [(&str, &str); 6] = [
    ("./", "Overview"),
    ("compare", "Comparison"),
    ("trend", "Trend"),
    ("reproducibility", "Reproducibility"),
    ("reference", "Reference quality"),
    ("discrimination", "Discrimination"),
];

/// Command-line arguments for a command's own parser, so a screen asks exactly what
/// the command would compute.
#[derive(Parser)]
struct Wrap<A: clap::Args> {
    #[command(flatten)]
    args: A,
}

fn parse<A: clap::Args>(words: &[&str]) -> Result<A, CliError> {
    Wrap::<A>::try_parse_from(std::iter::once("evalsig").chain(words.iter().copied()))
        .map(|w| w.args)
        .map_err(|e| CliError::usage(e.render().to_string()))
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
/// Only known run IDs reach a command's parser, so a value cannot pose as a flag.
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

/// The runs, in import order.
fn run_ids(root: &Path) -> Result<Vec<String>, CliError> {
    Ok(list::runs(root)?.envelope.results["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["run_id"].as_str().map(str::to_owned))
        .collect())
}

/// A form choosing runs (and a period length), as plain GET links.
fn picker(route: &str, runs: &[String], fields: &[(&str, &str)]) -> String {
    let mut s = format!("<form method=\"get\" action=\"{}\">", esc(route));
    for (name, chosen) in fields {
        s.push_str(&format!(
            "<label>{} <select name=\"{}\">",
            esc(name),
            esc(name)
        ));
        for r in runs {
            let sel = if r == chosen { " selected" } else { "" };
            s.push_str(&format!("<option{sel}>{}</option>", esc(r)));
        }
        s.push_str("</select></label> ");
    }
    s.push_str("<button>Show</button></form>");
    s
}

/// "How to read" for the metrics on a screen, from the embedded glossary.
fn how_to_read(names: &[&str]) -> String {
    let mut items = String::new();
    for n in names {
        if let Ok(o) = crate::commands::explain::run(n)
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

fn warnings_of(v: &[crate::output::Warning]) -> Vec<String> {
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
    warnings: Vec<String>,
    body: String,
}

fn computed_report(root: &Path, words: &[&str]) -> Result<report::Computed, CliError> {
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    report::compute(&d, &config, &parse::<report::Args>(words)?)
}

fn overview(root: &Path) -> Result<Screen, CliError> {
    let runs = list::runs(root)?.envelope;
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
    let mut body = html::table(
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
    );
    let mut warnings = warnings_of(&runs.warnings);
    let mut names = Vec::new();
    // The project is read once for every run's report.
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let ids = runs.results["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["run_id"].as_str());
    for id in ids {
        // One run that cannot be reported shows its error; the others still render.
        let c = match parse::<report::Args>(&["--", id])
            .and_then(|a| report::compute(&d, &config, &a))
        {
            Ok(c) => c,
            Err(e) => {
                body.push_str(&format!("<h2>{}</h2><p>{}</p>", esc(id), esc(&e.message)));
                continue;
            }
        };
        warnings.extend(warnings_of(&c.warnings));
        body.push_str(&format!("<h2>{}</h2>", esc(id)));
        body.push_str(&svg::intervals(
            &svg::metric_rows(&c.results["metrics"], "value"),
            false,
        ));
        names.extend(metric_names(&c.results["metrics"]));
    }
    names.sort();
    names.dedup();
    body.push_str(&how_to_read(
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
    Ok(Screen {
        title: "Overview",
        warnings,
        body,
    })
}

fn comparison(root: &Path, q: &[(String, String)]) -> Result<Screen, CliError> {
    let runs = run_ids(root)?;
    if runs.len() < 2 {
        return Ok(Screen {
            title: "Comparison",
            warnings: vec![],
            body: "<p>A comparison needs two runs.</p>".into(),
        });
    }
    let base = chosen(q, "baseline", &runs, &runs[0]).to_owned();
    let cand = chosen(q, "candidate", &runs, &runs[1]).to_owned();
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let args = parse::<compare::Args>(&["--items", "--", &base, &cand])?;
    let c = compare::compute(&d, &config, &args, None)?;
    let r = &c.results;
    let mut body = picker(
        "compare",
        &runs,
        &[("baseline", &base), ("candidate", &cand)],
    );
    if let Some(v) = &c.verdict {
        body.push_str(&html::verdict(v.code.as_str(), &v.text));
    }
    body.push_str(&svg::intervals(
        &svg::metric_rows(&r["metrics"], "delta"),
        true,
    ));
    if r["transition"].is_object() {
        body.push_str("<h2>Transitions</h2>");
        body.push_str(&svg::transition(&r["transition"]));
    }
    // Item drill-down: each paired item's values in both arms.
    let items: Vec<Vec<String>> = r["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            let values: Vec<String> = i
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, _)| k.as_str() != "item_id")
                .map(|(k, v)| format!("{k}={}", cell(v)))
                .collect();
            vec![cell(&i["item_id"]), values.join(", ")]
        })
        .collect();
    if !items.is_empty() {
        body.push_str(&format!(
            "<details><summary>Items ({})</summary>{}</details>",
            items.len(),
            html::table(&["Item", "Values"], &items)
        ));
    }
    let names = metric_names(&r["metrics"]);
    body.push_str(&how_to_read(
        &names.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
    Ok(Screen {
        title: "Comparison",
        warnings: warnings_of(&c.warnings),
        body,
    })
}

fn trend_screen(root: &Path, q: &[(String, String)]) -> Result<Screen, CliError> {
    let runs = run_ids(root)?;
    let Some(first) = runs.first() else {
        return Ok(Screen {
            title: "Trend",
            warnings: vec![],
            body: "<p>No runs.</p>".into(),
        });
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let mut body = picker("trend", &runs, &[("run", &run)]);
    match trend::run(root, &parse::<trend::Args>(&["--", &run])?) {
        Ok(out) => {
            let r = &out.envelope.results;
            if let Some(v) = &out.envelope.verdict {
                body.push_str(&html::verdict(v.code.as_str(), &v.text));
            }
            let primary = r["primary_metric"].as_str().unwrap_or_default();
            body.push_str(&svg::trend(&r["periods"], primary));
            body.push_str(&how_to_read(&[primary, "drifted"]));
            Ok(Screen {
                title: "Trend",
                warnings: warnings_of(&out.envelope.warnings),
                body,
            })
        }
        Err(e) => {
            body.push_str(&format!("<p>{}</p>", esc(&e.message)));
            Ok(Screen {
                title: "Trend",
                warnings: vec![],
                body,
            })
        }
    }
}

fn reproducibility(root: &Path, q: &[(String, String)]) -> Result<Screen, CliError> {
    let runs = run_ids(root)?;
    let Some(first) = runs.first() else {
        return Ok(Screen {
            title: "Reproducibility",
            warnings: vec![],
            body: "<p>No runs.</p>".into(),
        });
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let c = computed_report(root, &["--k-curve", "--", &run])?;
    let mut body = picker("reproducibility", &runs, &[("run", &run)]);
    let rep = &c.results["reproducibility"];
    let rows: Vec<Vec<String>> = rep
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| vec![k.clone(), cell(v)])
        .collect();
    body.push_str(&html::table(&["Measure", "Value"], &rows));
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
                    format!("{} – {}", cell(&m["ci"][0]), cell(&m["ci"][1])),
                    cell(&p["cost_per_item"]),
                ]
            })
            .collect();
        body.push_str("<h2>Performance and cost by k</h2>");
        body.push_str(&html::table(
            &["k", "Rule", "Metric", "Value", "Interval", "Cost per item"],
            &rows,
        ));
    }
    body.push_str(&how_to_read(&["replicate_disagreement", "noise_floor"]));
    Ok(Screen {
        title: "Reproducibility",
        warnings: warnings_of(&c.warnings),
        body,
    })
}

fn reference(root: &Path, q: &[(String, String)]) -> Result<Screen, CliError> {
    let runs = run_ids(root)?;
    // The system's row joins the human ones when a run is chosen.
    let run = runs
        .first()
        .map(|first| chosen(q, "run", &runs, first).to_owned());
    let mut words = vec![];
    if let Some(r) = &run {
        words.extend(["--run", r.as_str()]);
    }
    let mut body = picker("reference", &runs, &[("run", run.as_deref().unwrap_or(""))]);
    let warnings = match agree::run(root, &parse::<agree::Args>(&words)?) {
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
            body.push_str("<h2>Same rule: each rater, grader or run against the humans</h2>");
            body.push_str(&html::table(
                &["Who", "Kind", "Mean κ", "Humans compared"],
                &same,
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
            body.push_str("<h2>Rater pairs</h2>");
            body.push_str(&html::table(
                &[
                    "Pair",
                    "Items",
                    "Cohen κ",
                    "Gwet AC1",
                    "Positive agreement",
                    "Negative agreement",
                ],
                &pairs,
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
            body.push_str("<h2>Rater variability</h2>");
            body.push_str(&html::table(
                &["Task", "Within a rater", "Pairs", "Between raters", "Pairs"],
                &variability,
            ));
            if !r["ceiling"].is_null() {
                body.push_str(&format!(
                    "<p>Ceiling with a soft reference: <span data-ceiling=\"{}\">{}</span></p>",
                    esc(&cell(&r["ceiling"])),
                    esc(&cell(&r["ceiling"]))
                ));
            }
            warnings_of(&o.envelope.warnings)
        }
        Err(e) => {
            body.push_str(&format!("<p>{}</p>", esc(&e.message)));
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
        warnings,
        body,
    })
}

fn discrimination(root: &Path, q: &[(String, String)]) -> Result<Screen, CliError> {
    let runs = run_ids(root)?;
    let Some(first) = runs.first() else {
        return Ok(Screen {
            title: "Discrimination",
            warnings: vec![],
            body: "<p>No runs.</p>".into(),
        });
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let c = computed_report(root, &["--", &run])?;
    let r = &c.results;
    let mut body = picker("discrimination", &runs, &[("run", &run)]);
    if r["roc"].is_object() {
        body.push_str(&format!(
            "<p>AUC <span data-auc=\"{}\">{}</span> [{}, {}]</p>",
            esc(&r["roc"]["auc"].to_string()),
            esc(&r["roc"]["auc"].to_string()),
            esc(&r["roc"]["ci"][0].to_string()),
            esc(&r["roc"]["ci"][1].to_string())
        ));
        body.push_str(&svg::roc(&r["roc"]));
        body.push_str(&svg::zroc(&r["roc"]));
    } else {
        body.push_str("<p>No ROC: the run's judgments are not ordered scores.</p>");
    }
    if r["calibration"].is_object() {
        body.push_str("<h2>Calibration</h2>");
        body.push_str(&svg::reliability(&r["calibration"]));
    }
    body.push_str(&how_to_read(&["auc", "dprime", "criterion"]));
    Ok(Screen {
        title: "Discrimination",
        warnings: warnings_of(&c.warnings),
        body,
    })
}

fn route(root: &Path, path: &str) -> Page {
    let (route, q) = query(path);
    let screen = match route {
        "" => overview(root),
        "compare" => comparison(root, &q),
        "trend" => trend_screen(root, &q),
        "reproducibility" => reproducibility(root, &q),
        "reference" => reference(root, &q),
        "discrimination" => discrimination(root, &q),
        _ => return Page::not_found(),
    };
    match screen {
        Ok(s) => Page::html(html::page(s.title, &NAV, &s.warnings, &s.body)),
        Err(e) => Page::html(html::page("Error", &NAV, &[e.message], "")),
    }
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    Project::open(root)?;
    let token = evalsig_store::seal::random_token();
    let srv = server::bind(args.port, token)
        .map_err(|e| CliError::usage(format!("cannot listen on 127.0.0.1:{}: {e}", args.port)))?;
    use std::io::Write as _;
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{}", json!({ "url": srv.url() }));
    let _ = out.flush();
    let root: PathBuf = root.to_path_buf();
    srv.serve(move |path| route(&root, path));
    Err(CliError::usage("the dashboard stopped"))
}
