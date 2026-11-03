//! A strict RFC 4180 reader: comma-separated, `"`-quoted fields with `""` escapes,
//! CRLF or LF line ends, and a header row.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CsvError {
    #[error("line {line}: unterminated quoted field")]
    Unterminated { line: usize },
    #[error("line {line}: unexpected character after a closing quote")]
    AfterQuote { line: usize },
    #[error("line {line}: expected {expected} fields, found {found}")]
    Width {
        line: usize,
        expected: usize,
        found: usize,
    },
    #[error("the file has no header row")]
    NoHeader,
    #[error("duplicate column {0:?} in the header")]
    DuplicateColumn(String),
}

/// A parsed table: the header and the data rows with their starting line numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<(usize, Vec<String>)>,
}

impl Table {
    pub fn column(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|h| h == name)
    }

    /// Each row as an object of strings keyed by the header, with its line number.
    /// Empty cells are left out, as in a plain CSV import.
    pub fn objects(&self) -> Vec<(usize, serde_json::Value)> {
        self.rows
            .iter()
            .map(|(line, cells)| {
                let row = self
                    .header
                    .iter()
                    .zip(cells)
                    .filter(|(_, c)| !c.is_empty())
                    .map(|(h, c)| (h.clone(), serde_json::Value::String(c.clone())))
                    .collect();
                (*line, serde_json::Value::Object(row))
            })
            .collect()
    }
}

pub fn parse(text: &str) -> Result<Table, CsvError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut records = Vec::new();
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut line = 1;
    let mut record_line = 1;
    let mut in_record = false;
    while let Some(c) = chars.next() {
        in_record = true;
        match c {
            '"' if field.is_empty() => {
                let start = line;
                loop {
                    match chars.next() {
                        None => return Err(CsvError::Unterminated { line: start }),
                        Some('"') if chars.peek() == Some(&'"') => {
                            chars.next();
                            field.push('"');
                        }
                        Some('"') => break,
                        Some(ch) => {
                            if ch == '\n' {
                                line += 1;
                            }
                            field.push(ch);
                        }
                    }
                }
                match chars.peek() {
                    None | Some(',' | '\n' | '\r') => {}
                    Some(_) => return Err(CsvError::AfterQuote { line }),
                }
            }
            ',' => fields.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                fields.push(std::mem::take(&mut field));
                records.push((record_line, std::mem::take(&mut fields)));
                line += 1;
                record_line = line;
                in_record = false;
            }
            ch => field.push(ch),
        }
    }
    if in_record {
        fields.push(field);
        records.push((record_line, fields));
    }
    let mut records = records
        .into_iter()
        .filter(|(_, f)| !(f.len() == 1 && f[0].is_empty()));
    let (_, header) = records.next().ok_or(CsvError::NoHeader)?;
    for (i, h) in header.iter().enumerate() {
        if header[..i].contains(h) {
            return Err(CsvError::DuplicateColumn(h.clone()));
        }
    }
    let mut rows = Vec::new();
    for (n, r) in records {
        if r.len() != header.len() {
            return Err(CsvError::Width {
                line: n,
                expected: header.len(),
                found: r.len(),
            });
        }
        rows.push((n, r));
    }
    Ok(Table { header, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_and_line_ends() {
        let t =
            parse("a,b,c\r\n1,\"x, y\",\"say \"\"hi\"\"\"\n2,\"multi\nline\",\n\n3,,z").unwrap();
        assert_eq!(t.header, ["a", "b", "c"]);
        assert_eq!(
            t.rows[0],
            (2, vec!["1".into(), "x, y".into(), "say \"hi\"".into()])
        );
        assert_eq!(
            t.rows[1],
            (3, vec!["2".into(), "multi\nline".into(), String::new()])
        );
        assert_eq!(t.rows[2], (6, vec!["3".into(), String::new(), "z".into()]));
    }

    #[test]
    fn errors() {
        assert_eq!(parse("a\n\"open"), Err(CsvError::Unterminated { line: 2 }));
        assert_eq!(
            parse("a,b\n\"x\"y,1"),
            Err(CsvError::AfterQuote { line: 2 })
        );
        assert_eq!(
            parse("a,b\n1"),
            Err(CsvError::Width {
                line: 2,
                expected: 2,
                found: 1
            })
        );
        assert_eq!(parse(""), Err(CsvError::NoHeader));
        assert_eq!(parse("a,a\n"), Err(CsvError::DuplicateColumn("a".into())));
        assert_eq!(parse("\u{feff}a\n1").unwrap().header, ["a"]);
    }
}
