//! Declarative mappings (`mapping.toml`, `mapping/1`, D-011): external records to
//! `records/1` without a converter script.
//!
//! [`Mapping`] is the file as written; [`compile`] checks it, parses its paths and
//! lists any `DECIDE` left; [`apply`] turns source rows into record JSON, which the
//! caller validates like any other import.

pub mod path;

use std::collections::{BTreeMap, BTreeSet};

use evalsig_core::config::{Import, ImportAs};
use serde::Deserialize;
use serde_json::{Map, Value as Json, json};

use path::{Path, PathError, Scope, Template, text};

pub const SCHEMA: &str = "evalsig.mapping/1";
const DECIDE: &str = "DECIDE";

#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("not a mapping/1 file: {0}")]
    Parse(String),
    #[error("{at}: {source}")]
    Path {
        at: String,
        #[source]
        source: PathError,
    },
    #[error("{at}: {message}")]
    Invalid { at: String, message: String },
    #[error(
        "row {row}: does not match source.expect, so the records are in a format this mapping does not know"
    )]
    Unexpected { row: usize },
    #[error("{0}")]
    Read(String),
}

// ---- The file as written -------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Json,
    Jsonl,
    Csv,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ExprDef {
    Text(String),
    Table(ExprTable),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExprTable {
    pub path: Option<String>,
    pub template: Option<String>,
    pub arg: Option<String>,
    #[serde(rename = "const")]
    pub constant: Option<Json>,
    pub value_map: Option<BTreeMap<String, Json>>,
    pub default: Option<Json>,
    pub offset: Option<f64>,
    #[serde(rename = "type")]
    pub ty: Option<Ty>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ty {
    Boolean,
    Integer,
    Number,
    String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub path: String,
    pub exists: Option<bool>,
    pub equals: Option<Json>,
    #[serde(rename = "in")]
    pub one_of: Option<Vec<Json>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mapping {
    pub schema: String,
    pub name: String,
    pub source: Source,
    pub run: RunMap,
    pub item: ItemMap,
    pub judgment: Option<JudgmentMap>,
    pub label: Option<LabelMap>,
    #[serde(default)]
    pub decisions: Vec<Json>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub format: Format,
    pub records_path: Option<String>,
    pub fingerprint: Option<String>,
    pub expect: Option<Condition>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunMap {
    pub run_id: ExprDef,
    pub system: SystemMap,
    pub grader: Option<GraderMap>,
    pub value_semantics: String,
    pub output_type: Option<String>,
    pub levels: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemMap {
    pub id: ExprDef,
    pub version: ExprDef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraderMap {
    pub id: Option<ExprDef>,
    pub version: Option<ExprDef>,
    pub kind: Option<ExprDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemMap {
    pub id: ExprDef,
    pub cluster: Option<ExprDef>,
    #[serde(default)]
    pub strata: BTreeMap<String, ExprDef>,
    pub occurred_at: Option<ExprDef>,
    pub input_ref: Option<ExprDef>,
    pub reference_ref: Option<ExprDef>,
    pub weight: Option<ExprDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Explode {
    pub keys: Option<Vec<String>>,
    pub path: Option<String>,
    #[serde(rename = "as")]
    pub var: Option<String>,
    pub index: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Features {
    pub explode: String,
    pub feature_id: ExprDef,
    pub fired: ExprDef,
    pub strength: Option<ExprDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissingIf {
    pub when: Condition,
    pub reason: ExprDef,
    pub detail: Option<ExprDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgmentMap {
    pub explode: Option<Explode>,
    pub gen_replicate: Option<ExprDef>,
    pub grade_replicate: Option<ExprDef>,
    pub task: Option<ExprDef>,
    pub value: ExprDef,
    pub features: Option<Features>,
    pub output_ref: Option<ExprDef>,
    pub rationale_ref: Option<ExprDef>,
    #[serde(default)]
    pub cost: BTreeMap<String, ExprDef>,
    #[serde(default)]
    pub missing_if: Vec<MissingIf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Join {
    pub file: ExprDef,
    pub format: Option<Format>,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelMap {
    pub join: Option<Join>,
    pub rater_id: ExprDef,
    pub source: ExprDef,
    pub value: ExprDef,
    pub version: Option<ExprDef>,
    pub kind: Option<ExprDef>,
    pub labeled_at: Option<ExprDef>,
}

/// Parse a mapping file. A file always maps judgments.
pub fn parse(text: &str) -> Result<Mapping, MapError> {
    let m: Mapping = toml::from_str(text).map_err(|e| MapError::Parse(e.to_string()))?;
    if m.schema != SCHEMA {
        return Err(MapError::Parse(format!("schema must be {SCHEMA:?}")));
    }
    if m.judgment.is_none() {
        return Err(MapError::Parse("missing [judgment]".into()));
    }
    Ok(m)
}

// ---- Compiled form -------------------------------------------------------------

#[derive(Debug, Clone)]
enum Src {
    Path(Path),
    Template(Template),
    Arg(String),
    Const(Json),
    Current,
}

#[derive(Debug, Clone)]
pub struct Expr {
    at: String,
    src: Src,
    value_map: Option<BTreeMap<String, Json>>,
    default: Option<Json>,
    offset: Option<f64>,
    ty: Option<Ty>,
}

#[derive(Debug, Clone)]
struct Cond {
    path: Path,
    exists: Option<bool>,
    equals: Option<Json>,
    one_of: Option<Vec<Json>>,
}

#[derive(Debug, Clone)]
struct Run {
    run_id: Expr,
    system: (Expr, Expr),
    grader: Vec<(&'static str, Expr)>,
    value_semantics: String,
    output_type: Option<String>,
    levels: Option<u32>,
}

#[derive(Debug, Clone)]
struct Item {
    id: Expr,
    fields: Vec<(&'static str, Expr)>,
    strata: Vec<(String, Expr)>,
    bodies: Vec<(&'static str, Expr)>,
    emit: bool,
}

#[derive(Debug, Clone)]
enum Spread {
    None,
    Keys(Vec<String>),
    Path(Path),
}

#[derive(Debug, Clone)]
struct Judgment {
    spread: Spread,
    var: Option<String>,
    index: Option<String>,
    replicates: Vec<(&'static str, Expr)>,
    task: Option<Expr>,
    value: Expr,
    features: Option<(Path, Expr, Expr, Option<Expr>)>,
    bodies: Vec<(&'static str, Expr)>,
    cost: Vec<(String, Expr)>,
    missing_if: Vec<(Cond, Expr, Option<Expr>)>,
}

#[derive(Debug, Clone)]
struct Label {
    join: Option<(Expr, Format, Template)>,
    fields: Vec<(&'static str, Expr)>,
    value: Expr,
}

/// A checked mapping, ready to apply.
#[derive(Debug, Clone)]
pub struct Compiled {
    pub name: String,
    format: Format,
    records_path: Option<Path>,
    expect: Option<Cond>,
    run: Option<Run>,
    item: Item,
    judgment: Option<Judgment>,
    label: Option<Label>,
    /// Where `DECIDE` is still written.
    pub undecided: Vec<String>,
}

struct Compiler {
    undecided: Vec<String>,
}

impl Compiler {
    fn path(&self, at: &str, s: &str) -> Result<Path, MapError> {
        Path::parse(s).map_err(|source| MapError::Path {
            at: at.to_owned(),
            source,
        })
    }

    fn choice(&mut self, at: &str, v: &str) -> String {
        if v == DECIDE {
            self.undecided.push(at.to_owned());
        }
        v.to_owned()
    }

    fn expr(&mut self, at: &str, def: &ExprDef) -> Result<Expr, MapError> {
        let plain = |src| Expr {
            at: at.to_owned(),
            src,
            value_map: None,
            default: None,
            offset: None,
            ty: None,
        };
        let t = match def {
            ExprDef::Text(s) if s == DECIDE => {
                self.undecided.push(at.to_owned());
                return Ok(plain(Src::Const(Json::Null)));
            }
            ExprDef::Text(s) => return Ok(plain(Src::Path(self.path(at, s)?))),
            ExprDef::Table(t) => t,
        };
        let sources = [
            t.path.is_some(),
            t.template.is_some(),
            t.arg.is_some(),
            t.constant.is_some(),
        ];
        if sources.iter().filter(|s| **s).count() > 1 {
            return Err(MapError::Invalid {
                at: at.to_owned(),
                message: "give one of path, template, arg and const".into(),
            });
        }
        let src = if let Some(p) = &t.path {
            Src::Path(self.path(at, p)?)
        } else if let Some(s) = &t.template {
            Src::Template(Template::parse(s).map_err(|source| MapError::Path {
                at: at.to_owned(),
                source,
            })?)
        } else if let Some(a) = &t.arg {
            Src::Arg(a.clone())
        } else if let Some(c) = &t.constant {
            Src::Const(c.clone())
        } else {
            Src::Current
        };
        if let Some(m) = &t.value_map {
            for (k, v) in m {
                if v.as_str() == Some(DECIDE) {
                    self.undecided.push(format!("{at}.value_map.{k}"));
                }
            }
        }
        Ok(Expr {
            src,
            value_map: t.value_map.clone(),
            default: t.default.clone(),
            offset: t.offset,
            ty: t.ty,
            ..plain(Src::Current)
        })
    }

    fn opt(&mut self, at: &str, def: &Option<ExprDef>) -> Result<Option<Expr>, MapError> {
        def.as_ref().map(|d| self.expr(at, d)).transpose()
    }

    fn fields(
        &mut self,
        prefix: &str,
        defs: &[(&'static str, &Option<ExprDef>)],
    ) -> Result<Vec<(&'static str, Expr)>, MapError> {
        let mut out = Vec::new();
        for (name, def) in defs {
            if let Some(e) = self.opt(&format!("{prefix}.{name}"), def)? {
                out.push((*name, e));
            }
        }
        Ok(out)
    }

    fn cond(&self, at: &str, c: &Condition) -> Result<Cond, MapError> {
        Ok(Cond {
            path: self.path(at, &c.path)?,
            exists: c.exists,
            equals: c.equals.clone(),
            one_of: c.one_of.clone(),
        })
    }
}

/// Check a mapping and parse its paths.
pub fn compile(m: &Mapping) -> Result<Compiled, MapError> {
    let mut c = Compiler {
        undecided: Vec::new(),
    };
    let records_path = m
        .source
        .records_path
        .as_deref()
        .map(|p| c.path("source.records_path", p))
        .transpose()?;
    let expect = m
        .source
        .expect
        .as_ref()
        .map(|e| c.cond("source.expect", e))
        .transpose()?;
    let r = &m.run;
    let run = Run {
        run_id: c.expr("run.run_id", &r.run_id)?,
        system: (
            c.expr("run.system.id", &r.system.id)?,
            c.expr("run.system.version", &r.system.version)?,
        ),
        grader: match &r.grader {
            Some(g) => c.fields(
                "run.grader",
                &[("id", &g.id), ("version", &g.version), ("kind", &g.kind)],
            )?,
            None => Vec::new(),
        },
        value_semantics: c.choice("run.value_semantics", &r.value_semantics),
        output_type: r
            .output_type
            .as_deref()
            .map(|o| c.choice("run.output_type", o)),
        levels: r.levels,
    };
    let i = &m.item;
    let item = Item {
        id: c.expr("item.id", &i.id)?,
        fields: c.fields(
            "item",
            &[
                ("cluster", &i.cluster),
                ("occurred_at", &i.occurred_at),
                ("weight", &i.weight),
            ],
        )?,
        strata: i
            .strata
            .iter()
            .map(|(k, d)| Ok((k.clone(), c.expr(&format!("item.strata.{k}"), d)?)))
            .collect::<Result<_, MapError>>()?,
        bodies: c.fields(
            "item",
            &[
                ("input_ref", &i.input_ref),
                ("reference_ref", &i.reference_ref),
            ],
        )?,
        emit: true,
    };
    let judgment = m
        .judgment
        .as_ref()
        .map(|j| -> Result<Judgment, MapError> {
            let (spread, var, index) = match &j.explode {
                None => (Spread::None, None, None),
                Some(e) => {
                    let spread = match (&e.keys, &e.path) {
                        (Some(k), None) => Spread::Keys(k.clone()),
                        (None, Some(p)) => Spread::Path(c.path("judgment.explode.path", p)?),
                        _ => {
                            return Err(MapError::Invalid {
                                at: "judgment.explode".into(),
                                message: "give keys or path".into(),
                            });
                        }
                    };
                    let index = e
                        .index
                        .as_deref()
                        .map(|x| c.choice("judgment.explode.index", x));
                    if let Some(x) = index.as_deref()
                        && !matches!(x, "gen_replicate" | "grade_replicate" | DECIDE)
                    {
                        return Err(MapError::Invalid {
                            at: "judgment.explode.index".into(),
                            message: "must be gen_replicate or grade_replicate".into(),
                        });
                    }
                    (spread, e.var.clone(), index)
                }
            };
            let features = j
                .features
                .as_ref()
                .map(|f| -> Result<_, MapError> {
                    Ok((
                        c.path("judgment.features.explode", &f.explode)?,
                        c.expr("judgment.features.feature_id", &f.feature_id)?,
                        c.expr("judgment.features.fired", &f.fired)?,
                        c.opt("judgment.features.strength", &f.strength)?,
                    ))
                })
                .transpose()?;
            Ok(Judgment {
                spread,
                var,
                index,
                replicates: c.fields(
                    "judgment",
                    &[
                        ("gen_replicate", &j.gen_replicate),
                        ("grade_replicate", &j.grade_replicate),
                    ],
                )?,
                task: c.opt("judgment.task", &j.task)?,
                value: c.expr("judgment.value", &j.value)?,
                features,
                bodies: c.fields(
                    "judgment",
                    &[
                        ("output_ref", &j.output_ref),
                        ("rationale_ref", &j.rationale_ref),
                    ],
                )?,
                cost: j
                    .cost
                    .iter()
                    .map(|(k, d)| Ok((k.clone(), c.expr(&format!("judgment.cost.{k}"), d)?)))
                    .collect::<Result<_, MapError>>()?,
                missing_if: j
                    .missing_if
                    .iter()
                    .enumerate()
                    .map(|(n, mi)| {
                        let at = format!("judgment.missing_if[{n}]");
                        Ok((
                            c.cond(&format!("{at}.when"), &mi.when)?,
                            c.expr(&format!("{at}.reason"), &mi.reason)?,
                            c.opt(&format!("{at}.detail"), &mi.detail)?,
                        ))
                    })
                    .collect::<Result<_, MapError>>()?,
            })
        })
        .transpose()?;
    let label = m
        .label
        .as_ref()
        .map(|l| -> Result<Label, MapError> {
            let join = l
                .join
                .as_ref()
                .map(|j| -> Result<_, MapError> {
                    Ok((
                        c.expr("label.join.file", &j.file)?,
                        j.format.unwrap_or(Format::Json),
                        Template::parse(&j.path).map_err(|source| MapError::Path {
                            at: "label.join.path".into(),
                            source,
                        })?,
                    ))
                })
                .transpose()?;
            Ok(Label {
                join,
                fields: c.fields(
                    "label",
                    &[
                        ("rater_id", &Some(l.rater_id.clone())),
                        ("source", &Some(l.source.clone())),
                        ("version", &l.version),
                        ("kind", &l.kind),
                        ("labeled_at", &l.labeled_at),
                    ],
                )?,
                value: c.expr("label.value", &l.value)?,
            })
        })
        .transpose()?;
    Ok(Compiled {
        name: m.name.clone(),
        format: m.source.format,
        records_path,
        expect,
        run: Some(run),
        item,
        judgment,
        label,
        undecided: c.undecided,
    })
}

/// The mapping an `[[import]]` block of the configuration stands for: paths for
/// JSON sources, column names for CSV. A judgment block gives one run record per
/// run ID, with the block name as the system ID; a label block maps labels only.
pub fn from_import(import: &Import) -> Result<Compiled, MapError> {
    use evalsig_core::config::Adapter;
    let format = match import.adapter {
        Adapter::Jsonl => Format::Jsonl,
        Adapter::Csv => Format::Csv,
        other => {
            return Err(MapError::Invalid {
                at: format!("import {:?}", import.name),
                message: format!("adapter {other:?} is not a flat table"),
            });
        }
    };
    let m = &import.map;
    let at = |field: &str| format!("import {:?}: map.{field}", import.name);
    let path = |p: &str| match format {
        Format::Csv => format!("$.{p}"),
        _ => p.to_owned(),
    };
    let def = |p: &Option<String>| p.as_deref().map(|p| ExprDef::Text(path(p)));
    let need = |field: &str, p: &Option<String>| {
        def(p).ok_or_else(|| MapError::Invalid {
            at: at(field),
            message: "is required".into(),
        })
    };
    let value_map: Option<BTreeMap<String, Json>> = (!import.value_map.is_empty()).then(|| {
        import
            .value_map
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    serde_json::to_value(v).expect("values serialize"),
                )
            })
            .collect()
    });
    let value = ExprDef::Table(ExprTable {
        path: Some(path(m.value.as_deref().ok_or_else(|| {
            MapError::Invalid {
                at: at("value"),
                message: "is required".into(),
            }
        })?)),
        value_map,
        offset: m.offset.map(|o| o as f64),
        ..ExprTable::default()
    });
    let constant = |v: Json| {
        ExprDef::Table(ExprTable {
            constant: Some(v),
            ..ExprTable::default()
        })
    };
    let item = ItemMap {
        id: need("item_id", &m.item_id)?,
        cluster: None,
        strata: m
            .strata
            .iter()
            .map(|(k, p)| (k.clone(), ExprDef::Text(path(p))))
            .collect(),
        occurred_at: None,
        input_ref: def(&m.input_ref),
        reference_ref: None,
        weight: None,
    };
    let semantics = serde_json::to_value(import.value_semantics.unwrap_or_default())
        .expect("serializes")
        .as_str()
        .expect("a unit variant")
        .to_owned();
    let run = RunMap {
        run_id: need("run_id", &m.run_id).or_else(|e| match import.import_as {
            ImportAs::Label => Ok(constant(Json::Null)),
            ImportAs::Judgment => Err(e),
        })?,
        system: SystemMap {
            id: constant(json!(import.name)),
            version: constant(json!("unknown")),
        },
        grader: None,
        value_semantics: semantics,
        output_type: m.output_type.map(|o| {
            serde_json::to_value(o)
                .expect("serializes")
                .as_str()
                .expect("unit")
                .to_owned()
        }),
        levels: m.levels,
    };
    let (judgment, label) = match import.import_as {
        ImportAs::Judgment => (
            Some(JudgmentMap {
                explode: None,
                gen_replicate: def(&m.gen_replicate),
                grade_replicate: def(&m.grade_replicate),
                task: def(&m.task),
                value,
                features: None,
                output_ref: None,
                rationale_ref: None,
                cost: BTreeMap::new(),
                missing_if: Vec::new(),
            }),
            None,
        ),
        ImportAs::Label => (
            None,
            Some(LabelMap {
                join: None,
                rater_id: need("rater_id", &m.rater_id)?,
                source: constant(
                    serde_json::to_value(m.source.ok_or_else(|| MapError::Invalid {
                        at: at("source"),
                        message: "is required".into(),
                    })?)
                    .expect("serializes"),
                ),
                value,
                version: Some(constant(json!(m.version.unwrap_or(1)))),
                kind: None,
                labeled_at: def(&m.labeled_at),
            }),
        ),
    };
    let mapping = Mapping {
        schema: SCHEMA.into(),
        name: import.name.clone(),
        source: Source {
            format,
            records_path: None,
            fingerprint: None,
            expect: None,
        },
        run,
        item,
        judgment,
        label,
        decisions: Vec::new(),
    };
    let mut compiled = compile(&mapping)?;
    if import.import_as == ImportAs::Label {
        // Bare item records would replace richer ones imported before.
        compiled.item.emit = false;
        compiled.run = None;
    }
    Ok(compiled)
}

// ---- Applying ------------------------------------------------------------------

impl Compiled {
    /// Whether the source is JSON (rather than CSV).
    pub fn reads_json(&self) -> bool {
        self.format != Format::Csv
    }
}

/// What a mapping needs from the importer.
pub trait Host {
    fn arg(&self, name: &str) -> Option<&str>;
    /// The bytes of a joined file.
    fn read(&mut self, file: &str) -> Result<Vec<u8>, String>;
    /// Keep a body (an input, output, reference or rationale) and return its
    /// reference.
    fn body(&mut self, value: &Json) -> Result<String, String>;
}

/// A problem with one source row; the import stops on any.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub row: usize,
    pub at: String,
    pub message: String,
}

/// Values with the source row they come from, numbered from 1 (lines for JSONL and
/// CSV, elements for JSON).
pub type Numbered = Vec<(usize, Json)>;

/// The source rows of a file.
pub fn rows(c: &Compiled, bytes: &[u8]) -> Result<Numbered, MapError> {
    let text = std::str::from_utf8(bytes).map_err(|_| MapError::Read("not UTF-8".into()))?;
    let bad = |line: usize, e: serde_json::Error| MapError::Read(format!("line {line}: {e}"));
    match c.format {
        Format::Jsonl => text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, l)| Ok((i + 1, serde_json::from_str(l).map_err(|e| bad(i + 1, e))?)))
            .collect(),
        Format::Json => {
            let root: Json = serde_json::from_str(text).map_err(|e| bad(e.line(), e))?;
            let vars = BTreeMap::new();
            let scope = Scope {
                row: &root,
                current: None,
                vars: &vars,
            };
            let found = match &c.records_path {
                Some(p) => p.eval(scope),
                None => match root {
                    Json::Array(ref xs) => xs.clone(),
                    ref other => vec![other.clone()],
                },
            };
            Ok(found
                .into_iter()
                .enumerate()
                .map(|(i, v)| (i + 1, v))
                .collect())
        }
        Format::Csv => {
            let table = crate::csv::parse(text).map_err(|e| MapError::Read(e.to_string()))?;
            Ok(table
                .rows
                .iter()
                .map(|(line, cells)| {
                    let row: Map<String, Json> = table
                        .header
                        .iter()
                        .cloned()
                        .zip(cells.iter().map(|c| Json::String(c.clone())))
                        .collect();
                    (*line, Json::Object(row))
                })
                .collect())
        }
    }
}

/// Why an expression gave no value.
#[derive(Debug)]
enum Miss {
    /// The value is absent and there is no default.
    Absent,
    /// The value cannot be mapped (not in `value_map`, wrong type).
    Unmapped(String),
}

fn number(v: &Json) -> Option<f64> {
    match v {
        Json::Number(n) => n.as_f64(),
        Json::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn int(x: f64) -> Option<Json> {
    (x.fract() == 0.0 && x.abs() < 9_007_199_254_740_992.0).then(|| json!(x as i64))
}

impl Expr {
    fn eval(&self, scope: Scope<'_>, host: &dyn Host) -> Result<Json, Miss> {
        let raw = match &self.src {
            Src::Path(p) => p.first(scope),
            Src::Template(t) => t.render(scope).map(Json::String),
            Src::Arg(a) => host.arg(a).map(|v| Json::String(v.to_owned())),
            Src::Const(c) => Some(c.clone()),
            Src::Current => scope.current.cloned(),
        };
        let Some(raw) = raw.filter(|v| !v.is_null()) else {
            return self.default.clone().ok_or(Miss::Absent);
        };
        let shown = || text(&raw).unwrap_or_else(|| raw.to_string());
        let mut v = match &self.value_map {
            Some(m) => match text(&raw).and_then(|k| m.get(&k)) {
                Some(mapped) => mapped.clone(),
                None => return self.default.clone().ok_or_else(|| Miss::Unmapped(shown())),
            },
            None => raw.clone(),
        };
        if let Some(o) = self.offset {
            v = number(&v)
                .map(|x| x - o)
                .and_then(|x| int(x).or_else(|| serde_json::Number::from_f64(x).map(Json::Number)))
                .ok_or_else(|| Miss::Unmapped(shown()))?;
        }
        match self.ty {
            None => Ok(v),
            Some(Ty::Boolean) => match &v {
                Json::Bool(_) => Ok(v),
                Json::String(s) if s == "true" || s == "false" => Ok(json!(s == "true")),
                _ => Err(Miss::Unmapped(shown())),
            },
            Some(Ty::Integer) => number(&v)
                .and_then(int)
                .ok_or_else(|| Miss::Unmapped(shown())),
            Some(Ty::Number) => number(&v)
                .and_then(serde_json::Number::from_f64)
                .map(Json::Number)
                .ok_or_else(|| Miss::Unmapped(shown())),
            Some(Ty::String) => text(&v)
                .map(Json::String)
                .ok_or_else(|| Miss::Unmapped(shown())),
        }
    }

    /// A value the record cannot do without.
    fn need(&self, scope: Scope<'_>, host: &dyn Host) -> Result<Json, String> {
        self.eval(scope, host).map_err(|m| match m {
            Miss::Absent => format!("{}: no value", self.at),
            Miss::Unmapped(v) => format!("{}: cannot map {v:?}", self.at),
        })
    }

    /// An optional value: absent is `None`, unmappable is an error.
    fn maybe(&self, scope: Scope<'_>, host: &dyn Host) -> Result<Option<Json>, String> {
        match self.eval(scope, host) {
            Ok(v) => Ok(Some(v)),
            Err(Miss::Absent) => Ok(None),
            Err(Miss::Unmapped(v)) => Err(format!("{}: cannot map {v:?}", self.at)),
        }
    }

    fn id(&self, scope: Scope<'_>, host: &dyn Host) -> Result<String, String> {
        let v = self.need(scope, host)?;
        text(&v).ok_or_else(|| format!("{}: {v} is not text", self.at))
    }
}

impl Cond {
    fn holds(&self, scope: Scope<'_>) -> bool {
        let got = self.path.first(scope).filter(|v| !v.is_null());
        if let Some(e) = &self.equals {
            return got.as_ref() == Some(e);
        }
        if let Some(xs) = &self.one_of {
            return got.is_some_and(|g| xs.contains(&g));
        }
        got.is_some() == self.exists.unwrap_or(true)
    }
}

/// An integer that a numeric string may also give (CSV cells).
fn as_int(v: Json, at: &str) -> Result<Json, String> {
    number(&v)
        .and_then(int)
        .ok_or_else(|| format!("{at}: {v} is not an integer"))
}

/// A judgment value in the run's output type: CSV cells arrive as text.
fn typed_value(v: Json, output_type: Option<&str>) -> Option<Json> {
    match (output_type, &v) {
        (_, Json::Bool(_)) | (_, Json::Number(_)) => Some(v),
        (Some("binary"), Json::String(s)) => match s.as_str() {
            "true" => Some(json!(true)),
            "false" => Some(json!(false)),
            _ => None,
        },
        (Some("ordinal"), _) => number(&v).and_then(int),
        (Some("score") | None, _) => number(&v)
            .and_then(serde_json::Number::from_f64)
            .map(Json::Number),
        _ => None,
    }
}

struct Applier<'h> {
    host: &'h mut dyn Host,
    joined: BTreeMap<String, Json>,
    labelled: BTreeSet<String>,
    records: Vec<(usize, Json)>,
}

impl Applier<'_> {
    fn bodies(
        &mut self,
        fields: &[(&'static str, Expr)],
        scope: Scope<'_>,
        out: &mut Map<String, Json>,
    ) -> Result<(), String> {
        for (name, e) in fields {
            if let Some(v) = e.maybe(scope, &*self.host)? {
                let r = self.host.body(&v).map_err(|m| format!("{}: {m}", e.at))?;
                out.insert((*name).into(), json!(r));
            }
        }
        Ok(())
    }

    fn joined(&mut self, file: &str, format: Format) -> Result<&Json, String> {
        if !self.joined.contains_key(file) {
            let bytes = self.host.read(file)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| format!("{file}: not UTF-8"))?;
            let root = match format {
                Format::Json => serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?,
                Format::Jsonl => Json::Array(
                    text.lines()
                        .filter(|l| !l.trim().is_empty())
                        .map(|l| serde_json::from_str(l).map_err(|e| format!("{file}: {e}")))
                        .collect::<Result<_, _>>()?,
                ),
                Format::Csv => {
                    let t = crate::csv::parse(text).map_err(|e| format!("{file}: {e}"))?;
                    Json::Array(
                        t.rows
                            .iter()
                            .map(|(_, cells)| {
                                Json::Object(
                                    t.header
                                        .iter()
                                        .cloned()
                                        .zip(cells.iter().map(|c| json!(c)))
                                        .collect(),
                                )
                            })
                            .collect(),
                    )
                }
            };
            self.joined.insert(file.to_owned(), root);
        }
        Ok(&self.joined[file])
    }

    fn row(&mut self, c: &Compiled, n: usize, row: &Json) -> Result<(), String> {
        let vars = BTreeMap::new();
        let scope = Scope {
            row,
            current: None,
            vars: &vars,
        };
        let host = &*self.host;
        let mut run_meta = (None, None);
        let mut run_id = Json::Null;
        if let Some(r) = &c.run {
            let id = r.run_id.id(scope, host)?;
            let mut run = Map::new();
            run.insert("record".into(), json!("run"));
            run.insert("run_id".into(), json!(id));
            run.insert(
                "system".into(),
                json!({"id": r.system.0.id(scope, host)?, "version": r.system.1.id(scope, host)?}),
            );
            let mut grader = Map::new();
            for (k, e) in &r.grader {
                if let Some(v) = e.maybe(scope, host)? {
                    grader.insert((*k).into(), v);
                }
            }
            if !grader.is_empty() {
                run.insert("grader".into(), Json::Object(grader));
            }
            run.insert("value_semantics".into(), json!(r.value_semantics));
            if let Some(o) = &r.output_type {
                run.insert("output_type".into(), json!(o));
            }
            if let Some(l) = r.levels {
                run.insert("levels".into(), json!(l));
            }
            run_meta = (r.output_type.clone(), r.levels);
            run_id = json!(id);
            self.records.push((n, Json::Object(run)));
        }
        let item_id = c.item.id.id(scope, host)?;
        if c.item.emit {
            let mut item = Map::new();
            item.insert("record".into(), json!("item"));
            item.insert("item_id".into(), json!(item_id));
            for (k, e) in &c.item.fields {
                if let Some(v) = e.maybe(scope, host)? {
                    item.insert((*k).into(), v);
                }
            }
            let mut strata = Map::new();
            for (k, e) in &c.item.strata {
                if let Some(v) = e.maybe(scope, host)? {
                    strata.insert(k.clone(), v);
                }
            }
            if !strata.is_empty() {
                item.insert("strata".into(), Json::Object(strata));
            }
            self.bodies(&c.item.bodies, scope, &mut item)?;
            self.records.push((n, Json::Object(item)));
        }
        if let Some(l) = &c.label {
            self.label(l, n, scope, &item_id)?;
        }
        if let Some(j) = &c.judgment {
            self.judgments(j, n, scope, &item_id, &run_id, &run_meta)?;
        }
        Ok(())
    }

    fn label(
        &mut self,
        l: &Label,
        n: usize,
        scope: Scope<'_>,
        item_id: &str,
    ) -> Result<(), String> {
        let joined = match &l.join {
            None => None,
            Some((file, format, path)) => {
                if self.labelled.contains(item_id) {
                    return Ok(());
                }
                let file = file.id(scope, &*self.host)?;
                // Insertions read the row; the resulting path reads the joined file.
                let Some(p) = path.render(scope) else {
                    return Ok(());
                };
                let p = Path::parse(&p).map_err(|e| format!("label.join.path: {e}"))?;
                let root = self.joined(&file, *format)?.clone();
                let none = BTreeMap::new();
                let found = p.first(Scope {
                    row: &root,
                    current: None,
                    vars: &none,
                });
                match found {
                    Some(v) => Some(v),
                    None => return Ok(()),
                }
            }
        };
        let scope = match &joined {
            Some(v) => scope.with_current(v),
            None => scope,
        };
        let host = &*self.host;
        let mut label = Map::new();
        label.insert("record".into(), json!("label"));
        label.insert("item_id".into(), json!(item_id));
        for (k, e) in &l.fields {
            let v = match *k {
                "rater_id" | "source" => Some(e.need(scope, host)?),
                "version" => e
                    .maybe(scope, host)?
                    .map(|v| as_int(v, &e.at))
                    .transpose()?,
                _ => e.maybe(scope, host)?,
            };
            if let Some(v) = v {
                label.insert((*k).into(), v);
            }
        }
        label.entry("version").or_insert(json!(1));
        match l.value.eval(scope, host) {
            Ok(v) => {
                label.insert("value".into(), v);
            }
            Err(m) => {
                label.insert("missing".into(), invalid_output(&l.value, m));
            }
        }
        self.labelled.insert(item_id.to_owned());
        self.records.push((n, Json::Object(label)));
        Ok(())
    }

    fn judgments(
        &mut self,
        j: &Judgment,
        n: usize,
        scope: Scope<'_>,
        item_id: &str,
        run_id: &Json,
        (output_type, levels): &(Option<String>, Option<u32>),
    ) -> Result<(), String> {
        // Each element: the variable's value, its position and the element itself.
        let elements: Vec<(Option<String>, usize, Option<Json>)> = match &j.spread {
            Spread::None => vec![(None, 0, None)],
            Spread::Keys(keys) => keys
                .iter()
                .enumerate()
                .map(|(i, k)| (Some(k.clone()), i, None))
                .collect(),
            Spread::Path(p) => match p.first(scope) {
                Some(Json::Array(xs)) => xs
                    .into_iter()
                    .enumerate()
                    .map(|(i, x)| (Some(i.to_string()), i, Some(x)))
                    .collect(),
                Some(Json::Object(m)) => m
                    .into_iter()
                    .enumerate()
                    .map(|(i, (k, x))| (Some(k), i, Some(x)))
                    .collect(),
                _ => Vec::new(),
            },
        };
        for (var, index, element) in elements {
            let mut vars = BTreeMap::new();
            if let (Some(name), Some(v)) = (&j.var, var) {
                vars.insert(name.clone(), v);
            }
            let mut scope = Scope {
                vars: &vars,
                ..scope
            };
            if let Some(e) = &element {
                scope = scope.with_current(e);
            }
            let host = &*self.host;
            let mut out = Map::new();
            out.insert("record".into(), json!("judgment"));
            out.insert("item_id".into(), json!(item_id));
            out.insert("run_id".into(), run_id.clone());
            out.insert("gen_replicate".into(), json!(0));
            for (k, e) in &j.replicates {
                if let Some(v) = e.maybe(scope, host)? {
                    out.insert((*k).into(), as_int(v, &e.at)?);
                }
            }
            if let Some(field) = &j.index {
                out.insert(field.clone(), json!(index));
            }
            if let Some(t) = &j.task
                && let Some(v) = t.maybe(scope, host)?
            {
                out.insert("task".into(), v);
            }
            if let Some(o) = output_type {
                out.insert("output_type".into(), json!(o));
            }
            if let Some(l) = levels {
                out.insert("levels".into(), json!(l));
            }
            let mut missing = None;
            for (cond, reason, detail) in &j.missing_if {
                if cond.holds(scope) {
                    let mut m = Map::new();
                    m.insert("reason".into(), reason.need(scope, host)?);
                    if let Some(d) = detail
                        && let Some(v) = d.maybe(scope, host)?
                    {
                        m.insert("detail".into(), v);
                    }
                    missing = Some(Json::Object(m));
                    break;
                }
            }
            if missing.is_none() {
                match j.value.eval(scope, host) {
                    Ok(v) => match typed_value(v.clone(), output_type.as_deref()) {
                        Some(v) => {
                            out.insert("value".into(), v);
                        }
                        None => {
                            let shown = text(&v).unwrap_or_else(|| v.to_string());
                            missing = Some(invalid_output(&j.value, Miss::Unmapped(shown)));
                        }
                    },
                    Err(m) => missing = Some(invalid_output(&j.value, m)),
                }
            }
            match missing {
                Some(m) => {
                    out.insert("missing".into(), m);
                }
                None => {
                    if let Some((path, id, fired, strength)) = &j.features {
                        let mut list = Vec::new();
                        for f in path.eval(scope) {
                            let s = scope.with_current(&f);
                            let mut feature = Map::new();
                            feature.insert("feature_id".into(), json!(id.id(s, host)?));
                            let on = fired.need(s, host)?;
                            feature.insert(
                                "fired".into(),
                                json!(on.as_bool().ok_or_else(|| format!(
                                    "{}: {on} is not a boolean",
                                    fired.at
                                ))?),
                            );
                            if let Some(st) = strength
                                && let Some(v) = st.maybe(s, host)?
                            {
                                feature.insert("strength".into(), v);
                            }
                            list.push(Json::Object(feature));
                        }
                        out.insert("features".into(), Json::Array(list));
                    }
                }
            }
            let mut cost = Map::new();
            for (k, e) in &j.cost {
                if let Some(v) = e.maybe(scope, host)? {
                    cost.insert(k.clone(), v);
                }
            }
            if !cost.is_empty() {
                out.insert("cost".into(), Json::Object(cost));
            }
            self.bodies(&j.bodies, scope, &mut out)?;
            self.records.push((n, Json::Object(out)));
        }
        Ok(())
    }
}

fn invalid_output(e: &Expr, m: Miss) -> Json {
    let detail = match m {
        Miss::Absent => format!("{}: no value", e.at),
        Miss::Unmapped(v) => format!("{}: {v:?} is not mapped", e.at),
    };
    json!({"reason": "invalid_output", "detail": detail})
}

/// The records of the source rows, each with its row number, in row order. Stops at
/// the first row that `source.expect` rejects; other problems are listed per row.
pub fn apply(
    c: &Compiled,
    rows: &[(usize, Json)],
    host: &mut dyn Host,
) -> Result<(Numbered, Vec<Issue>), MapError> {
    if let Some(at) = c.undecided.first() {
        return Err(MapError::Invalid {
            at: at.clone(),
            message: "is still DECIDE".into(),
        });
    }
    let mut a = Applier {
        host,
        joined: BTreeMap::new(),
        labelled: BTreeSet::new(),
        records: Vec::new(),
    };
    let mut issues = Vec::new();
    let none = BTreeMap::new();
    for (n, row) in rows {
        if let Some(e) = &c.expect
            && !e.holds(Scope {
                row,
                current: None,
                vars: &none,
            })
        {
            return Err(MapError::Unexpected { row: *n });
        }
        if let Err(message) = a.row(c, *n, row) {
            let (at, message) = match message.split_once(": ") {
                Some((at, m)) => (at.to_owned(), m.to_owned()),
                None => (String::new(), message),
            };
            issues.push(Issue {
                row: *n,
                at,
                message,
            });
        }
    }
    Ok((a.records, issues))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Args(BTreeMap<String, String>);

    impl Host for Args {
        fn arg(&self, name: &str) -> Option<&str> {
            self.0.get(name).map(String::as_str)
        }
        fn read(&mut self, file: &str) -> Result<Vec<u8>, String> {
            Err(format!("{file}: no files in this test"))
        }
        fn body(&mut self, value: &Json) -> Result<String, String> {
            Ok(format!("body:{value}"))
        }
    }

    fn expr(def: Json) -> Expr {
        let def: ExprDef = serde_json::from_value(def).unwrap();
        Compiler {
            undecided: Vec::new(),
        }
        .expr("x", &def)
        .unwrap()
    }

    fn eval(e: &Expr, row: &Json) -> Result<Json, Miss> {
        let vars = BTreeMap::new();
        let scope = Scope {
            row,
            current: None,
            vars: &vars,
        };
        e.eval(scope, &Args(BTreeMap::from([("id".into(), "r-1".into())])))
    }

    #[test]
    fn expressions_map_offset_and_type_values() {
        let row = json!({"v": "high", "n": 3, "flag": "true", "s": "2"});
        let mapped = expr(json!({"path": "$.v", "value_map": {"high": 2}}));
        assert_eq!(eval(&mapped, &row).unwrap(), json!(2));
        let unmapped = expr(json!({"path": "$.v", "value_map": {"low": 0}}));
        assert!(matches!(eval(&unmapped, &row), Err(Miss::Unmapped(v)) if v == "high"));
        let fallback = expr(json!({"path": "$.v", "value_map": {"low": 0}, "default": 9}));
        assert_eq!(eval(&fallback, &row).unwrap(), json!(9));
        assert_eq!(
            eval(&expr(json!({"path": "$.n", "offset": 1})), &row).unwrap(),
            json!(2)
        );
        assert_eq!(
            eval(&expr(json!({"path": "$.flag", "type": "boolean"})), &row).unwrap(),
            json!(true)
        );
        assert_eq!(
            eval(&expr(json!({"path": "$.s", "type": "integer"})), &row).unwrap(),
            json!(2)
        );
        assert_eq!(
            eval(&expr(json!({"arg": "id"})), &row).unwrap(),
            json!("r-1")
        );
        assert!(matches!(
            eval(&expr(json!("$.absent")), &row),
            Err(Miss::Absent)
        ));
    }

    #[test]
    fn decide_is_listed_wherever_it_is_written() {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/mapping.toml"
        ))
        .unwrap()
        .replace(r#"index = "gen_replicate""#, r#"index = "DECIDE""#)
        .replace("none = 0", r#"none = "DECIDE""#);
        let c = compile(&parse(&text).unwrap()).unwrap();
        assert_eq!(
            c.undecided,
            ["judgment.explode.index", "judgment.value.value_map.none"]
        );
        let rows = vec![(1, json!({"schema": "triage-run/1"}))];
        assert!(apply(&c, &rows, &mut Args(BTreeMap::new())).is_err());
    }

    #[test]
    fn conditions_test_presence_equality_and_membership() {
        let row = json!({"a": 1, "b": null});
        let vars = BTreeMap::new();
        let scope = Scope {
            row: &row,
            current: None,
            vars: &vars,
        };
        let c = Compiler {
            undecided: Vec::new(),
        };
        let cond = |v: Json| c.cond("w", &serde_json::from_value(v).unwrap()).unwrap();
        assert!(cond(json!({"path": "$.a"})).holds(scope));
        assert!(cond(json!({"path": "$.b", "exists": false})).holds(scope));
        assert!(cond(json!({"path": "$.a", "equals": 1})).holds(scope));
        assert!(!cond(json!({"path": "$.a", "in": [2, 3]})).holds(scope));
    }

    #[test]
    fn flat_blocks_need_their_keys() {
        let config = evalsig_core::config::Config::from_toml(
            "[[import]]\nname = \"x\"\nadapter = \"jsonl\"\npath = \"a.jsonl\"\nmap.value = \"$.v\"\n",
        )
        .unwrap();
        let e = from_import(&config.imports[0]).unwrap_err().to_string();
        assert!(e.contains("map.item_id"), "{e}");
    }
}
