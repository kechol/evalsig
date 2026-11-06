//! Renderings of the output envelope as JSON, Markdown, JUnit XML, JSON Lines, HTML
//! and text.

use std::fmt::Write as _;

use clap::ValueEnum;
use evalsig_project::envelope::{Envelope, Output, Schema};
use evalsig_project::exit::Exit;
use serde_json::{Map, Value as Json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum Format {
    Json,
    Md,
    Junit,
    #[default]
    Text,
    /// One JSON object per line: the per-item rows of `compare --items` (D-023).
    Jsonl,
    /// One self-contained HTML page with the data embedded (D-019).
    Html,
}

pub fn render(output: &Output, format: Format) -> String {
    let e = &output.envelope;
    match format {
        Format::Json => e.to_pretty_json(),
        Format::Md => crate::markdown::render(e).unwrap_or_else(|| markdown(e)),
        Format::Junit => junit(e, output.exit),
        Format::Html => {
            evalsig_dashboard::static_report(&serde_json::to_value(e).expect("envelope serializes"))
        }
        Format::Text => text(e),
        Format::Jsonl => {
            // Rows sit in `results.items`, or in each comparison's `items` when several
            // candidates were compared.
            let nested = e.results["comparisons"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| &c["items"]);
            std::iter::once(&e.results["items"])
                .chain(nested)
                .filter_map(Json::as_array)
                .flatten()
                .map(|r| format!("{r}\n"))
                .collect()
        }
    }
}

fn scalar(v: &Json) -> String {
    match v {
        Json::String(s) => s.clone(),
        Json::Null => "—".to_owned(),
        other => other.to_string(),
    }
}

fn markdown(e: &Envelope) -> String {
    let mut s = format!("### `{}`\n\n", e.schema);
    if let Some(v) = &e.verdict {
        let _ = writeln!(s, "**{}** — {}\n", v.code.as_str(), v.text);
    }
    if let Json::Object(m) = &e.results
        && !m.is_empty()
    {
        let (tables, fields): (Vec<_>, Vec<_>) =
            m.iter().partition(|(_, v)| table_columns(v).is_some());
        if !fields.is_empty() {
            s.push_str("| Field | Value |\n|---|---|\n");
            for (k, v) in fields {
                let _ = writeln!(s, "| {k} | {} |", md_cell(v));
            }
            s.push('\n');
        }
        for (k, v) in tables {
            let rows = v.as_array().expect("tables are arrays");
            let cols = table_columns(v).expect("partitioned on columns");
            let _ = writeln!(s, "**{k}**\n");
            let _ = writeln!(s, "| {} |", cols.join(" | "));
            let _ = writeln!(s, "|{}", "---|".repeat(cols.len()));
            for row in rows {
                let cells: Vec<String> = cols.iter().map(|c| md_cell(&row[c.as_str()])).collect();
                let _ = writeln!(s, "| {} |", cells.join(" | "));
            }
            s.push('\n');
        }
    }
    for w in &e.warnings {
        let _ = writeln!(s, "- ⚠ `{}`: {}", w.code, w.message);
    }
    for n in &e.next {
        let _ = writeln!(s, "- Next: `{}` — {}", n.command, n.why);
    }
    let _ = writeln!(s, "\n<sub>{}</sub>", e.provenance.tool);
    s
}

/// Column names when `v` is a non-empty array of objects, in first-seen order.
fn table_columns(v: &Json) -> Option<Vec<String>> {
    let rows = v.as_array().filter(|r| !r.is_empty())?;
    let mut cols: Vec<String> = Vec::new();
    for r in rows {
        for k in r.as_object()?.keys() {
            if !cols.contains(k) {
                cols.push(k.clone());
            }
        }
    }
    Some(cols)
}

/// A Markdown table cell: one line, pipes escaped, structured values in a code
/// span whose fence is longer than any backtick run inside.
fn md_cell(v: &Json) -> String {
    let cell = if v.is_object() || v.is_array() {
        crate::markdown::code(&v.to_string())
    } else {
        scalar(v)
    };
    cell.replace(['\n', '\r'], " ").replace('|', "\\|")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One test case per condition of a check or gate, failed when it does not hold and
/// skipped when it could not be decided.
/// An exit other than success with no failed condition adds one failed case, so the
/// suite is never green when the process was not.
fn junit_conditions(e: &Envelope, conditions: &[Json], exit: Exit) -> String {
    let failed_conditions = conditions.iter().filter(|c| c["passed"] == false).count();
    // A stop, or any other exit that is not success with no failed condition (too few
    // items, every condition undecided), adds one failed case.
    let blocked = exit != Exit::Success && failed_conditions == 0;
    let failures = failed_conditions + usize::from(blocked);
    let skipped = conditions.iter().filter(|c| c["passed"].is_null()).count();
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        s,
        "<testsuites><testsuite name=\"{}\" tests=\"{}\" failures=\"{failures}\" skipped=\"{skipped}\">",
        xml_escape(&e.schema),
        conditions.len() + usize::from(blocked),
    );
    for c in conditions {
        let name = format!(
            "{} {} {}",
            c["metric"].as_str().unwrap_or("?"),
            c["rule"].as_str().unwrap_or("?"),
            c["value"]
        );
        let _ = write!(
            s,
            "<testcase classname=\"evalsig\" name=\"{}\">",
            xml_escape(&name)
        );
        // The referee returns verdicts only, so a gate case names no number.
        let detail = if e.schema == Schema::Gate.name() {
            "the referee returns the verdict only".to_owned()
        } else {
            format!(
                "estimate {}, bound {} ({}-sided)",
                c["estimate"],
                c["bound"],
                c["sided"].as_str().unwrap_or("?")
            )
        };
        if c["passed"] == false {
            let _ = write!(s, "<failure message=\"{}\"/>", xml_escape(&detail));
        } else if c["passed"].is_null() {
            let _ = write!(
                s,
                "<skipped message=\"{}\"/>",
                xml_escape(&format!("insufficient data: {detail}"))
            );
        }
        s.push_str("</testcase>\n");
    }
    if blocked {
        // The case's name says whether a validity check stopped the run.
        let msg = e.verdict.as_ref().map_or("", |v| v.text.as_str());
        let _ = writeln!(
            s,
            "<testcase classname=\"evalsig\" name=\"{}\"><failure message=\"{}\"/></testcase>",
            if exit == Exit::Blocked {
                "validity checks"
            } else {
                "verdict"
            },
            xml_escape(msg)
        );
    }
    s.push_str("</testsuite></testsuites>\n");
    s
}

fn junit(e: &Envelope, exit: Exit) -> String {
    if let Some(conditions) = e.results["conditions"].as_array() {
        return junit_conditions(e, conditions, exit);
    }
    let name = e.verdict.as_ref().map_or("ok", |v| v.code.as_str());
    let failed = exit != Exit::Success;
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        s,
        "<testsuites><testsuite name=\"{}\" tests=\"1\" failures=\"{}\">",
        xml_escape(&e.schema),
        u8::from(failed)
    );
    let _ = write!(
        s,
        "<testcase classname=\"evalsig\" name=\"{}\">",
        xml_escape(name)
    );
    if failed {
        let msg = e.verdict.as_ref().map_or("", |v| v.text.as_str());
        let _ = write!(
            s,
            "<failure message=\"{}\" type=\"{}\"/>",
            xml_escape(msg),
            exit.code()
        );
    }
    s.push_str("</testcase></testsuite></testsuites>\n");
    s
}

fn text_cell(v: &Json) -> String {
    if v.is_object() || v.is_array() {
        v.to_string()
    } else {
        scalar(v)
    }
}

fn text(e: &Envelope) -> String {
    let mut s = String::new();
    if let Some(v) = &e.verdict {
        let _ = writeln!(s, "{}: {}", v.code.as_str(), v.text);
    }
    if let Json::Object(m) = &e.results {
        write_text_fields(&mut s, m, 0);
    }
    for w in &e.warnings {
        let _ = writeln!(s, "warning[{}]: {}", w.code, w.message);
    }
    for n in &e.next {
        let _ = writeln!(s, "next: {}  # {}", n.command, n.why);
    }
    s
}

fn write_text_fields(s: &mut String, m: &Map<String, Json>, depth: usize) {
    let pad = "  ".repeat(depth);
    for (k, v) in m {
        match v {
            Json::Object(inner) if !inner.is_empty() => {
                let _ = writeln!(s, "{pad}{k}:");
                write_text_fields(s, inner, depth + 1);
            }
            Json::Array(_) if table_columns(v).is_some() => {
                let cols = table_columns(v).expect("checked");
                let rows: Vec<Vec<String>> = v
                    .as_array()
                    .expect("checked")
                    .iter()
                    .map(|r| cols.iter().map(|c| text_cell(&r[c.as_str()])).collect())
                    .collect();
                let widths: Vec<usize> = cols
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        rows.iter()
                            .map(|r| r[i].chars().count())
                            .chain([c.len()])
                            .max()
                            .unwrap_or(0)
                    })
                    .collect();
                let _ = writeln!(s, "{pad}{k}:");
                let line = |cells: Vec<&str>| {
                    let padded: Vec<String> = cells
                        .iter()
                        .zip(&widths)
                        .map(|(c, w)| format!("{c:<w$}"))
                        .collect();
                    format!("{pad}  {}", padded.join("  ").trim_end())
                };
                let _ = writeln!(s, "{}", line(cols.iter().map(String::as_str).collect()));
                for r in &rows {
                    let _ = writeln!(s, "{}", line(r.iter().map(String::as_str).collect()));
                }
            }
            Json::Array(xs) if xs.is_empty() => {
                let _ = writeln!(s, "{pad}{k}: (none)");
            }
            Json::Array(xs) if xs.iter().all(|x| !x.is_object() && !x.is_array()) => {
                let items: Vec<String> = xs.iter().map(scalar).collect();
                let _ = writeln!(s, "{pad}{k}: {}", items.join(", "));
            }
            Json::Array(_) | Json::Object(_) => {
                let _ = writeln!(s, "{pad}{k}: {v}");
            }
            _ => {
                let _ = writeln!(s, "{pad}{k}: {}", scalar(v));
            }
        }
    }
}
