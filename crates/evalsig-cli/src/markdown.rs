//! Markdown for pull-request comments and GitHub step summaries (SPEC §12.2), for
//! the commands whose results people read in review: `report` and `compare`.

use std::fmt::Write as _;

use serde_json::Value;

use crate::output::Envelope;

/// A metric value with four decimals.
fn num(v: &Value) -> String {
    v.as_f64()
        .map_or_else(|| "—".to_owned(), |x| format!("{x:.4}"))
}

/// A signed difference with four decimals.
fn signed(v: &Value) -> String {
    v.as_f64()
        .map_or_else(|| "—".to_owned(), |x| format!("{x:+.4}"))
}

/// A count: whole numbers without decimals, weighted counts with up to two.
fn count(v: &Value) -> String {
    match v.as_f64() {
        Some(x) if x.fract() == 0.0 => format!("{x:.0}"),
        Some(x) => format!("{x:.2}"),
        None => "—".to_owned(),
    }
}

fn pair(v: &Value, f: fn(&Value) -> String) -> String {
    match v.as_array() {
        Some(a) if a.len() == 2 => format!("{} to {}", f(&a[0]), f(&a[1])),
        _ => "—".to_owned(),
    }
}

/// An inline code span whose fence is longer than any backtick run in the text, with
/// padding when the text starts or ends with a backtick (CommonMark).
pub fn code(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!(
        "{fence}{pad}{}{pad}{fence}",
        text.replace(['\n', '\r'], " ")
    )
}

/// Text placed in a table cell or on one line: no line breaks, pipes escaped.
fn inline(text: &str) -> String {
    text.replace(['\n', '\r'], " ").replace('|', "\\|")
}

fn id(v: &Value) -> String {
    code(v.as_str().unwrap_or("?"))
}

fn interval_cell(m: &Value, f: fn(&Value) -> String) -> String {
    match m["interval"].as_str() {
        Some("too_few_units") => "too few units".to_owned(),
        _ => pair(&m["ci"], f),
    }
}

fn footer(s: &mut String, e: &Envelope) {
    for w in &e.warnings {
        let _ = writeln!(s, "> **Warning** {}: {}", code(&w.code), inline(&w.message));
    }
    if !e.warnings.is_empty() {
        s.push('\n');
    }
    for n in &e.next {
        let _ = writeln!(s, "Next: {} — {}", code(&n.command), inline(&n.why));
    }
    let _ = writeln!(
        s,
        "\n<sub>{} · {} snapshot(s) · config {} · seed {}</sub>",
        e.provenance.tool,
        e.provenance.snapshots.len(),
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

fn metric_table(s: &mut String, metrics: &Value, level: f64) {
    let _ = writeln!(
        s,
        "| Metric | Value | {level:.0}% interval | If missing were worst / best | n |"
    );
    s.push_str("|---|---:|---|---|---:|\n");
    for m in metrics.as_array().into_iter().flatten() {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} |",
            inline(m["metric"].as_str().unwrap_or("?")),
            num(&m["value"]),
            interval_cell(m, num),
            pair(&m["missing_bounds"], num),
            count(&m["n"]),
        );
    }
}

fn report(e: &Envelope) -> String {
    let r = &e.results;
    let level = r["level"].as_f64().unwrap_or(0.95) * 100.0;
    let mut s = format!(
        "### evalsig report — {}\n\n{} judgments of {} items · estimand {} · {} missing\n\n",
        id(&r["run"]),
        r["n_judgments"],
        r["n_items"],
        id(&r["estimand"]),
        r["n_missing"],
    );
    if let Some(reference) = r["reference"].as_object() {
        let _ = writeln!(
            s,
            "Reference: {} from {} raters; {} items with a reference ({} model-derived)\n",
            id(&reference["mode"]),
            reference["raters"],
            reference["items_with_reference"],
            reference["model_derived_items"],
        );
    }
    metric_table(&mut s, &r["metrics"], level);
    if let Some(by) = r["by"].as_object() {
        for (stratum, metrics) in by {
            let _ = writeln!(s, "\n**{}**\n", inline(stratum));
            metric_table(&mut s, metrics, level);
        }
    }
    if let Some(rep) = r["reproducibility"].as_object().filter(|m| !m.is_empty()) {
        s.push_str("\n| Replicates | Pairwise disagreement | Upper bound | Items |\n|---|---:|---:|---:|\n");
        for (kind, row) in rep {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} |",
                inline(kind),
                num(&row["pairwise_disagreement"]),
                num(&row["upper_bound"]),
                row["items"]
            );
        }
    }
    if r["n_insufficient_replicates"]
        .as_u64()
        .is_some_and(|n| n > 0)
    {
        let _ = writeln!(
            s,
            "\n{} items have fewer replicates than the estimand needs and are left out.",
            r["n_insufficient_replicates"]
        );
    }
    if let Some(items) = r["items"].as_array() {
        let _ = writeln!(
            s,
            "\n{} item rows are in the JSON output (`--format json`).",
            items.len()
        );
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
            "| {} | {} | {} | {} | {} | {} |",
            inline(m["metric"].as_str().unwrap_or("?")),
            num(&m["baseline"]),
            num(&m["candidate"]),
            signed(&m["delta"]),
            interval_cell(m, signed),
            id(&m["verdict"]),
        );
    }
    if let Some(t) = c["transition"].as_object() {
        let _ = writeln!(
            s,
            "\nNew hits {} · lost hits {} · new false alarms {} · cleared false alarms {}",
            count(&t["new_hits"]),
            count(&t["lost_hits"]),
            count(&t["new_false_alarms"]),
            count(&t["cleared_false_alarms"]),
        );
    }
}

fn compare(e: &Envelope) -> String {
    let r = &e.results;
    let mut s = String::from("### evalsig compare\n\n");
    if let Some(v) = &e.verdict {
        let _ = writeln!(s, "**{}** — {}\n", code(v.code.as_str()), inline(&v.text));
    }
    match r["comparisons"].as_array() {
        None => {
            let _ = writeln!(
                s,
                "{} → {} on {} paired items ({} unpaired)\n",
                id(&r["arms"]["baseline"]),
                id(&r["arms"]["candidate"]),
                r["n_used"],
                r["n_unpaired"],
            );
            compare_table(&mut s, r);
        }
        Some(list) => {
            for c in list {
                let _ = writeln!(
                    s,
                    "#### {} → {} ({} paired items)\n",
                    id(&c["arms"]["baseline"]),
                    id(&c["arms"]["candidate"]),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_spans_outgrow_their_content() {
        assert_eq!(code("run-1"), "`run-1`");
        assert_eq!(code("a``b"), "```a``b```");
        assert_eq!(code("`x"), "`` `x ``");
        assert_eq!(inline("a|b\nc"), "a\\|b c");
    }
}
