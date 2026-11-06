//! The dashboard's six screens (SPEC §11, D-019). They render the same results the CLI
//! prints, computed without writing anything: the report and comparison are computed
//! as `report` and `compare` compute them, never recorded or saved. The data in force
//! are read once and kept while the project ledger's head is unchanged (D-028).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use evalsig_core::config::Config;
use evalsig_core::records::Record;
use evalsig_store::ledger;
use serde_json::{Value, json};

use evalsig_project::data::Data;
use evalsig_project::envelope::Output;
use evalsig_project::project::Project;
use evalsig_project::{Error, agree, compare, data, explain, list, report, trend};

use crate::html::{self, esc};
use crate::server::{self, Page};
use crate::svg;

pub const NAV: [(&str, &str); 6] = [
    ("./", "Overview"),
    ("compare", "Comparison"),
    ("trend", "Trend"),
    ("reproducibility", "Reproducibility"),
    ("reference", "Reference quality"),
    ("discrimination", "Discrimination"),
];

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

/// The runs, in import order.
fn run_ids(d: &Data) -> Vec<String> {
    d.records
        .iter()
        .filter_map(|r| match r {
            Record::Run(run) => Some(run.run_id.clone()),
            _ => None,
        })
        .collect()
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
    warnings: Vec<String>,
    body: String,
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
    let ids = runs.results["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["run_id"].as_str());
    for id in ids {
        // One run that cannot be reported shows its error; the others still render.
        let c = match v.report(&report_of(id)) {
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

fn comparison(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    if runs.len() < 2 {
        return Ok(Screen {
            title: "Comparison",
            warnings: vec![],
            body: "<p>A comparison needs two runs.</p>".into(),
        });
    }
    let base = chosen(q, "baseline", &runs, &runs[0]).to_owned();
    let cand = chosen(q, "candidate", &runs, &runs[1]).to_owned();
    let request = compare::Request {
        baseline: base.clone(),
        candidates: vec![cand.clone()],
        items: true,
        ..compare::Request::default()
    };
    let c = compare::compute(v.d, v.config, &request, None)?;
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

fn trend_screen(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen {
            title: "Trend",
            warnings: vec![],
            body: "<p>No runs.</p>".into(),
        });
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let mut body = picker("trend", &runs, &[("run", &run)]);
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

fn reproducibility(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen {
            title: "Reproducibility",
            warnings: vec![],
            body: "<p>No runs.</p>".into(),
        });
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let c = v.report(&report::Request {
        k_curve: true,
        ..report_of(&run)
    })?;
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

fn reference(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    // The system's row joins the human ones when a run is chosen.
    let run = runs
        .first()
        .map(|first| chosen(q, "run", &runs, first).to_owned());
    let mut body = picker("reference", &runs, &[("run", run.as_deref().unwrap_or(""))]);
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

fn discrimination(v: &View, q: &[(String, String)]) -> Result<Screen, Error> {
    let runs = run_ids(v.d);
    let Some(first) = runs.first() else {
        return Ok(Screen {
            title: "Discrimination",
            warnings: vec![],
            body: "<p>No runs.</p>".into(),
        });
    };
    let run = chosen(q, "run", &runs, first).to_owned();
    let c = v.report(&report_of(&run))?;
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

/// The data in force, kept with the ledger head they were read at.
struct Loaded {
    head: String,
    data: Arc<Data>,
}

/// The data in force: those already read while the ledger's head is unchanged, else
/// read again. The configuration is read on every request, since editing it writes no
/// ledger line.
fn current(root: &Path, kept: &Mutex<Option<Loaded>>) -> Result<(Arc<Data>, Config), Error> {
    let project = Project::open(root)?;
    let config = project.config()?;
    let entries = ledger::read(&project.ledger_path()).map_err(Error::store)?;
    let head = ledger::head(&entries).to_owned();
    let mut kept = kept.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(k) = kept.as_ref().filter(|k| k.head == head) {
        return Ok((k.data.clone(), config));
    }
    let data = Arc::new(data::load(&project)?);
    *kept = Some(Loaded {
        head,
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
        Ok(s) => Page::html(html::page(s.title, &NAV, &s.warnings, &s.body)),
        Err(e) => Page::html(html::page("Error", &NAV, &[e.message], "")),
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
