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
nav a{margin-right:1rem}table{border-collapse:collapse;width:100%;margin:1rem 0}\
th,td{text-align:left;padding:.3rem .6rem;border-bottom:1px solid #ddd}td.n{text-align:right;font-variant-numeric:tabular-nums}\
.banner{background:#fff4e5;border-left:4px solid #e6a23c;padding:.5rem 1rem;margin:1rem 0}\
details{margin:1rem 0}summary{cursor:pointer;color:#555}svg{max-width:100%}";

/// A page: the navigation, warning banners above everything, then the body.
pub fn page(title: &str, nav: &[(&str, &str)], warnings: &[String], body: &str) -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{} · evalsig</title><style>{STYLE}</style></head><body><nav>",
        esc(title)
    );
    for (href, label) in nav {
        let _ = write!(s, "<a href=\"{}\">{}</a>", esc(href), esc(label));
    }
    s.push_str("</nav>");
    for w in warnings {
        let _ = write!(s, "<div class=\"banner\">{}</div>", esc(w));
    }
    let _ = write!(s, "<h1>{}</h1>{body}</body></html>", esc(title));
    s
}

/// A table from header cells and rows of cells; numbers align right.
pub fn table(head: &[&str], rows: &[Vec<String>]) -> String {
    let mut s = String::from("<table><thead><tr>");
    for h in head {
        let _ = write!(s, "<th>{}</th>", esc(h));
    }
    s.push_str("</tr></thead><tbody>");
    for r in rows {
        s.push_str("<tr>");
        for c in r {
            let numeric = c.parse::<f64>().is_ok();
            let _ = write!(
                s,
                "<td{}>{}</td>",
                if numeric { " class=\"n\"" } else { "" },
                esc(c)
            );
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
    }
}
