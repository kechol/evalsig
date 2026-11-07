//! Charts drawn on the server as SVG, from the CLI's result JSON (SPEC §11). Every
//! drawn value also sits in a `data-*` attribute and a `<title>`, written exactly as
//! the JSON writes it, so what the page shows is what the CLI printed.

use std::fmt::Write as _;

use evalsig_core::numeric::{norm_cdf, norm_pdf};
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
            "<line x1=\"{z:.1}\" y1=\"0\" x2=\"{z:.1}\" y2=\"{h}\" class=\"guide\" stroke=\"#999\" stroke-dasharray=\"3 3\"/>"
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
                "<line x1=\"{:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" class=\"mark\" stroke=\"#3b6ea5\" stroke-width=\"2\"/>",
                x(a),
                x(b)
            );
        }
        if let Some(p) = f(r.point) {
            let _ = write!(
                s,
                "<circle cx=\"{:.1}\" cy=\"{y:.1}\" r=\"4\" class=\"mark\" fill=\"#3b6ea5\"/>",
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

/// A curve on a square of `range` on both axes, such as an ROC or reliability curve,
/// with ticks; `attrs` names the two attributes that carry each point's reported values,
/// and `diagonal` draws the chance or perfect-calibration line.
pub fn curve(
    class: &str,
    points: &[Pt],
    attrs: (&str, &str),
    diagonal: bool,
    axes: (&str, &str),
    range: (f64, f64),
) -> String {
    let fr = Frame {
        x: range,
        y: range,
        left: 56.0,
        top: 12.0,
        w: 300.0,
        h: 300.0,
    };
    let (width, height) = (380.0, fr.top + fr.h + 44.0);
    let mut s = format!(
        "<svg class=\"{}\" viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" role=\"img\">",
        esc(class)
    );
    let t = ticks(range.0, range.1, 5);
    fr.axes(&mut s, &t, &t, axes);
    if diagonal {
        let _ = write!(
            s,
            "<line class=\"chance\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
            fr.px(range.0),
            fr.py(range.0),
            fr.px(range.1),
            fr.py(range.1)
        );
    }
    let line: Vec<(f64, f64)> = points.iter().map(|p| (p.x, p.y)).collect();
    let _ = write!(
        s,
        "<g class=\"s1\"><polyline class=\"line\" points=\"{}\"/>",
        fr.path(&line)
    );
    for p in points {
        let (a, b) = &p.shown;
        let _ = write!(
            s,
            "<circle class=\"pt dot\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"3\" {}=\"{a}\" {}=\"{b}\"><title>{a}, {b}</title></circle>",
            fr.px(p.x),
            fr.py(p.y),
            attrs.0,
            attrs.1
        );
    }
    s.push_str("</g></svg>");
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
        (0.0, 1.0),
    )
}

/// The z-ROC (probit of both rates), whose straightness checks the equal-variance model:
/// a straight line of slope 1 is the model d′ assumes. Drawn in z units on both axes.
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
    let pad = ((hi - lo) * 0.1).max(0.25);
    curve(
        "zroc",
        &reported,
        ("data-zx", "data-zy"),
        true,
        ("z(false alarm rate)", "z(hit rate)"),
        ((lo - pad).floor(), (hi + pad).ceil()),
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
        (0.0, 1.0),
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

/// A plotting area: data ranges mapped onto a rectangle of the drawing.
struct Frame {
    x: (f64, f64),
    y: (f64, f64),
    left: f64,
    top: f64,
    w: f64,
    h: f64,
}

impl Frame {
    fn px(&self, v: f64) -> f64 {
        self.left + (v - self.x.0) / (self.x.1 - self.x.0) * self.w
    }

    fn py(&self, v: f64) -> f64 {
        self.top + (1.0 - (v - self.y.0) / (self.y.1 - self.y.0)) * self.h
    }

    /// Drawing coordinates of data points, for `points="…"`.
    fn path(&self, pts: &[(f64, f64)]) -> String {
        pts.iter()
            .map(|&(x, y)| format!("{:.1},{:.1}", self.px(x), self.py(y)))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The axes with their ticks and titles; an empty tick list leaves that axis out.
    fn axes(&self, s: &mut String, xt: &[f64], yt: &[f64], titles: (&str, &str)) {
        let (x0, x1, y0, y1) = (self.left, self.left + self.w, self.top, self.top + self.h);
        let _ = write!(
            s,
            "<line class=\"axis\" x1=\"{x0}\" y1=\"{y1}\" x2=\"{x1}\" y2=\"{y1}\"/>"
        );
        for &t in xt {
            let x = self.px(t);
            let _ = write!(
                s,
                "<line class=\"axis\" x1=\"{x:.1}\" y1=\"{y1}\" x2=\"{x:.1}\" y2=\"{}\"/><text class=\"tick\" x=\"{x:.1}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
                y1 + 4.0,
                y1 + 16.0,
                tick_label(t)
            );
        }
        if !yt.is_empty() {
            let _ = write!(
                s,
                "<line class=\"axis\" x1=\"{x0}\" y1=\"{y0}\" x2=\"{x0}\" y2=\"{y1}\"/>"
            );
        }
        for &t in yt {
            let y = self.py(t);
            let _ = write!(
                s,
                "<line class=\"grid\" x1=\"{x0}\" y1=\"{y:.1}\" x2=\"{x1}\" y2=\"{y:.1}\"/><text class=\"tick\" x=\"{}\" y=\"{:.1}\" text-anchor=\"end\">{}</text>",
                x0 - 6.0,
                y + 4.0,
                tick_label(t)
            );
        }
        let _ = write!(
            s,
            "<text class=\"axis-title\" x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\">{}</text>",
            (x0 + x1) / 2.0,
            y1 + 32.0,
            esc(titles.0)
        );
        if !titles.1.is_empty() {
            let (cx, cy) = (x0 - 34.0, (y0 + y1) / 2.0);
            let _ = write!(
                s,
                "<text class=\"axis-title\" x=\"{cx}\" y=\"{cy:.1}\" text-anchor=\"middle\" transform=\"rotate(-90 {cx} {cy:.1})\">{}</text>",
                esc(titles.1)
            );
        }
    }
}

/// Round tick values covering `lo..=hi`, about `n` of them.
fn ticks(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    let raw = (hi - lo) / n.max(1) as f64;
    if raw.is_nan() || raw <= 0.0 || raw.is_infinite() {
        return vec![lo];
    }
    let e = 10f64.powf(raw.log10().floor());
    let step = e * match raw / e {
        f if f < 1.5 => 1.0,
        f if f < 3.0 => 2.0,
        f if f < 7.0 => 5.0,
        _ => 10.0,
    };
    let first = (lo / step).ceil() as i64;
    let last = (hi / step).floor() as i64;
    (first..=last).map(|i| i as f64 * step).collect()
}

fn tick_label(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

/// The class of the `i`th series, which `style.css` colours.
fn series(i: usize) -> &'static str {
    ["s1", "s2", "s3"][i % 3]
}

/// A legend row inside a chart: a swatch of `class` and its text.
fn legend(s: &mut String, x: f64, y: f64, class: &str, text: &str) {
    let _ = write!(
        s,
        "<rect class=\"{class}\" x=\"{x}\" y=\"{}\" width=\"10\" height=\"10\" rx=\"2\"/><text class=\"legend\" x=\"{}\" y=\"{y}\">{}</text>",
        y - 9.0,
        x + 16.0,
        esc(text)
    );
}

/// The equal-variance normal model behind d′ and c (metrics spec §2): noise N(0, 1) for
/// the reference negatives and signal N(d′, 1) for the positives on one decision axis,
/// the criterion at d′/2 + c, and the areas beyond it shaded: under the signal the hits,
/// under the noise the false alarms. Empty without both numbers.
pub fn sdt_model(sdt: &Value) -> String {
    let (Some(d), Some(c)) = (f(&sdt["dprime"]), f(&sdt["criterion"])) else {
        return String::new();
    };
    let k = d / 2.0 + c;
    let lo = (-3.5f64).min(d - 3.5).min(k - 0.5).floor();
    let hi = 3.5f64.max(d + 3.5).max(k + 0.5).ceil();
    let fr = Frame {
        x: (lo, hi),
        y: (0.0, 0.56),
        left: 20.0,
        top: 16.0,
        w: 640.0,
        h: 180.0,
    };
    let (width, height) = (680.0, 270.0);
    const N: usize = 160;
    let xs: Vec<f64> = (0..=N)
        .map(|i| lo + (hi - lo) * i as f64 / N as f64)
        .collect();
    let density =
        |mu: f64| -> Vec<(f64, f64)> { xs.iter().map(|&x| (x, norm_pdf(x - mu))).collect() };
    // The area under a density beyond the criterion, closed along the axis.
    let beyond = |mu: f64| -> String {
        let mut pts = vec![(k, 0.0), (k, norm_pdf(k - mu))];
        pts.extend(
            xs.iter()
                .filter(|&&x| x > k)
                .map(|&x| (x, norm_pdf(x - mu))),
        );
        pts.push((hi, 0.0));
        fr.path(&pts)
    };
    let (dv, cv) = (num(&sdt["dprime"]), num(&sdt["criterion"]));
    let mut s = format!(
        "<svg class=\"sdt-model\" viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" role=\"img\" data-dprime=\"{dv}\" data-criterion=\"{cv}\"><title>Equal-variance model: d′ {dv}, criterion {cv}</title>"
    );
    // The false alarms lie under the hits wherever d′ > 0, so they are drawn on top.
    let _ = write!(
        s,
        "<polygon class=\"area positive\" points=\"{}\"/><polygon class=\"area negative\" points=\"{}\"/>",
        beyond(d),
        beyond(0.0)
    );
    let _ = write!(
        s,
        "<polyline class=\"line s0\" points=\"{}\"/><polyline class=\"line s1\" points=\"{}\"/>",
        fr.path(&density(0.0)),
        fr.path(&density(d))
    );
    // The two means and the distance between them.
    let top = fr.py(0.52);
    let (m0, m1) = (fr.px(0.0), fr.px(d));
    let _ = write!(
        s,
        "<line class=\"bracket\" x1=\"{m0:.1}\" y1=\"{top:.1}\" x2=\"{m1:.1}\" y2=\"{top:.1}\"/><line class=\"bracket\" x1=\"{m0:.1}\" y1=\"{:.1}\" x2=\"{m0:.1}\" y2=\"{:.1}\"/><line class=\"bracket\" x1=\"{m1:.1}\" y1=\"{:.1}\" x2=\"{m1:.1}\" y2=\"{:.1}\"/><text class=\"note\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">d′ = {d:.4}</text>",
        top - 4.0,
        top + 4.0,
        top - 4.0,
        top + 4.0,
        (m0 + m1) / 2.0,
        top - 6.0
    );
    let kx = fr.px(k);
    let _ = write!(
        s,
        "<line class=\"criterion\" x1=\"{kx:.1}\" y1=\"{:.1}\" x2=\"{kx:.1}\" y2=\"{:.1}\"/><text class=\"note\" x=\"{:.1}\" y=\"{:.1}\">criterion, c = {c:.4}</text>",
        fr.py(0.45),
        fr.py(0.0),
        kx + 4.0,
        fr.py(0.43)
    );
    fr.axes(
        &mut s,
        &ticks(lo, hi, 8),
        &[],
        ("decision axis (standard deviations)", ""),
    );
    let y = height - 14.0;
    legend(&mut s, 20.0, y, "swatch s0", "reference negative (noise)");
    legend(&mut s, 200.0, y, "swatch s1", "reference positive (signal)");
    legend(&mut s, 380.0, y, "area positive", "hits");
    legend(&mut s, 450.0, y, "area negative", "false alarms");
    s.push_str("</svg>");
    s
}

/// The rates (F′, H′) that d′ and c fix under the equal-variance model: the criterion
/// sits at d′/2 + c, so F′ = 1 − Φ(d′/2 + c) and H′ = 1 − Φ(c − d′/2).
fn model_rates(d: f64, c: f64) -> (f64, f64) {
    let k = d / 2.0 + c;
    (1.0 - norm_cdf(k), 1.0 - norm_cdf(k - d))
}

/// One run in ROC space: its label and its `sdt` detail as reported.
pub struct RocRun<'a> {
    pub label: String,
    pub sdt: &'a Value,
}

/// ROC space for binary decisions: each run at its corrected rates (F′, H′), which the
/// reported d′ and c fix, on the equal-variance ROC of its d′ (every criterion at the
/// same sensitivity), with the chance line. A run moved along its curve changed its
/// criterion; a run on a higher curve separates the classes better.
pub fn roc_space(runs: &[RocRun<'_>]) -> String {
    let runs: Vec<(&RocRun, f64, f64)> = runs
        .iter()
        .filter_map(|r| Some((r, f(&r.sdt["dprime"])?, f(&r.sdt["criterion"])?)))
        .collect();
    if runs.is_empty() {
        return String::new();
    }
    let fr = Frame {
        x: (0.0, 1.0),
        y: (0.0, 1.0),
        left: 56.0,
        top: 12.0,
        w: 300.0,
        h: 300.0,
    };
    let width = 380.0;
    let height = fr.top + fr.h + 44.0 + 20.0 * runs.len() as f64;
    let mut s = format!(
        "<svg class=\"roc-space\" viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" role=\"img\"><title>ROC space</title>"
    );
    let grid = ticks(0.0, 1.0, 5);
    fr.axes(&mut s, &grid, &grid, ("false alarm rate", "hit rate"));
    let _ = write!(
        s,
        "<line class=\"chance\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
        fr.px(0.0),
        fr.py(0.0),
        fr.px(1.0),
        fr.py(1.0)
    );
    for (i, (r, d, c)) in runs.iter().enumerate() {
        // The criterion sweeps the decision axis; F = 1 − Φ(λ), H = 1 − Φ(λ − d′).
        let iso: Vec<(f64, f64)> = (0..=200)
            .map(|j| {
                let l = -6.0 + (d.max(0.0) + 12.0) * j as f64 / 200.0;
                (1.0 - norm_cdf(l), 1.0 - norm_cdf(l - d))
            })
            .collect();
        let (fp, hp) = model_rates(*d, *c);
        let (dv, cv) = (num(&r.sdt["dprime"]), num(&r.sdt["criterion"]));
        let label = esc(&r.label);
        let _ = write!(
            s,
            "<g class=\"{}\"><polyline class=\"line\" points=\"{}\"/><circle class=\"dot\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"5\" data-label=\"{label}\" data-dprime=\"{dv}\" data-criterion=\"{cv}\"><title>{label}: d′ {dv}, c {cv}</title></circle></g>",
            series(i),
            fr.path(&iso),
            fr.px(fp),
            fr.py(hp)
        );
        legend(
            &mut s,
            fr.left,
            fr.top + fr.h + 52.0 + 20.0 * i as f64,
            &format!("swatch {}", series(i)),
            &format!("{}: d′ {d:.4}, c {c:.4}", r.label),
        );
    }
    s.push_str("</svg>");
    s
}

/// One bar: a label, its value as reported, and its series (`s0` for people, `s1` for
/// systems and graders).
pub struct Bar<'a> {
    pub label: String,
    pub value: &'a Value,
    pub series: &'static str,
}

/// Horizontal bars from 0 on the axis `range`, each value written beside its bar, with a
/// dashed reference line and its label.
pub fn bars(rows: &[Bar<'_>], range: (f64, f64), reference: Option<(f64, &str)>) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let fr = Frame {
        x: range,
        y: (0.0, 1.0),
        left: LEFT,
        top: 8.0,
        w: W - LEFT - VALUES,
        h: ROW * rows.len() as f64,
    };
    let height = fr.top + fr.h + 44.0;
    let mut s = format!(
        "<svg class=\"bars\" viewBox=\"0 0 {W} {height}\" width=\"{W}\" height=\"{height}\" role=\"img\">"
    );
    let x0 = fr.px(0f64.clamp(range.0, range.1));
    for (i, b) in rows.iter().enumerate() {
        let y = fr.top + ROW * i as f64;
        let label = esc(&b.label);
        let shown = f(b.value).map_or_else(|| "–".to_owned(), |v| format!("{v:.4}"));
        let _ = write!(
            s,
            "<g class=\"{}\" data-label=\"{label}\" data-value=\"{}\"><title>{label}: {}</title><text x=\"{}\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"12\">{label}</text>",
            b.series,
            num(b.value),
            num(b.value),
            LEFT - 8.0,
            y + ROW / 2.0 + 4.0
        );
        if let Some(v) = f(b.value) {
            let x1 = fr.px(v.clamp(range.0, range.1));
            let _ = write!(
                s,
                "<rect class=\"bar\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"2\"/>",
                x0.min(x1),
                y + 6.0,
                (x1 - x0).abs(),
                ROW - 12.0
            );
        }
        let _ = write!(
            s,
            "<text class=\"value\" x=\"{}\" y=\"{:.1}\" font-size=\"12\">{shown}</text></g>",
            W - VALUES + 12.0,
            y + ROW / 2.0 + 4.0
        );
    }
    if let Some((v, text)) = reference {
        let x = fr.px(v);
        let _ = write!(
            s,
            "<line class=\"criterion\" x1=\"{x:.1}\" y1=\"{}\" x2=\"{x:.1}\" y2=\"{:.1}\"/><text class=\"note\" x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text>",
            fr.top,
            fr.top + fr.h,
            fr.top + fr.h + 40.0,
            esc(text)
        );
    }
    fr.axes(&mut s, &ticks(range.0, range.1, 5), &[], ("", ""));
    s.push_str("</svg>");
    s
}

/// One line of a line chart: its label and its points (x, value, interval) as reported.
pub struct Line<'a> {
    pub label: String,
    pub points: Vec<(f64, &'a Value, &'a Value)>,
}

/// Lines over a numeric x, each with its interval as a band, the axes titled `titles`.
pub fn lines(series_: &[Line<'_>], titles: (&str, &str)) -> String {
    let all: Vec<(f64, f64)> = series_
        .iter()
        .flat_map(|l| l.points.iter())
        .flat_map(|(x, v, ci)| {
            [f(v), f(&ci[0]), f(&ci[1])]
                .into_iter()
                .flatten()
                .map(move |y| (*x, y))
        })
        .collect();
    if all.is_empty() {
        return String::new();
    }
    let fold = |g: fn(&(f64, f64)) -> f64| {
        all.iter()
            .map(g)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| {
                (l.min(v), h.max(v))
            })
    };
    let (xl, xh) = fold(|p| p.0);
    let (yl, yh) = fold(|p| p.1);
    let pad = ((yh - yl) * 0.1).max(0.01);
    let fr = Frame {
        x: (xl, if xh > xl { xh } else { xl + 1.0 }),
        y: (yl - pad, yh + pad),
        left: 64.0,
        top: 12.0,
        w: 560.0,
        h: 220.0,
    };
    let width = 680.0;
    let height = fr.top + fr.h + 44.0 + 20.0 * series_.len() as f64;
    let mut s = format!(
        "<svg class=\"lines\" viewBox=\"0 0 {width} {height}\" width=\"{width}\" height=\"{height}\" role=\"img\">"
    );
    let xt: Vec<f64> = if xh - xl <= 12.0 {
        (xl as i64..=xh as i64).map(|x| x as f64).collect()
    } else {
        ticks(xl, xh, 8)
    };
    fr.axes(&mut s, &xt, &ticks(fr.y.0, fr.y.1, 5), titles);
    for (i, l) in series_.iter().enumerate() {
        let band: Vec<(f64, f64, f64)> = l
            .points
            .iter()
            .filter_map(|(x, _, ci)| Some((*x, f(&ci[0])?, f(&ci[1])?)))
            .collect();
        let mut outline: Vec<(f64, f64)> = band.iter().map(|&(x, lo, _)| (x, lo)).collect();
        outline.extend(band.iter().rev().map(|&(x, _, hi)| (x, hi)));
        let line: Vec<(f64, f64)> = l
            .points
            .iter()
            .filter_map(|(x, v, _)| Some((*x, f(v)?)))
            .collect();
        let _ = write!(
            s,
            "<g class=\"{}\"><polygon class=\"band\" points=\"{}\"/><polyline class=\"line\" points=\"{}\"/>",
            series(i),
            fr.path(&outline),
            fr.path(&line)
        );
        let label = esc(&l.label);
        for (x, v, ci) in &l.points {
            if let Some(y) = f(v) {
                let _ = write!(
                    s,
                    "<circle class=\"dot\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"3.5\" data-x=\"{x}\" data-value=\"{}\" data-lo=\"{}\" data-hi=\"{}\"><title>{label}, {x}: {} [{}, {}]</title></circle>",
                    fr.px(*x),
                    fr.py(y),
                    num(v),
                    num(&ci[0]),
                    num(&ci[1]),
                    num(v),
                    num(&ci[0]),
                    num(&ci[1])
                );
            }
        }
        s.push_str("</g>");
        legend(
            &mut s,
            fr.left,
            fr.top + fr.h + 52.0 + 20.0 * i as f64,
            &format!("swatch {}", series(i)),
            &l.label,
        );
    }
    s.push_str("</svg>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_signal_detection_charts_carry_the_reported_values() {
        let sdt = json!({"dprime": 1.6542, "criterion": 0.1483});
        let m = sdt_model(&sdt);
        assert!(
            m.contains("data-dprime=\"1.6542\" data-criterion=\"0.1483\""),
            "{m}"
        );
        assert!(m.contains("class=\"area positive\"") && m.contains("class=\"area negative\""));
        assert!(sdt_model(&json!({})).is_empty());
        let r = roc_space(&[RocRun {
            label: "<b>".into(),
            sdt: &sdt,
        }]);
        assert!(
            r.contains("data-dprime=\"1.6542\" data-criterion=\"0.1483\""),
            "{r}"
        );
        assert!(!r.contains("<b>"));
        assert!(
            roc_space(&[RocRun {
                label: "x".into(),
                sdt: &json!(null)
            }])
            .is_empty()
        );
        // H′ = 0.75 and F′ = 0.25 give d′ = 2·z(0.75) and c = 0, and back.
        let d = 2.0 * evalsig_core::numeric::norm_ppf(0.75);
        let (fp, hp) = model_rates(d, 0.0);
        assert!(
            (fp - 0.25).abs() < 1e-12 && (hp - 0.75).abs() < 1e-12,
            "{fp} {hp}"
        );
        // A liberal criterion (c < 0) raises both rates along the same curve.
        let (fl, hl) = model_rates(d, -0.5);
        assert!(fl > fp && hl > hp);
    }

    #[test]
    fn ticks_fall_on_round_numbers() {
        let labels = |t: Vec<f64>| t.into_iter().map(tick_label).collect::<Vec<_>>();
        assert_eq!(
            labels(ticks(0.0, 1.0, 5)),
            ["0", "0.2", "0.4", "0.6", "0.8", "1"]
        );
        assert_eq!(
            labels(ticks(-4.0, 6.0, 4)),
            ["-4", "-2", "0", "2", "4", "6"]
        );
        assert_eq!(tick_label(-0.0), "0");
    }

    #[test]
    fn bars_and_lines_keep_each_value() {
        let k = json!(0.5493);
        let b = bars(
            &[Bar {
                label: "triage-v2".into(),
                value: &k,
                series: "s1",
            }],
            (0.0, 1.0),
            None,
        );
        assert!(b.contains("data-value=\"0.5493\""), "{b}");
        let (v, ci) = (json!(0.6), json!([0.4, 0.8]));
        let l = lines(
            &[Line {
                label: "majority".into(),
                points: vec![(1.0, &v, &ci), (2.0, &v, &ci)],
            }],
            ("k", "accuracy"),
        );
        assert!(
            l.contains("data-x=\"2\" data-value=\"0.6\" data-lo=\"0.4\" data-hi=\"0.8\""),
            "{l}"
        );
    }

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
