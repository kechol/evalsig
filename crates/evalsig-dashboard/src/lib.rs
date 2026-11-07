//! Local web dashboard for evalsig (SPEC §11, D-019): a loopback-only, read-only
//! server, its screens, and the HTML and SVG they render from result JSON.

pub mod html;
pub mod i18n;
pub mod screens;
pub mod server;
pub mod svg;

use evalsig_core::config::Language;
use evalsig_project::envelope::Schema;
use serde_json::Value;

/// One self-contained page for a result (`--format html`), in English as all CLI output
/// is (D-002): the verdict, warnings,
/// the charts its results allow, and the envelope itself as
/// `<script type="application/json">`. It loads nothing from anywhere.
pub fn static_report(e: &Value) -> String {
    let schema = e["schema"].as_str().unwrap_or_default();
    let r = &e["results"];
    let mut body = String::new();
    if let Some(code) = e["verdict"]["code"].as_str() {
        body.push_str(&html::verdict(
            code,
            e["verdict"]["text"].as_str().unwrap_or_default(),
        ));
    }
    let point = if schema == Schema::Compare.name() {
        "delta"
    } else {
        "value"
    };
    // Several candidates each carry their own comparison.
    let parts: Vec<&Value> = match r["comparisons"].as_array() {
        Some(c) => c.iter().collect(),
        None => vec![r],
    };
    for part in parts {
        if let Some(name) = part["arms"]["candidate"]
            .as_str()
            .filter(|_| r["comparisons"].is_array())
        {
            body.push_str(&format!("<h2>{}</h2>", html::esc(name)));
        }
        body.push_str(&svg::intervals(
            &svg::metric_rows(&part["metrics"], point),
            point == "delta",
        ));
        if part["transition"].is_object() {
            body.push_str(&svg::transition(&part["transition"], Language::En));
        }
    }
    if r["sdt"].is_object() {
        body.push_str(&svg::sdt_model(&r["sdt"], Language::En));
    }
    if r["roc"].is_object() {
        body.push_str(&svg::roc(&r["roc"], Language::En));
    }
    if r["calibration"].is_object() {
        body.push_str(&svg::reliability(&r["calibration"], Language::En));
    }
    // No `<` inside the element at all, so nothing can end or re-open it; in JSON a
    // `<` occurs only within strings, where `\u003c` means the same.
    let data = e.to_string().replace('<', "\\u003c");
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
    html::page("en", schema, "", &[], &warnings, &body)
}
