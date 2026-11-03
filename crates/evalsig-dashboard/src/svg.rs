//! Charts drawn on the server as SVG, from the CLI's result JSON (SPEC §11). Every
//! drawn value also sits in a `data-*` attribute and a `<title>`, written exactly as
//! the JSON writes it, so what the page shows is what the CLI printed.

use std::fmt::Write as _;

use serde_json::Value;

use crate::html::esc;

const W: f64 = 640.0;
const ROW: f64 = 28.0;
const LEFT: f64 = 180.0;
const PAD: f64 = 20.0;

/// A number as the JSON writes it.
fn num(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::Null => "null".into(),
        other => other.to_string(),
    }
}

fn f(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// One row of an interval plot: a label, a point and its interval.
pub struct Row<'a> {
    pub label: String,
    pub point: &'a Value,
    pub ci: &'a Value,
}

/// Points with their intervals on one axis; `zero` draws the line at 0 (differences).
pub fn intervals(rows: &[Row<'_>], zero: bool) -> String {
    let mut lo = if zero { 0.0 } else { f64::INFINITY };
    let mut hi = if zero { 0.0 } else { f64::NEG_INFINITY };
    for r in rows {
        for x in [f(r.point), f(&r.ci[0]), f(&r.ci[1])].into_iter().flatten() {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return String::new();
    }
    if hi - lo < 1e-9 {
        hi = lo + 1.0;
    }
    let (lo, hi) = (lo - (hi - lo) * 0.05, hi + (hi - lo) * 0.05);
    let x = |v: f64| LEFT + (v - lo) / (hi - lo) * (W - LEFT - PAD);
    let h = ROW * rows.len() as f64 + PAD;
    let mut s = format!(
        "<svg class=\"intervals\" viewBox=\"0 0 {W} {h}\" width=\"{W}\" height=\"{h}\" role=\"img\">"
    );
    if zero {
        let z = x(0.0);
        let _ = write!(
            s,
            "<line x1=\"{z:.1}\" y1=\"0\" x2=\"{z:.1}\" y2=\"{h}\" stroke=\"#999\" stroke-dasharray=\"3 3\"/>"
        );
    }
    for (i, r) in rows.iter().enumerate() {
        let y = PAD / 2.0 + ROW * i as f64 + ROW / 2.0;
        let label = esc(&r.label);
        let _ = write!(
            s,
            "<g class=\"row\" data-label=\"{label}\" data-value=\"{}\" data-lo=\"{}\" data-hi=\"{}\"><title>{label}: {} [{}, {}]</title><text x=\"{}\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"12\">{label}</text>",
            num(r.point),
            num(&r.ci[0]),
            num(&r.ci[1]),
            num(r.point),
            num(&r.ci[0]),
            num(&r.ci[1]),
            LEFT - 8.0,
            y + 4.0
        );
        if let (Some(a), Some(b)) = (f(&r.ci[0]), f(&r.ci[1])) {
            let _ = write!(
                s,
                "<line x1=\"{:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" stroke=\"#3b6ea5\" stroke-width=\"2\"/>",
                x(a),
                x(b)
            );
        }
        if let Some(p) = f(r.point) {
            let _ = write!(
                s,
                "<circle cx=\"{:.1}\" cy=\"{y:.1}\" r=\"4\" fill=\"#3b6ea5\"/>",
                x(p)
            );
        }
        s.push_str("</g>");
    }
    s.push_str("</svg>");
    s
}

/// A curve on the unit square, such as an ROC or reliability curve: `points` are
/// `[x, y]` pairs; `diagonal` draws the chance or perfect-calibration line.
pub fn curve(
    class: &str,
    points: &[(&Value, &Value)],
    diagonal: bool,
    axes: (&str, &str),
) -> String {
    const S: f64 = 300.0;
    let m = 40.0;
    let px = |v: f64| m + v * S;
    let py = |v: f64| m + (1.0 - v) * S;
    let size = S + 2.0 * m;
    let mut s = format!(
        "<svg class=\"{}\" viewBox=\"0 0 {size} {size}\" width=\"{size}\" height=\"{size}\" role=\"img\"><rect x=\"{m}\" y=\"{m}\" width=\"{S}\" height=\"{S}\" fill=\"none\" stroke=\"#ccc\"/>",
        esc(class)
    );
    if diagonal {
        let _ = write!(
            s,
            "<line x1=\"{m}\" y1=\"{}\" x2=\"{}\" y2=\"{m}\" stroke=\"#bbb\" stroke-dasharray=\"3 3\"/>",
            m + S,
            m + S
        );
    }
    let _ = write!(
        s,
        "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-size=\"12\">{}</text><text x=\"12\" y=\"{}\" font-size=\"12\" transform=\"rotate(-90 12 {})\" text-anchor=\"middle\">{}</text>",
        m + S / 2.0,
        size - 8.0,
        esc(axes.0),
        m + S / 2.0,
        m + S / 2.0,
        esc(axes.1)
    );
    let coords: Vec<String> = points
        .iter()
        .filter_map(|(a, b)| Some(format!("{:.1},{:.1}", px(f(a)?), py(f(b)?))))
        .collect();
    let _ = write!(
        s,
        "<polyline points=\"{}\" fill=\"none\" stroke=\"#3b6ea5\" stroke-width=\"2\"/>",
        coords.join(" ")
    );
    for (a, b) in points {
        if let (Some(x), Some(y)) = (f(a), f(b)) {
            let _ = write!(
                s,
                "<circle class=\"pt\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"2.5\" fill=\"#3b6ea5\" data-x=\"{}\" data-y=\"{}\"><title>{}, {}</title></circle>",
                px(x),
                py(y),
                num(a),
                num(b),
                num(a),
                num(b)
            );
        }
    }
    s.push_str("</svg>");
    s
}

/// The empirical ROC of a report's `roc` detail.
pub fn roc(roc: &Value) -> String {
    let pts: Vec<(&Value, &Value)> = roc["points"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| (&p[0], &p[1]))
        .collect();
    curve("roc", &pts, true, ("false alarm rate", "hit rate"))
}

/// The z-ROC (probit of both rates), whose straightness checks the equal-variance model.
pub fn zroc(roc: &Value) -> String {
    let Some(z) = roc["zroc"].as_array() else {
        return String::new();
    };
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in z {
        for v in [f(&p[0]), f(&p[1])].into_iter().flatten() {
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if !lo.is_finite() {
        return String::new();
    }
    // Drawn on the unit square after scaling; the data attributes keep the z values.
    let span = (hi - lo).max(1e-9);
    let scaled: Vec<(Value, Value)> = z
        .iter()
        .map(|p| {
            (
                Value::from(f(&p[0]).map_or(0.0, |v| (v - lo) / span)),
                Value::from(f(&p[1]).map_or(0.0, |v| (v - lo) / span)),
            )
        })
        .collect();
    let mut s = curve(
        "zroc",
        &scaled.iter().map(|(a, b)| (a, b)).collect::<Vec<_>>(),
        true,
        ("z(false alarm rate)", "z(hit rate)"),
    );
    // Replace the scaled coordinates in the attributes with the reported z values.
    for (p, (a, b)) in z.iter().zip(&scaled) {
        s = s.replacen(
            &format!("data-x=\"{}\" data-y=\"{}\"", num(a), num(b)),
            &format!("data-zx=\"{}\" data-zy=\"{}\"", num(&p[0]), num(&p[1])),
            1,
        );
    }
    s
}

/// A reliability diagram from a report's `calibration` detail.
pub fn reliability(calibration: &Value) -> String {
    let pts: Vec<(&Value, &Value)> = calibration["bins"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| (&b["mean_score"], &b["positive_share"]))
        .collect();
    curve("reliability", &pts, true, ("mean score", "share positive"))
}

/// The four cells of a comparison's transition table, as an HTML table.
pub fn transition(t: &Value) -> String {
    let cell = |k: &str| format!("<td class=\"n\" data-key=\"{k}\">{}</td>", num(&t[k]));
    format!(
        "<table class=\"transition\"><thead><tr><th></th><th>Gained</th><th>Lost</th></tr></thead><tbody><tr><th>Hits</th>{}{}</tr><tr><th>False alarms</th>{}{}</tr></tbody></table>",
        cell("new_hits"),
        cell("lost_hits"),
        cell("new_false_alarms"),
        cell("cleared_false_alarms")
    )
}

/// A trend: the primary metric's value and interval in each period.
pub fn trend(periods: &Value, metric: &str) -> String {
    let rows: Vec<Row<'_>> = periods
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let m = p["metrics"]
                .as_array()?
                .iter()
                .find(|m| m["metric"] == metric)?;
            Some(Row {
                label: p["period"].as_str().unwrap_or_default().to_owned(),
                point: &m["value"],
                ci: &m["ci"],
            })
        })
        .collect();
    intervals(&rows, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn drawn_values_are_the_json_values() {
        let metrics =
            json!([{"metric": "accuracy", "value": 0.7583333333, "ci": [0.7133333333, 0.805]}]);
        let rows: Vec<Row<'_>> = metrics
            .as_array()
            .unwrap()
            .iter()
            .map(|m| Row {
                label: m["metric"].as_str().unwrap().into(),
                point: &m["value"],
                ci: &m["ci"],
            })
            .collect();
        let s = intervals(&rows, false);
        assert!(
            s.contains("data-value=\"0.7583333333\" data-lo=\"0.7133333333\" data-hi=\"0.805\"")
        );
        let r = roc(&json!({"points": [[0.0, 0.0], [0.25, 0.7], [1.0, 1.0]]}));
        assert!(r.contains("data-x=\"0.25\" data-y=\"0.7\""));
        let t = transition(
            &json!({"new_hits": 9.0, "lost_hits": 6.5, "new_false_alarms": 7.0, "cleared_false_alarms": 18.5}),
        );
        assert!(t.contains("data-key=\"lost_hits\">6.5<"));
        let c = reliability(
            &json!({"brier": 0.1, "bins": [{"bin": 0, "n": 10, "mean_score": 0.15, "positive_share": 0.2}]}),
        );
        assert!(c.contains("data-x=\"0.15\" data-y=\"0.2\""));
        let z = zroc(&json!({"zroc": [[-1.2, -0.4], [0.3, 0.9]]}));
        assert!(z.contains("data-zx=\"-1.2\" data-zy=\"-0.4\""));
        // Labels from the data are escaped.
        let x = intervals(
            &[Row {
                label: "<script>".into(),
                point: &json!(1.0),
                ci: &json!([0.5, 1.5]),
            }],
            true,
        );
        assert!(!x.contains("<script>"));
    }
}
