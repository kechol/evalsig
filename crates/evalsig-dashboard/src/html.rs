//! HTML written through an escaping writer: every value from results goes through
//! `esc`, so text from the evaluated data cannot become markup.

use std::fmt::Write as _;

use evalsig_project::envelope::VerdictCode;
use serde_json::Value;

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// The design system (tokens and components), inlined into every page.
const STYLE: &str = include_str!("style.css");

/// The tone a state is coloured with; `style.css` gives each its colours.
#[derive(Clone, Copy)]
pub enum Tone {
    Positive,
    Negative,
    Warning,
    Neutral,
}

impl Tone {
    fn class(self) -> &'static str {
        match self {
            Tone::Positive => "positive",
            Tone::Negative => "negative",
            Tone::Warning => "warning",
            Tone::Neutral => "neutral",
        }
    }

    /// A verdict's tone: a good outcome, a bad one, no difference, or no answer.
    pub fn of_verdict(code: VerdictCode) -> Tone {
        use VerdictCode as V;
        match code {
            V::Pass | V::Improved | V::Stable | V::HumanParity => Tone::Positive,
            V::Fail | V::Worsened | V::Drifted | V::BelowHuman => Tone::Negative,
            V::Negligible | V::Indistinguishable => Tone::Neutral,
            V::InsufficientData | V::Blocked | V::NeedsDecisions | V::BudgetExhausted => {
                Tone::Warning
            }
        }
    }
}

/// One link of the navigation: where it goes, its label, and whether it is the page
/// shown.
pub struct Tab {
    pub href: String,
    pub label: &'static str,
    pub current: bool,
}

/// A page in the language `lang` (its `lang` attribute): the app bar with the screens as
/// tabs, the page head with the question the screen answers (`lead`, none when empty),
/// warnings above everything else, then the body.
pub fn page(
    lang: &str,
    title: &str,
    lead: &str,
    nav: &[Tab],
    warnings: &[String],
    body: &str,
) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "<!DOCTYPE html><html lang=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{} · evalsig</title><style>{STYLE}</style></head><body><header class=\"appbar\"><span class=\"brand\">evalsig</span>",
        esc(lang),
        esc(title)
    );
    if !nav.is_empty() {
        s.push_str("<nav class=\"tabs\" aria-label=\"Screens\">");
        for t in nav {
            let current = if t.current {
                " aria-current=\"page\""
            } else {
                ""
            };
            let _ = write!(
                s,
                "<a class=\"tab\" href=\"{}\"{current}>{}</a>",
                esc(&t.href),
                esc(t.label)
            );
        }
        s.push_str("</nav>");
    }
    let _ = write!(
        s,
        "</header><main><div class=\"page-head\"><h1>{}</h1>",
        esc(title)
    );
    if !lead.is_empty() {
        let _ = write!(s, "<p>{}</p>", esc(lead));
    }
    s.push_str("</div>");
    for w in warnings {
        s.push_str(&callout(Tone::Warning, None, w));
    }
    let _ = write!(s, "{body}</main></body></html>");
    s
}

/// A sentence in its tone with a bar of colour, `strong` (if any) leading it.
pub fn callout(tone: Tone, strong: Option<&str>, text: &str) -> String {
    let lead = strong.map_or_else(String::new, |t| format!("<strong>{}</strong>", esc(t)));
    format!(
        "<div class=\"callout {}\" role=\"status\">{lead}{}</div>",
        tone.class(),
        esc(text)
    )
}

/// A verdict as a callout in its tone: its code (kept in `data-code`) and its text. A
/// code this build does not know is shown as no answer.
pub fn verdict(code: &str, text: &str) -> String {
    let tone = VerdictCode::ALL
        .into_iter()
        .find(|c| c.as_str() == code)
        .map_or(Tone::Warning, Tone::of_verdict);
    format!(
        "<p class=\"verdict callout {}\" data-code=\"{}\"><strong>{}</strong>{}</p>",
        tone.class(),
        esc(code),
        esc(code),
        esc(text)
    )
}

/// A panel: one section of a screen under `title`, with `aside` (badges, actions; HTML)
/// in its head and `body` (HTML) below.
pub fn panel(title: &str, aside: &str, body: &str) -> String {
    format!(
        "<section class=\"panel\"><div class=\"panel-head\"><h2>{}</h2>{aside}</div>{body}</section>",
        esc(title)
    )
}

/// A sentence saying what is missing or empty.
pub fn empty(text: &str) -> String {
    format!("<p class=\"empty\">{}</p>", esc(text))
}

/// `body` (HTML) folded under `summary`.
pub fn details(summary: &str, body: &str) -> String {
    format!(
        "<details><summary>{}</summary>{body}</details>",
        esc(summary)
    )
}

/// A short state in its tone.
pub fn badge(tone: Tone, text: &str) -> String {
    format!(
        "<span class=\"badge {}\">{}</span>",
        tone.class(),
        esc(text)
    )
}

/// A link styled as a button; `quiet` for the ones beside the main action.
pub fn button(href: &str, text: &str, quiet: bool) -> String {
    format!(
        "<a class=\"button{}\" href=\"{}\">{}</a>",
        if quiet { " quiet" } else { "" },
        esc(href),
        esc(text)
    )
}

/// Actions for a panel's head.
pub fn actions(buttons: &[String]) -> String {
    format!("<div class=\"actions\">{}</div>", buttons.concat())
}

/// A number for reading: whole numbers as they are, others with four decimals.
pub fn shown(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        format!("{x:.4}")
    }
}

/// A difference for reading: its sign and four decimals.
pub fn signed(x: f64) -> String {
    format!("{x:+.4}")
}

/// A value of the results for reading: a number as `shown`, anything else a dash.
pub fn number(v: &Value) -> String {
    v.as_f64().map_or_else(|| "–".to_owned(), shown)
}

/// A table cell: text, a number of the results, or finished markup.
pub enum Cell {
    Text(String),
    Num(Value),
    Html(String),
}

impl Cell {
    /// A value of the results as a cell: numbers stay numbers, a missing value a dash,
    /// text as it is.
    pub fn of(v: &Value) -> Cell {
        match v {
            Value::Number(_) => Cell::Num(v.clone()),
            Value::Null => Cell::Text("–".to_owned()),
            Value::String(s) => Cell::Text(s.clone()),
            other => Cell::Text(other.to_string()),
        }
    }

    fn td(&self) -> String {
        match self {
            Cell::Text(t) => format!("<td>{}</td>", esc(t)),
            Cell::Num(v) => format!(
                "<td class=\"n\" data-value=\"{}\">{}</td>",
                esc(&v.to_string()),
                number(v)
            ),
            Cell::Html(h) => format!("<td>{h}</td>"),
        }
    }
}

impl From<String> for Cell {
    fn from(t: String) -> Cell {
        Cell::Text(t)
    }
}

impl From<&str> for Cell {
    fn from(t: &str) -> Cell {
        Cell::Text(t.to_owned())
    }
}

/// A table. A number is right-aligned and shown with `shown`, its exact value kept in
/// `data-value`; a column of numbers gets a right-aligned header.
pub fn table(head: &[&str], rows: &[Vec<Cell>]) -> String {
    let numeric =
        |j: usize| !rows.is_empty() && rows.iter().all(|r| matches!(r.get(j), Some(Cell::Num(_))));
    let mut s = String::from("<div class=\"table-wrap\"><table><thead><tr>");
    for (j, h) in head.iter().enumerate() {
        let class = if numeric(j) { " class=\"n\"" } else { "" };
        let _ = write!(s, "<th{class}>{}</th>", esc(h));
    }
    s.push_str("</tr></thead><tbody>");
    for r in rows {
        s.push_str("<tr>");
        for c in r {
            s.push_str(&c.td());
        }
        s.push_str("</tr>");
    }
    s.push_str("</tbody></table></div>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_never_becomes_markup() {
        assert_eq!(
            esc("<script>\"x\"&'"),
            "&lt;script&gt;&quot;x&quot;&amp;&#39;"
        );
        let p = page("en", "<b>", "<i>", &[], &["<img src=x>".into()], "");
        assert!(!p.contains("<b>") && !p.contains("<i>") && !p.contains("<img"));
        assert!(!Cell::from("<b>").td().contains("<b>"));
        assert!(!button("\"><b>", "<b>", true).contains("<b>"));
        assert!(!callout(Tone::Warning, Some("<b>"), "<i>").contains("<b>"));
    }

    #[test]
    fn numbers_show_four_decimals_and_keep_the_exact_value() {
        assert_eq!(
            Cell::of(&json!(0.7583333333)).td(),
            "<td class=\"n\" data-value=\"0.7583333333\">0.7583</td>"
        );
        assert_eq!(
            Cell::of(&json!(300)).td(),
            "<td class=\"n\" data-value=\"300\">300</td>"
        );
        assert_eq!(Cell::of(&Value::Null).td(), "<td>–</td>");
        // Text that looks like a number stays as written.
        assert_eq!(Cell::of(&json!("1.10")).td(), "<td>1.10</td>");
        let t = table(&["Run", "Items"], &[vec!["a".into(), Cell::of(&json!(3))]]);
        assert!(t.contains("<th>Run</th><th class=\"n\">Items</th>"), "{t}");
    }

    #[test]
    fn the_current_page_is_marked() {
        let nav = [
            Tab {
                href: "./".to_owned(),
                label: "Overview",
                current: false,
            },
            Tab {
                href: "compare".to_owned(),
                label: "Comparison",
                current: true,
            },
        ];
        let p = page("en", "Comparison", "", &nav, &[], "");
        assert!(
            p.contains("<a class=\"tab\" href=\"compare\" aria-current=\"page\">Comparison</a>")
        );
        assert!(p.contains("<a class=\"tab\" href=\"./\">Overview</a>"));
    }

    #[test]
    fn verdicts_take_the_tone_of_their_outcome() {
        assert!(verdict("improved", "x").contains("class=\"verdict callout positive\""));
        assert!(verdict("worsened", "x").contains("callout negative"));
        assert!(verdict("indistinguishable", "x").contains("callout neutral"));
        assert!(verdict("insufficient_data", "x").contains("callout warning"));
    }
}
