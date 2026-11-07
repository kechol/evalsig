//! HTML written through an escaping writer: every value from results goes through
//! `esc`, so text from the evaluated data cannot become markup.

use std::fmt::Write as _;

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
    pub fn of_verdict(code: &str) -> Tone {
        match code {
            "pass" | "improved" | "stable" | "human_parity" => Tone::Positive,
            "fail" | "worsened" | "drifted" | "below_human" => Tone::Negative,
            "negligible" | "indistinguishable" => Tone::Neutral,
            _ => Tone::Warning,
        }
    }
}

/// A page: the app bar with the screens as tabs, the page head with the question the
/// screen answers (`lead`, none when empty), warnings above everything else, then the
/// body. The tab labelled `title` is marked as the current page.
pub fn page(
    title: &str,
    lead: &str,
    nav: &[(String, &str)],
    warnings: &[String],
    body: &str,
) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{} · evalsig</title><style>{STYLE}</style></head><body><header class=\"appbar\"><span class=\"brand\">evalsig</span>",
        esc(title)
    );
    if !nav.is_empty() {
        s.push_str("<nav class=\"tabs\" aria-label=\"Screens\">");
        for (href, label) in nav {
            let current = if *label == title {
                " aria-current=\"page\""
            } else {
                ""
            };
            let _ = write!(
                s,
                "<a class=\"tab\" href=\"{}\"{current}>{}</a>",
                esc(href),
                esc(label)
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
        let _ = write!(
            s,
            "<div class=\"callout warning\" role=\"status\">{}</div>",
            esc(w)
        );
    }
    let _ = write!(s, "{body}</main></body></html>");
    s
}

/// A verdict as a callout in its tone: its code (kept in `data-code`) and its text.
pub fn verdict(code: &str, text: &str) -> String {
    format!(
        "<p class=\"verdict callout {}\" data-code=\"{}\"><strong>{}</strong>{}</p>",
        Tone::of_verdict(code).class(),
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

/// A number as the Markdown output writes it: whole numbers as they are, others with
/// four decimals.
pub fn shown(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 {
        format!("{x:.0}")
    } else {
        format!("{x:.4}")
    }
}

/// A table cell of text. A number is right-aligned and shown with `shown`, its exact
/// value kept in `data-value`.
pub fn td(c: &str) -> String {
    match c.parse::<f64>() {
        Ok(x) if x.is_finite() => format!(
            "<td class=\"n\" data-value=\"{}\">{}</td>",
            esc(c),
            shown(x)
        ),
        _ => format!("<td>{}</td>", esc(c)),
    }
}

/// A table from header cells and rows of text cells (see `td`).
pub fn table(head: &[&str], rows: &[Vec<String>]) -> String {
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|c| td(c)).collect())
        .collect();
    table_html(head, &rows)
}

/// A table from header cells and rows of finished `<td>` elements. A column whose
/// cells are all numbers gets a right-aligned header.
pub fn table_html(head: &[&str], rows: &[Vec<String>]) -> String {
    let numeric = |j: usize| {
        !rows.is_empty()
            && rows
                .iter()
                .all(|r| r.get(j).is_some_and(|c| c.starts_with("<td class=\"n\"")))
    };
    let mut s = String::from("<div class=\"table-wrap\"><table><thead><tr>");
    for (j, h) in head.iter().enumerate() {
        let class = if numeric(j) { " class=\"n\"" } else { "" };
        let _ = write!(s, "<th{class}>{}</th>", esc(h));
    }
    s.push_str("</tr></thead><tbody>");
    for r in rows {
        s.push_str("<tr>");
        for c in r {
            s.push_str(c);
        }
        s.push_str("</tr>");
    }
    s.push_str("</tbody></table></div>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_never_becomes_markup() {
        assert_eq!(
            esc("<script>\"x\"&'"),
            "&lt;script&gt;&quot;x&quot;&amp;&#39;"
        );
        let p = page("<b>", "<i>", &[], &["<img src=x>".into()], "");
        assert!(!p.contains("<b>") && !p.contains("<i>") && !p.contains("<img"));
        assert!(!td("<b>").contains("<b>") && !button("\"><b>", "<b>", true).contains("<b>"));
    }

    #[test]
    fn numbers_show_four_decimals_and_keep_the_exact_value() {
        assert_eq!(
            td("0.7583333333"),
            "<td class=\"n\" data-value=\"0.7583333333\">0.7583</td>"
        );
        assert_eq!(td("300"), "<td class=\"n\" data-value=\"300\">300</td>");
        assert_eq!(td("–"), "<td>–</td>");
        let t = table(&["Run", "Items"], &[vec!["a".into(), "3".into()]]);
        assert!(t.contains("<th>Run</th><th class=\"n\">Items</th>"), "{t}");
    }

    #[test]
    fn the_current_page_is_marked() {
        let nav = [
            ("./".to_owned(), "Overview"),
            ("compare".to_owned(), "Comparison"),
        ];
        let p = page("Comparison", "", &nav, &[], "");
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
