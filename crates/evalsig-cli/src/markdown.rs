//! Markdown for pull-request comments and GitHub step summaries (SPEC §12.2), for
//! the commands whose results people read in review: `report` and `compare`.

use std::fmt::Write as _;

use serde_json::Value;

use crate::output::Envelope;

/// Four decimals, or none for whole counts.
fn num(v: &Value) -> String {
    match v.as_f64() {
        Some(x) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{x:.0}"),
        Some(x) => format!("{x:.4}"),
        None => "—".to_owned(),
    }
}

fn signed(v: &Value) -> String {
    v.as_f64()
        .map_or_else(|| "—".to_owned(), |x| format!("{x:+.4}"))
}

fn pair(v: &Value, f: fn(&Value) -> String) -> String {
    match v.as_array() {
        Some(a) if a.len() == 2 => format!("{} to {}", f(&a[0]), f(&a[1])),
        _ => "—".to_owned(),
    }
}

fn footer(s: &mut String, e: &Envelope) {
    for w in &e.warnings {
        let _ = writeln!(s, "> **Warning** `{}`: {}", w.code, w.message);
    }
    if !e.warnings.is_empty() {
        s.push('\n');
    }
    for n in &e.next {
        let _ = writeln!(s, "Next: `{}` — {}", n.command, n.why);
    }
    let snapshots = e.provenance.snapshots.len();
    let _ = writeln!(
        s,
        "\n<sub>{} · {snapshots} snapshot(s) · config {} · seed {}</sub>",
        e.provenance.tool,
        e.provenance
            .config
            .as_deref()
            .map_or("—", |c| &c[..c.len().min(19)]),
        e.provenance
            .seed
            .map_or_else(|| "—".to_owned(), |x| x.to_string()),
    );
}

/// The Markdown for a report or compare envelope, or `None` for other commands.
pub fn render(e: &Envelope) -> Option<String> {
    match e.schema.as_str() {
        "evalsig.report/1" => Some(report(e)),
        "evalsig.compare/1" => Some(compare(e)),
        _ => None,
    }
}

fn report(e: &Envelope) -> String {
    let r = &e.results;
    let level = r["level"].as_f64().unwrap_or(0.95) * 100.0;
    let mut s = format!(
        "### evalsig report — `{}`\n\n{} judgments of {} items · estimand `{}` · {} missing\n\n",
        r["run"].as_str().unwrap_or("?"),
        r["n_judgments"],
        r["n_items"],
        r["estimand"].as_str().unwrap_or("?"),
        r["n_missing"],
    );
    let _ = writeln!(
        s,
        "| Metric | Value | {level:.0}% interval | If missing were worst / best | n |"
    );
    s.push_str("|---|---:|---|---|---:|\n");
    for m in r["metrics"].as_array().into_iter().flatten() {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} |",
            m["metric"].as_str().unwrap_or("?"),
            num(&m["value"]),
            match m["interval"].as_str() {
                Some("too_few_units") => "too few units".to_owned(),
                _ => pair(&m["ci"], num),
            },
            pair(&m["missing_bounds"], num),
            num(&m["n"]),
        );
    }
    if let Some(rep) = r["reproducibility"].as_object().filter(|m| !m.is_empty()) {
        s.push_str("\n| Replicates | Pairwise disagreement | Upper bound | Items |\n|---|---:|---:|---:|\n");
        for (kind, row) in rep {
            let _ = writeln!(
                s,
                "| {kind} | {} | {} | {} |",
                num(&row["pairwise_disagreement"]),
                num(&row["upper_bound"]),
                row["items"]
            );
        }
    }
    s.push('\n');
    footer(&mut s, e);
    s
}

fn compare_table(s: &mut String, c: &Value) {
    s.push_str("| Metric | Baseline | Candidate | Difference (better +) | Interval | Verdict |\n");
    s.push_str("|---|---:|---:|---:|---|---|\n");
    for m in c["metrics"].as_array().into_iter().flatten() {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} | `{}` |",
            m["metric"].as_str().unwrap_or("?"),
            num(&m["baseline"]),
            num(&m["candidate"]),
            signed(&m["delta"]),
            match m["interval"].as_str() {
                Some("too_few_units") => "too few units".to_owned(),
                _ => pair(&m["ci"], signed),
            },
            m["verdict"].as_str().unwrap_or("?"),
        );
    }
    if let Some(t) = c["transition"].as_object() {
        let _ = writeln!(
            s,
            "\nNew hits {} · lost hits {} · new false alarms {} · cleared false alarms {}",
            num(&t["new_hits"]),
            num(&t["lost_hits"]),
            num(&t["new_false_alarms"]),
            num(&t["cleared_false_alarms"]),
        );
    }
}

fn compare(e: &Envelope) -> String {
    let r = &e.results;
    let mut s = String::from("### evalsig compare\n\n");
    if let Some(v) = &e.verdict {
        let _ = writeln!(s, "**`{}`** — {}\n", v.code.as_str(), v.text);
    }
    match r["comparisons"].as_array() {
        None => {
            let _ = writeln!(
                s,
                "`{}` → `{}` on {} paired items ({} unpaired)\n",
                r["arms"]["baseline"].as_str().unwrap_or("?"),
                r["arms"]["candidate"].as_str().unwrap_or("?"),
                r["n_used"],
                r["n_unpaired"],
            );
            compare_table(&mut s, r);
        }
        Some(list) => {
            for c in list {
                let _ = writeln!(
                    s,
                    "#### `{}` → `{}` ({} paired items)\n",
                    c["arms"]["baseline"].as_str().unwrap_or("?"),
                    c["arms"]["candidate"].as_str().unwrap_or("?"),
                    c["n_used"],
                );
                compare_table(&mut s, c);
                s.push('\n');
            }
        }
    }
    s.push('\n');
    footer(&mut s, e);
    s
}
