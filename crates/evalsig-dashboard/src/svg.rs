//! Charts drawn on the server as SVG, from the CLI's result JSON (SPEC §11). Every
//! drawn value also sits in a `data-*` attribute and a `<title>`, written exactly as
//! the JSON writes it, so what the page shows is what the CLI printed.

use std::fmt::Write as _;

use serde_json::Value;

use crate::html::esc;

const W: f64 = 720.0;
const ROW: f64 = 28.0;
const LEFT: f64 = 180.0;
const PAD: f64 = 20.0;
/// The width kept on the right of an interval plot for its numbers.
const VALUES: f64 = 200.0;

/// A number as the JSON writes it.
fn num(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::Null => "null".into(),
        // Not expected, but never markup: whatever else arrives is escaped.
        other => esc(&other.to_string()),
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

/// The rows of a results `metrics` array: each metric's `point` field (`value` or
/// `delta`) and its `ci`.
pub fn metric_rows<'a>(metrics: &'a Value, point: &str) -> Vec<Row<'a>> {
    metrics
        .as_array()
        .into_iter()
        .flatten()
        .map(|m| Row {
            label: m["metric"].as_str().unwrap_or_default().to_owned(),
            point: &m[point],
            ci: &m["ci"],
        })
        .collect()
}

/// The value range an interval plot of `rows` spans, padded by 5% on each side; with
/// `zero`, 0 is inside it. `None` when no row has a number.
pub fn range(rows: &[Row<'_>], zero: bool) -> Option<(f64, f64)> {
    let mut lo = if zero { 0.0 } else { f64::INFINITY };
    let mut hi = if zero { 0.0 } else { f64::NEG_INFINITY };
    for r in rows {
        for x in [f(r.point), f(&r.ci[0]), f(&r.ci[1])].into_iter().flatten() {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    if hi - lo < 1e-9 {
        hi = lo + 1.0;
    }
    Some((lo - (hi - lo) * 0.05, hi + (hi - lo) * 0.05))
}

/// Each row's point and interval on one axis, with a dashed line at 0 when `zero`
/// (differences), and the numbers written beside each row.
pub fn intervals(rows: &[Row<'_>], zero: bool) -> String {
    range(rows, zero).map_or_else(String::new, |r| intervals_in(rows, zero, r))
}

/// `intervals` on a given range, so that several plots share one axis.
pub fn intervals_in(rows: &[Row<'_>], zero: bool, (lo, hi): (f64, f64)) -> String {
    let x = |v: f64| LEFT + (v - lo) / (hi - lo) * (W - LEFT - VALUES);
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
    let shown = |v: &Value| match f(v) {
        Some(n) if zero => format!("{n:+.4}"),
        Some(n) => format!("{n:.4}"),
        None => "–".to_owned(),
    };
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
        let _ = write!(
            s,
            "<text x=\"{}\" y=\"{:.1}\" font-size=\"12\" class=\"value\">{} [{}, {}]</text></g>",
            W - VALUES + 12.0,
            y + 4.0,
            shown(r.point),
            shown(&r.ci[0]),
            shown(&r.ci[1])
        );
    }
    s.push_str("</svg>");
    s
}

/// A point of a curve: where it is drawn on the unit square, and the values it reports
/// in its attributes, written as the JSON writes them.
pub struct Pt {
    pub x: f64,
    pub y: f64,
    pub shown: (String, String),
}

fn pts(points: &[(&Value, &Value)]) -> Vec<Pt> {
    points
        .iter()
        .filter_map(|(a, b)| {
            Some(Pt {
                x: f(a)?,
                y: f(b)?,
                shown: (num(a), num(b)),
            })
        })
        .collect()
}

/// A curve on the unit square, such as an ROC or reliability curve; `attrs` names the
/// two attributes that carry each point's reported values, and `diagonal` draws the
/// chance or perfect-calibration line.
pub fn curve(
    class: &str,
    points: &[Pt],
    attrs: (&str, &str),
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
        .map(|p| format!("{:.1},{:.1}", px(p.x), py(p.y)))
        .collect();
    let _ = write!(
        s,
        "<polyline points=\"{}\" fill=\"none\" stroke=\"#3b6ea5\" stroke-width=\"2\"/>",
        coords.join(" ")
    );
    for p in points {
        let (a, b) = &p.shown;
        let _ = write!(
            s,
            "<circle class=\"pt\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"2.5\" fill=\"#3b6ea5\" {}=\"{a}\" {}=\"{b}\"><title>{a}, {b}</title></circle>",
            px(p.x),
            py(p.y),
            attrs.0,
            attrs.1
        );
    }
    s.push_str("</svg>");
    s
}

/// The empirical ROC of a report's `roc` detail.
pub fn roc(roc: &Value) -> String {
    let p: Vec<(&Value, &Value)> = roc["points"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| (&p[0], &p[1]))
        .collect();
    curve(
        "roc",
        &pts(&p),
        ("data-x", "data-y"),
        true,
        ("false alarm rate", "hit rate"),
    )
}

/// The z-ROC (probit of both rates), whose straightness checks the equal-variance model.
/// Drawn on the unit square after scaling; the attributes keep the reported z values.
pub fn zroc(roc: &Value) -> String {
    let Some(z) = roc["zroc"].as_array() else {
        return String::new();
    };
    let reported = pts(&z.iter().map(|p| (&p[0], &p[1])).collect::<Vec<_>>());
    let (lo, hi) = reported
        .iter()
        .flat_map(|p| [p.x, p.y])
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| {
            (l.min(v), h.max(v))
        });
    if !lo.is_finite() {
        return String::new();
    }
    let span = (hi - lo).max(1e-9);
    let drawn: Vec<Pt> = reported
        .into_iter()
        .map(|p| Pt {
            x: (p.x - lo) / span,
            y: (p.y - lo) / span,
            shown: p.shown,
        })
        .collect();
    curve(
        "zroc",
        &drawn,
        ("data-zx", "data-zy"),
        true,
        ("z(false alarm rate)", "z(hit rate)"),
    )
}

/// A reliability diagram from a report's `calibration` detail.
pub fn reliability(calibration: &Value) -> String {
    let p: Vec<(&Value, &Value)> = calibration["bins"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| (&b["mean_score"], &b["positive_share"]))
        .collect();
    curve(
        "reliability",
        &pts(&p),
        ("data-x", "data-y"),
        true,
        ("mean score", "share positive"),
    )
}

/// The four cells of a comparison's transition table, as an HTML table.
pub fn transition(t: &Value) -> String {
    let cell = |k: &str| format!("<td class=\"n\" data-key=\"{k}\">{}</td>", num(&t[k]));
    format!(
        "<table class=\"transition\"><thead><tr><th></th><th class=\"n\">Gained</th><th class=\"n\">Lost</th></tr></thead><tbody><tr><th>Hits</th>{}{}</tr><tr><th>False alarms</th>{}{}</tr></tbody></table>",
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
        let rows = metric_rows(&metrics, "value");
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
