//! The output envelope shared by every command (D-008, agent-interface spec §2.2) and
//! its renderings as JSON, Markdown, JUnit XML and text.

use std::fmt::Write as _;

use clap::ValueEnum;
use serde::Serialize;
use serde_json::{Map, Value as Json};

use crate::exit::Exit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum Format {
    Json,
    Md,
    Junit,
    #[default]
    Text,
}

/// The conclusion of an output. The set is fixed within a schema version; agents
/// quote unknown codes without interpreting them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictCode {
    Pass,
    Fail,
    Improved,
    Negligible,
    Worsened,
    Indistinguishable,
    Drifted,
    Stable,
    HumanParity,
    BelowHuman,
    InsufficientData,
    Blocked,
    NeedsDecisions,
    BudgetExhausted,
}

impl VerdictCode {
    pub const ALL: [VerdictCode; 14] = [
        VerdictCode::Pass,
        VerdictCode::Fail,
        VerdictCode::Improved,
        VerdictCode::Negligible,
        VerdictCode::Worsened,
        VerdictCode::Indistinguishable,
        VerdictCode::Drifted,
        VerdictCode::Stable,
        VerdictCode::HumanParity,
        VerdictCode::BelowHuman,
        VerdictCode::InsufficientData,
        VerdictCode::Blocked,
        VerdictCode::NeedsDecisions,
        VerdictCode::BudgetExhausted,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            VerdictCode::Pass => "pass",
            VerdictCode::Fail => "fail",
            VerdictCode::Improved => "improved",
            VerdictCode::Negligible => "negligible",
            VerdictCode::Worsened => "worsened",
            VerdictCode::Indistinguishable => "indistinguishable",
            VerdictCode::Drifted => "drifted",
            VerdictCode::Stable => "stable",
            VerdictCode::HumanParity => "human_parity",
            VerdictCode::BelowHuman => "below_human",
            VerdictCode::InsufficientData => "insufficient_data",
            VerdictCode::Blocked => "blocked",
            VerdictCode::NeedsDecisions => "needs_decisions",
            VerdictCode::BudgetExhausted => "budget_exhausted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Verdict {
    pub code: VerdictCode,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Warning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Next {
    pub command: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct Provenance {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub snapshots: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ledger_seq: Option<u64>,
}

impl Provenance {
    pub fn tool_only() -> Self {
        Provenance {
            tool: crate::tool_version(),
            ..Provenance::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Truncated {
    pub rows_omitted: u64,
    pub hash: String,
}

/// The envelope. `ok` says the command ran to the end, not whether a gate passed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Envelope {
    pub schema: String,
    pub ok: bool,
    pub verdict: Option<Verdict>,
    pub results: Json,
    pub warnings: Vec<Warning>,
    pub next: Vec<Next>,
    pub provenance: Provenance,
    pub truncated: Option<Truncated>,
}

impl Envelope {
    pub fn new(schema: &str, results: impl Serialize) -> Self {
        Envelope {
            schema: schema.to_owned(),
            ok: true,
            verdict: None,
            results: serde_json::to_value(results).expect("results serialize to JSON"),
            warnings: Vec::new(),
            next: Vec::new(),
            provenance: Provenance::tool_only(),
            truncated: None,
        }
    }
}

/// What a command produced: the envelope and the exit code it implies.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub envelope: Envelope,
    pub exit: Exit,
}

impl Output {
    pub fn success(envelope: Envelope) -> Self {
        Output {
            envelope,
            exit: Exit::Success,
        }
    }
}

pub fn render(output: &Output, format: Format) -> String {
    let e = &output.envelope;
    match format {
        Format::Json => {
            let mut s = serde_json::to_string_pretty(e).expect("envelope serializes");
            s.push('\n');
            s
        }
        Format::Md => markdown(e),
        Format::Junit => junit(e, output.exit),
        Format::Text => text(e),
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

fn md_cell(v: &Json) -> String {
    let cell = if v.is_object() || v.is_array() {
        format!("`{v}`")
    } else {
        scalar(v)
    };
    cell.replace('|', "\\|")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn junit(e: &Envelope, exit: Exit) -> String {
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
