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

const STYLE: &str = "body{font:15px/1.5 system-ui,sans-serif;margin:2rem auto;max-width:60rem;padding:0 1rem;color:#1d1d1f}\
nav a{margin-right:1rem}nav a[aria-current]{font-weight:600;color:inherit;text-decoration:none}\
table{border-collapse:collapse;width:100%;margin:1rem 0}\
th,td{text-align:left;padding:.3rem .6rem;border-bottom:1px solid #ddd}td.n,th.n{text-align:right;font-variant-numeric:tabular-nums}\
p.links a{margin-right:.8rem}\
.banner{background:#fff4e5;border-left:4px solid #e6a23c;padding:.5rem 1rem;margin:1rem 0}\
details{margin:1rem 0}summary{cursor:pointer;color:#555}svg{max-width:100%}";

/// A page: the navigation, warning banners above everything, then the body. The
/// navigation entry labelled `title` is marked as the current page.
pub fn page(title: &str, nav: &[(String, &str)], warnings: &[String], body: &str) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{} · evalsig</title><style>{STYLE}</style></head><body><nav>",
        esc(title)
    );
    for (href, label) in nav {
        let current = if *label == title {
            " aria-current=\"page\""
        } else {
            ""
        };
        let _ = write!(s, "<a href=\"{}\"{current}>{}</a>", esc(href), esc(label));
    }
    s.push_str("</nav>");
    for w in warnings {
        let _ = write!(s, "<div class=\"banner\">{}</div>", esc(w));
    }
    let _ = write!(s, "<h1>{}</h1>{body}</body></html>", esc(title));
    s
}

/// A verdict line: its code (kept in `data-code`) and its text.
pub fn verdict(code: &str, text: &str) -> String {
    format!(
        "<p class=\"verdict\" data-code=\"{}\"><strong>{}</strong>: {}</p>",
        esc(code),
        esc(code),
        esc(text)
    )
}

/// A link; `href` and `text` are escaped.
pub fn link(href: &str, text: &str) -> String {
    format!("<a href=\"{}\">{}</a>", esc(href), esc(text))
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
    let mut s = String::from("<table><thead><tr>");
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
    s.push_str("</tbody></table>");
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
        let p = page("<b>", &[], &["<img src=x>".into()], "");
        assert!(!p.contains("<b>") && !p.contains("<img"));
        assert!(!td("<b>").contains("<b>") && !link("\"><b>", "<b>").contains("<b>"));
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
        let p = page("Comparison", &nav, &[], "");
        assert!(p.contains("<a href=\"compare\" aria-current=\"page\">Comparison</a>"));
        assert!(p.contains("<a href=\"./\">Overview</a>"));
    }
}
