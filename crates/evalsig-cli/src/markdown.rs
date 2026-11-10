//! Markdown for pull-request comments and GitHub step summaries (SPEC §12.2), for
//! the commands whose results people read in review: `report` and `compare`.

use std::fmt::Write as _;

use serde_json::Value;

use evalsig_project::envelope::{Envelope, Schema};

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
        Some("zero_variance") => "zero variance".to_owned(),
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
    match Schema::from_name(&e.schema)? {
        Schema::Report => Some(report(e)),
        Schema::Compare => Some(compare(e)),
        _ => None,
    }
}

fn metric_table(s: &mut String, metrics: &Value, level: f64) {
    let _ = writeln!(
        s,
        "| Metric | Value | {}% interval | Range if missing were worst or best | n |",
        (level * 10.0).round() / 10.0
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
        if let Some(design) = reference["rater_design"].as_str() {
            let _ = writeln!(
                s,
                "Rater design: {} ({}); the intervals estimate {}\n",
                inline(design),
                if reference["detected"] == true {
                    "detected"
                } else {
                    "set"
                },
                inline(reference["estimand"].as_str().unwrap_or("?")),
            );
        }
    }
    metric_table(&mut s, &r["metrics"], level);
    let spread: Vec<&Value> = r["metrics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| !m["rater_spread"].is_null())
        .collect();
    if !spread.is_empty() {
        s.push_str("\n| Metric | Lowest–highest with one rater as the reference |\n|---|---|\n");
        for m in spread {
            let _ = writeln!(
                s,
                "| {} | {} |",
                inline(m["metric"].as_str().unwrap_or("?")),
                pair(&m["rater_spread"], num),
            );
        }
    }
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
    if let Some(roc) = r["roc"].as_object() {
        let _ = writeln!(
            s,
            "\nAUC {} ({}, {})",
            num(&roc["auc"]),
            pair(&roc["ci"], num),
            id(&roc["interval"]),
        );
    }
    if let Some(c) = r["calibration"].as_object() {
        let _ = writeln!(s, "\nBrier score {}", num(&c["brier"]));
    }
    if let Some(ec) = r["expected_cost"].as_object() {
        let _ = writeln!(
            s,
            "\nExpected cost {} ({}, {}; {} items)",
            num(&ec["value"]),
            pair(&ec["ci"], num),
            id(&ec["interval"]),
            ec["n_used"],
        );
        if let Some(strata) = ec.get("strata").and_then(Value::as_array) {
            let _ = writeln!(
                s,
                "\nStrata combined by {} shares.\n\n| Stratum | Prevalence | Items | Expected cost | Interval |\n|---|---:|---:|---:|---|",
                id(&ec["mix"]),
            );
            for g in strata {
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} | {} |",
                    inline(g["stratum"].as_str().unwrap_or("(none)")),
                    num(&g["prevalence"]),
                    g["n"],
                    num(&g["value"]),
                    pair(&g["ci"], num),
                );
            }
        }
    }
    if let Some(p) = r["proxy"].as_object() {
        let _ = writeln!(
            s,
            "\nHuman-scale {} corrected with {}: {} ({}); {} human-labelled, {} proxy-only items, λ {}{}",
            id(&p["metric"]),
            id(&p["proxy"]),
            num(&p["estimate"]),
            pair(&p["ci"], num),
            p["labelled"],
            p["unlabelled"],
            num(&p["lambda"]),
            if p["corrected"] == false {
                " (not corrected: the human-labelled items are not known to be random)"
            } else {
                ""
            },
        );
    }
    if let Some(features) = r["features"].as_array().filter(|f| !f.is_empty()) {
        s.push_str(
            "\n| Feature | LR | Interval | Adjusted interval | Class |\n|---|---:|---|---|---|\n",
        );
        for f in features {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} |",
                inline(f["feature_id"].as_str().unwrap_or("?")),
                num(&f["lr"]),
                pair(&f["ci"], num),
                pair(&f["adjusted_ci"], num),
                id(&f["class"]),
            );
        }
    }
    if let Some(points) = r["k_curve"]["points"].as_array() {
        s.push_str("\n| Rule | k | Metric | Value | Interval |\n|---|---:|---|---:|---|\n");
        for p in points {
            for m in p["metrics"].as_array().into_iter().flatten() {
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} | {} |",
                    inline(p["rule"].as_str().unwrap_or("?")),
                    p["k"],
                    inline(m["metric"].as_str().unwrap_or("?")),
                    num(&m["value"]),
                    interval_cell(m, num),
                );
            }
        }
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
    if let Some(seq) = c["sequential"]["metrics"].as_array() {
        s.push_str("\n| Metric | Items added | Difference | Confidence sequence | Verdict |\n|---|---:|---:|---|---|\n");
        for m in seq {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} |",
                inline(m["metric"].as_str().unwrap_or("?")),
                m["n"],
                signed(&m["delta"]),
                pair(&m["confidence_sequence"], signed),
                id(&m["verdict"]),
            );
        }
    }
    if let Some(p) = c["preference"].as_object() {
        let w = &p["win_rate"];
        let _ = writeln!(
            s,
            "\nPairwise preference: candidate win rate {} ({}) over {} items from {}; left side wins {} of decided swapped judgments",
            num(&w["value"]),
            pair(&w["ci"], num),
            w["items"],
            p["raters"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| inline(r.as_str().unwrap_or("?")))
                .collect::<Vec<_>>()
                .join(", "),
            num(&p["position_bias"]["left_win_rate"]),
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
            if r["paired"] == false {
                let _ = writeln!(
                    s,
                    "{} → {} as independent samples ({} items in the two arms)\n",
                    id(&r["arms"]["baseline"]),
                    id(&r["arms"]["candidate"]),
                    r["n_used"],
                );
            } else {
                let _ = writeln!(
                    s,
                    "{} → {} on {} paired items ({} unpaired)\n",
                    id(&r["arms"]["baseline"]),
                    id(&r["arms"]["candidate"]),
                    r["n_used"],
                    r["n_unpaired"],
                );
            }
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
    if let Some(uses) = r["validation_uses"].as_u64() {
        let _ = writeln!(s, "\nComparison {uses} on this split.");
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
