//! Local web dashboard for evalsig (SPEC §11, D-019): a loopback-only, read-only
//! server and the HTML and SVG it renders from the CLI's result JSON.

pub mod html;
pub mod server;
pub mod svg;

use serde_json::Value;

/// One self-contained page for a result (`--format html`): the verdict, warnings,
/// the charts its results allow, and the envelope itself as
/// `<script type="application/json">`. It loads nothing from anywhere.
pub fn static_report(schema: &str, data: &str, e: &Value) -> String {
    let r = &e["results"];
    let mut body = String::new();
    if let Some(code) = e["verdict"]["code"].as_str() {
        body.push_str(&format!(
            "<p class=\"verdict\" data-code=\"{}\"><strong>{}</strong>: {}</p>",
            html::esc(code),
            html::esc(code),
            html::esc(e["verdict"]["text"].as_str().unwrap_or_default())
        ));
    }
    let point = if schema == "evalsig.compare/1" {
        "delta"
    } else {
        "value"
    };
    if let Some(metrics) = r["metrics"].as_array() {
        let rows: Vec<svg::Row<'_>> = metrics
            .iter()
            .map(|m| svg::Row {
                label: m["metric"].as_str().unwrap_or_default().to_owned(),
                point: &m[point],
                ci: &m["ci"],
            })
            .collect();
        body.push_str(&svg::intervals(&rows, point == "delta"));
    }
    if r["transition"].is_object() {
        body.push_str(&svg::transition(&r["transition"]));
    }
    if r["roc"].is_object() {
        body.push_str(&svg::roc(&r["roc"]));
    }
    if r["calibration"].is_object() {
        body.push_str(&svg::reliability(&r["calibration"]));
    }
    // No `<` inside the element at all, so nothing can end or re-open it; in JSON a
    // `<` occurs only within strings, where `\u003c` means the same.
    let data = data.replace('<', "\\u003c");
    body.push_str(&format!(
        "<script type=\"application/json\" id=\"evalsig-data\">{data}</script>"
    ));
    let warnings: Vec<String> = e["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| {
            format!(
                "{}: {}",
                w["code"].as_str().unwrap_or_default(),
                w["message"].as_str().unwrap_or_default()
            )
        })
        .collect();
    html::page(schema, &[], &warnings, &body)
}
