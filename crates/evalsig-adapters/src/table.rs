//! Flat tables whose column names are `records/1` field names: one label or one
//! judgment per row. Values that cannot be mapped become missing (`invalid_output`)
//! rather than a number.

use evalsig_core::records::{MissingReason, OutputType};
use serde_json::{Map, Value, json};

use crate::csv::Table;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Judgment,
    Label,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TableError {
    #[error("unknown column {column:?} for {kind} rows; known columns: {known}")]
    UnknownColumn {
        column: String,
        kind: &'static str,
        known: String,
    },
    #[error("missing required column {0:?}")]
    MissingColumn(&'static str),
    #[error("line {line}, column {column:?}: {message}")]
    Cell {
        line: usize,
        column: String,
        message: String,
    },
}

const LABEL_COLUMNS: [&str; 12] = [
    "item_id",
    "rater_id",
    "source",
    "task",
    "kind",
    "label_replicate",
    "value",
    "version",
    "reason",
    "labeled_at",
    "source_ref",
    "levels",
];
const LABEL_REQUIRED: [&str; 3] = ["item_id", "rater_id", "value"];

const JUDGMENT_COLUMNS: [&str; 10] = [
    "item_id",
    "run_id",
    "task",
    "gen_replicate",
    "grade_replicate",
    "output_type",
    "levels",
    "value",
    "judged_at",
    "source_ref",
];
const JUDGMENT_REQUIRED: [&str; 4] = ["item_id", "run_id", "output_type", "value"];

/// Columns whose cells are integers.
const INTEGER_COLUMNS: [&str; 5] = [
    "label_replicate",
    "version",
    "levels",
    "gen_replicate",
    "grade_replicate",
];

/// Convert each row into a record object, with its line number. Defaults: a label's
/// `source` is `human` and its `version` is 1; a judgment's `gen_replicate` is 0. Empty
/// optional cells are absent. The objects still need the usual record validation.
pub fn to_records(table: &Table, kind: RowKind) -> Result<Vec<(usize, Value)>, TableError> {
    let (known, required, kind_name): (&[&str], &[&str], _) = match kind {
        RowKind::Label => (&LABEL_COLUMNS, &LABEL_REQUIRED, "label"),
        RowKind::Judgment => (&JUDGMENT_COLUMNS, &JUDGMENT_REQUIRED, "judgment"),
    };
    for h in &table.header {
        if !known.contains(&h.as_str()) {
            return Err(TableError::UnknownColumn {
                column: h.clone(),
                kind: kind_name,
                known: known.join(", "),
            });
        }
    }
    for r in required {
        if table.column(r).is_none() {
            return Err(TableError::MissingColumn(r));
        }
    }
    table
        .rows
        .iter()
        .map(|(line, cells)| row_to_record(&table.header, cells, *line, kind).map(|v| (*line, v)))
        .collect()
}

fn row_to_record(
    header: &[String],
    cells: &[String],
    line: usize,
    kind: RowKind,
) -> Result<Value, TableError> {
    let mut m = Map::new();
    m.insert(
        "record".into(),
        json!(if kind == RowKind::Label {
            "label"
        } else {
            "judgment"
        }),
    );
    let mut raw_value = None;
    for (h, cell) in header.iter().zip(cells) {
        if h == "value" {
            raw_value = Some(cell.as_str());
            continue;
        }
        if cell.is_empty() {
            if JUDGMENT_REQUIRED.contains(&h.as_str()) || LABEL_REQUIRED.contains(&h.as_str()) {
                return Err(TableError::Cell {
                    line,
                    column: h.clone(),
                    message: "required cell is empty".into(),
                });
            }
            continue;
        }
        let v = if INTEGER_COLUMNS.contains(&h.as_str()) {
            let n: u64 = cell.parse().map_err(|_| TableError::Cell {
                line,
                column: h.clone(),
                message: format!("{cell:?} is not a non-negative integer"),
            })?;
            json!(n)
        } else {
            json!(cell)
        };
        m.insert(h.clone(), v);
    }
    if kind == RowKind::Label {
        m.entry("source").or_insert(json!("human"));
        m.entry("version").or_insert(json!(1));
    } else {
        m.entry("gen_replicate").or_insert(json!(0));
    }
    let output_type = m
        .get("output_type")
        .and_then(Value::as_str)
        .and_then(|t| match t {
            "binary" => Some(OutputType::Binary),
            "ordinal" => Some(OutputType::Ordinal),
            "score" => Some(OutputType::Score),
            _ => None,
        });
    let levels = m.get("levels").and_then(Value::as_u64);
    match map_value(raw_value.unwrap_or(""), output_type, levels) {
        Ok(v) => {
            m.insert("value".into(), v);
        }
        Err(detail) => {
            m.insert(
                "missing".into(),
                json!({"reason": MissingReason::InvalidOutput, "detail": detail}),
            );
        }
    }
    Ok(Value::Object(m))
}

/// Map a cell to a value of the output type (labels have none: infer bool, then
/// integer, then number).
fn map_value(
    cell: &str,
    output_type: Option<OutputType>,
    levels: Option<u64>,
) -> Result<Value, String> {
    let s = cell.trim();
    let not = |what: &str| format!("{s:?} is not {what}");
    let boolean = || match s.to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    let integer = || s.parse::<i64>().ok();
    let number = || s.parse::<f64>().ok().filter(|f| f.is_finite());
    match output_type {
        Some(OutputType::Binary) => boolean().map(Value::Bool).ok_or_else(|| not("a boolean")),
        Some(OutputType::Ordinal) => {
            let i = integer().ok_or_else(|| not("an integer level"))?;
            match levels {
                Some(l) if i < 0 || i as u64 >= l => Err(format!("level {i} is outside 0..{l}")),
                _ => Ok(json!(i)),
            }
        }
        Some(OutputType::Score) => number()
            .map(|f| json!(f))
            .ok_or_else(|| not("a finite number")),
        None => boolean()
            .map(Value::Bool)
            .or_else(|| integer().map(|i| json!(i)))
            .or_else(|| number().map(|f| json!(f)))
            .ok_or_else(|| not("a boolean or a number")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csv::parse;
    use evalsig_core::records::{Observation, Record, Value as V};

    fn records(t: &Table, kind: RowKind) -> Vec<Record> {
        to_records(t, kind)
            .unwrap()
            .into_iter()
            .map(|(_, v)| Record::from_value(v).unwrap())
            .collect()
    }

    #[test]
    fn label_rows() {
        let t = parse("item_id,rater_id,value,labeled_at\nq-0001,user:dana,true,2026-08-03T00:00:00Z\nq-0002,user:dana,maybe,\n").unwrap();
        let r = records(&t, RowKind::Label);
        let Record::Label(a) = &r[0] else { panic!() };
        assert_eq!(a.value, Some(V::Bool(true)));
        assert_eq!(a.version, 1);
        let Record::Label(b) = &r[1] else { panic!() };
        assert!(
            matches!(b.observation().unwrap(), Observation::Missing(m) if m.reason == MissingReason::InvalidOutput)
        );
        assert_eq!(b.labeled_at, None);
    }

    #[test]
    fn judgment_rows_map_by_output_type() {
        let t = parse("item_id,run_id,output_type,levels,value\na,r,ordinal,4,3\nb,r,ordinal,4,7\nc,r,score,,0.5\nd,r,binary,,yes\n").unwrap();
        let r = records(&t, RowKind::Judgment);
        let obs: Vec<bool> = r
            .iter()
            .map(|x| match x {
                Record::Judgment(j) => matches!(j.observation().unwrap(), Observation::Value(_)),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(obs, [true, false, true, false]);
    }

    #[test]
    fn strict_columns() {
        let t = parse("item_id,rater_id,value,colour\na,u,1,red\n").unwrap();
        assert!(matches!(
            to_records(&t, RowKind::Label),
            Err(TableError::UnknownColumn { .. })
        ));
        let t = parse("item_id,value\na,1\n").unwrap();
        assert_eq!(
            to_records(&t, RowKind::Label),
            Err(TableError::MissingColumn("rater_id"))
        );
        let t = parse("item_id,rater_id,value,version\na,u,1,x\n").unwrap();
        assert!(matches!(
            to_records(&t, RowKind::Label),
            Err(TableError::Cell { line: 2, .. })
        ));
    }
}
