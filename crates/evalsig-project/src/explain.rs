//! `evalsig explain <name>`: what a term, metric, verdict, validity check or exit code
//! means, from the glossary and the code's own tables (agent-interface spec §2.6).

use serde_json::{Value, json};

use crate::Error;
use crate::envelope::{Envelope, Output, VerdictCode};
use crate::exit::Exit;

pub const GLOSSARY: &str = include_str!("../../../docs/glossary.md");

/// The glossary rows whose identifiers include `name`: (section, term, identifiers,
/// meaning).
fn glossary(name: &str) -> Vec<Value> {
    let mut section = "";
    let mut out = Vec::new();
    for line in GLOSSARY.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            section = h;
            continue;
        }
        let cells: Vec<&str> = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if cells.len() < 3 || cells[0] == "Term" || cells[0].starts_with("---") {
            continue;
        }
        let ids: Vec<&str> = cells[1]
            .split('`')
            .skip(1)
            .step_by(2)
            .flat_map(|c| c.split(['/', ' ']))
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect();
        if ids.contains(&name) || cells[0].eq_ignore_ascii_case(name) {
            out.push(json!({
                "kind": "term",
                "section": section,
                "term": cells[0],
                "identifiers": cells[1],
                "meaning": cells[2],
                "source": "docs/glossary.md",
            }));
        }
    }
    out
}

pub fn run(name: &str) -> Result<Output, Error> {
    let mut matches = glossary(name);
    if let Some(v) = VerdictCode::ALL.into_iter().find(|v| v.as_str() == name) {
        matches.push(json!({ "kind": "verdict", "code": v.as_str(), "meaning": v.meaning() }));
    }
    if let Some(c) = evalsig_core::checks::CHECKS.iter().find(|c| c.code == name) {
        matches.push(json!({
            "kind": "validity_check",
            "code": c.code,
            "meaning": c.summary,
            "default": format!("{:?}", c.default).to_lowercase(),
            "override_flag": c.override_flag,
        }));
    }
    let code = name.strip_prefix("exit-").unwrap_or(name);
    if let Some(e) = Exit::ALL.into_iter().find(|e| e.code().to_string() == code) {
        matches.push(json!({ "kind": "exit_code", "code": e.code(), "meaning": e.meaning() }));
    }
    if matches.is_empty() {
        return Err(Error::usage(format!("nothing is called {name:?}"))
            .with_hint("names are glossary identifiers, verdict codes, validity checks and exit codes; see evalsig help --json"));
    }
    Ok(Output::success(Envelope::new(
        "evalsig.explain/1",
        json!({ "name": name, "matches": matches }),
    )))
}
