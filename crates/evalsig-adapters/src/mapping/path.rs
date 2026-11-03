//! The path subset of `mapping/1` (D-011): `$` (the row) or `@` (the current
//! element), then `.name`, `[n]`, `[*]` and `[?(@.x)]` / `[?(@.x == literal)]`
//! segments, and `["key"]` for keys with any text. `{name}` inserts an `explode`
//! variable and `{$.x}` / `{@.x}` the text of a value, inside names and templates
//! alike.

use std::collections::BTreeMap;

use serde_json::Value as Json;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{text:?}: {message}")]
pub struct PathError {
    pub text: String,
    pub message: String,
}

/// Text with insertions.
#[derive(Debug, Clone, PartialEq)]
pub struct Template(Vec<Part>);

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Lit(String),
    Var(String),
    Path(Path),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Root {
    Row,
    Current,
}

#[derive(Debug, Clone, PartialEq)]
enum Seg {
    Key(Template),
    Index(usize),
    All,
    Filter(Box<Path>, Option<Json>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    root: Root,
    segs: Vec<Seg>,
}

/// What paths are evaluated against.
#[derive(Debug, Clone, Copy)]
pub struct Scope<'a> {
    pub row: &'a Json,
    pub current: Option<&'a Json>,
    pub vars: &'a BTreeMap<String, String>,
}

impl<'a> Scope<'a> {
    pub fn with_current(self, current: &'a Json) -> Scope<'a> {
        Scope {
            current: Some(current),
            ..self
        }
    }
}

fn err(text: &str, message: impl Into<String>) -> PathError {
    PathError {
        text: text.to_owned(),
        message: message.into(),
    }
}

/// The index just past the `}` that closes the `{` at `open`.
fn closing(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

impl Template {
    pub fn parse(s: &str) -> Result<Template, PathError> {
        let mut parts = Vec::new();
        let mut lit = String::new();
        let mut i = 0;
        while i < s.len() {
            let c = s[i..].chars().next().expect("in bounds");
            if c == '{' {
                let end = closing(s, i).ok_or_else(|| err(s, "unclosed {"))?;
                if !lit.is_empty() {
                    parts.push(Part::Lit(std::mem::take(&mut lit)));
                }
                let inner = &s[i + 1..end - 1];
                parts.push(if inner.starts_with(['$', '@']) {
                    Part::Path(Path::parse(inner)?)
                } else if !inner.is_empty()
                    && inner.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    Part::Var(inner.to_owned())
                } else {
                    return Err(err(
                        s,
                        format!("{{{inner}}} is neither a variable nor a path"),
                    ));
                });
                i = end;
            } else {
                lit.push(c);
                i += c.len_utf8();
            }
        }
        if !lit.is_empty() || parts.is_empty() {
            parts.push(Part::Lit(lit));
        }
        Ok(Template(parts))
    }

    /// The text, or `None` if an insertion has no value.
    pub fn render(&self, scope: Scope<'_>) -> Option<String> {
        let mut out = String::new();
        for p in &self.0 {
            match p {
                Part::Lit(s) => out.push_str(s),
                Part::Var(v) => out.push_str(scope.vars.get(v)?),
                Part::Path(p) => out.push_str(&text(p.first(scope)?)?),
            }
        }
        Some(out)
    }
}

/// The text of a scalar for templates and `value_map` keys.
pub fn text(v: &Json) -> Option<String> {
    match v {
        Json::String(s) => Some(s.clone()),
        Json::Number(n) => Some(n.to_string()),
        Json::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

impl Path {
    pub fn parse(s: &str) -> Result<Path, PathError> {
        let root = match s.chars().next() {
            Some('$') => Root::Row,
            Some('@') => Root::Current,
            _ => return Err(err(s, "a path starts with $ or @")),
        };
        let mut segs = Vec::new();
        let mut i = 1;
        while i < s.len() {
            let rest = &s[i..];
            if let Some(after) = rest.strip_prefix('.') {
                // A name runs to the next `.` or `[` outside braces.
                let mut end = 0;
                while end < after.len() {
                    let c = after[end..].chars().next().expect("in bounds");
                    if c == '.' || c == '[' {
                        break;
                    }
                    end = if c == '{' {
                        closing(after, end).ok_or_else(|| err(s, "unclosed {"))?
                    } else {
                        end + c.len_utf8()
                    };
                }
                if end == 0 {
                    return Err(err(s, "empty name"));
                }
                segs.push(Seg::Key(Template::parse(&after[..end])?));
                i += 1 + end;
            } else if let Some(after) = rest.strip_prefix('[') {
                let close = matching_bracket(after).ok_or_else(|| err(s, "unclosed ["))?;
                let inner = &after[..close];
                segs.push(if inner == "*" {
                    Seg::All
                } else if inner.starts_with(['"', '\'']) {
                    // A quoted key: any text, dots and brackets included.
                    Seg::Key(Template(vec![Part::Lit(literal(s, inner)?)]))
                } else if let Some(f) = inner.strip_prefix("?(").and_then(|f| f.strip_suffix(')')) {
                    filter(s, f.trim())?
                } else {
                    Seg::Index(
                        inner
                            .parse()
                            .map_err(|_| err(s, format!("bad index [{inner}]")))?,
                    )
                });
                i += 1 + close + 1;
            } else {
                return Err(err(s, format!("unexpected {:?}", rest.chars().next())));
            }
        }
        Ok(Path { root, segs })
    }

    /// Every value the path reaches, in document order.
    pub fn eval<'a>(&self, scope: Scope<'a>) -> Vec<&'a Json> {
        let start = match self.root {
            Root::Row => scope.row,
            Root::Current => match scope.current {
                Some(c) => c,
                None => return Vec::new(),
            },
        };
        self.walk(start, scope)
    }

    /// The first value the path reaches from `start` instead of its root; insertions
    /// still read the scope (a join path reads the joined file, its insertions the row).
    pub fn first_from<'a>(&self, start: &'a Json, scope: Scope<'a>) -> Option<&'a Json> {
        self.walk(start, scope).into_iter().next()
    }

    fn walk<'a>(&self, start: &'a Json, scope: Scope<'a>) -> Vec<&'a Json> {
        let mut values = vec![start];
        for seg in &self.segs {
            values = values
                .into_iter()
                .flat_map(|v| step(v, seg, scope))
                .collect();
        }
        values
    }

    pub fn first<'a>(&self, scope: Scope<'a>) -> Option<&'a Json> {
        self.eval(scope).into_iter().next()
    }
}

/// The index of the `]` that closes an opened `[`, skipping quoted text.
fn matching_bracket(s: &str) -> Option<usize> {
    let mut depth = 1;
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[') => depth += 1,
            (None, ']') => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// A quoted literal: `'text'` or a JSON string.
fn literal(s: &str, q: &str) -> Result<String, PathError> {
    if let Some(inner) = q.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        return Ok(inner.to_owned());
    }
    serde_json::from_str(q).map_err(|_| err(s, format!("bad quoted key {q}")))
}

/// `==` outside quotes.
fn split_equals(f: &str) -> Option<(&str, &str)> {
    let mut quote = None;
    for (i, c) in f.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '=') if f[i..].starts_with("==") => return Some((&f[..i], &f[i + 2..])),
            _ => {}
        }
    }
    None
}

fn filter(s: &str, f: &str) -> Result<Seg, PathError> {
    let (lhs, rhs) = match split_equals(f) {
        Some((l, r)) => (l.trim(), Some(r.trim())),
        None => (f, None),
    };
    let path = Path::parse(lhs)?;
    let literal = rhs
        .map(|r| {
            if let Some(q) = r.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
                Ok(Json::String(q.to_owned()))
            } else {
                serde_json::from_str(r).map_err(|_| err(s, format!("bad literal {r}")))
            }
        })
        .transpose()?;
    Ok(Seg::Filter(Box::new(path), literal))
}

fn truthy(v: &Json) -> bool {
    !matches!(v, Json::Null | Json::Bool(false))
}

fn step<'a>(v: &'a Json, seg: &Seg, scope: Scope<'a>) -> Vec<&'a Json> {
    match (seg, v) {
        (Seg::Key(t), Json::Object(m)) => t
            .render(scope)
            .and_then(|k| m.get(&k))
            .into_iter()
            .collect(),
        (Seg::Index(i), Json::Array(xs)) => xs.get(*i).into_iter().collect(),
        (Seg::All, Json::Array(xs)) => xs.iter().collect(),
        (Seg::All, Json::Object(m)) => m.values().collect(),
        (Seg::Filter(p, lit), Json::Array(xs)) => xs
            .iter()
            .filter(|x| match (p.first(scope.with_current(x)), lit) {
                (Some(g), Some(l)) => g == l,
                (Some(g), None) => truthy(g),
                (None, _) => false,
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn scope<'a>(row: &'a Json, vars: &'a BTreeMap<String, String>) -> Scope<'a> {
        Scope {
            row,
            current: None,
            vars,
        }
    }

    #[test]
    fn names_indices_wildcards_and_filters() {
        let row = json!({"a": {"b": [1, 2]}, "rules": [{"id": "x", "on": true}, {"id": "y", "on": false}]});
        let vars = BTreeMap::new();
        let s = scope(&row, &vars);
        assert_eq!(Path::parse("$.a.b[1]").unwrap().eval(s), [&json!(2)]);
        assert_eq!(
            Path::parse("$.a.b[*]").unwrap().eval(s),
            [&json!(1), &json!(2)]
        );
        let on = Path::parse("$.rules[?(@.on)]").unwrap().eval(s);
        assert_eq!(on, [&json!({"id": "x", "on": true})]);
        let y = Path::parse("$.rules[?(@.id == 'y')]").unwrap().eval(s);
        assert_eq!(y.len(), 1);
        assert!(Path::parse("$.missing").unwrap().eval(s).is_empty());
    }

    #[test]
    fn insertions_in_names_and_templates() {
        let row = json!({"task": "priority", "id": "t-1", "runs": {"a": {"level": "high"}}, "missing": {"b": "timeout"}});
        let vars = BTreeMap::from([("r".to_owned(), "a".to_owned())]);
        let s = scope(&row, &vars);
        assert_eq!(
            Path::parse("$.runs.{r}.level").unwrap().first(s),
            Some(&json!("high"))
        );
        let t = Template::parse("{$.task}:{$.id}").unwrap();
        assert_eq!(t.render(s).as_deref(), Some("priority:t-1"));
        let vars_b = BTreeMap::from([("r".to_owned(), "b".to_owned())]);
        let t = Template::parse("run {r}: {$.missing.{r}}").unwrap();
        assert_eq!(
            t.render(scope(&row, &vars_b)).as_deref(),
            Some("run b: timeout")
        );
        assert_eq!(t.render(s), None);
    }

    #[test]
    fn quoted_keys_and_literals_may_hold_any_text() {
        let row = json!({"model.score": 3, "rules": [{"id": "a]b"}, {"id": "x==y"}]});
        let vars = BTreeMap::new();
        let s = scope(&row, &vars);
        assert_eq!(
            Path::parse(r#"$["model.score"]"#).unwrap().first(s),
            Some(&json!(3))
        );
        assert_eq!(
            Path::parse("$.rules[?(@.id == 'a]b')]")
                .unwrap()
                .eval(s)
                .len(),
            1
        );
        assert_eq!(
            Path::parse("$.rules[?(@.id == 'x==y')]")
                .unwrap()
                .eval(s)
                .len(),
            1
        );
    }

    #[test]
    fn malformed_paths_are_rejected() {
        for bad in ["a.b", "$.", "$.a[", "$.{x", "$[x]", "$.{a b}"] {
            assert!(Path::parse(bad).is_err(), "{bad}");
        }
    }
}
