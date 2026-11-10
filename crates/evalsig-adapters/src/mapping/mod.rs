//! Declarative mappings (`mapping.toml`, `mapping/1`, D-011): external records to
//! `records/1` without a converter script.
//!
//! [`Mapping`] is the file as written; [`compile`] checks it, parses its paths and
//! lists any `DECIDE` left; [`apply`] turns source rows into record JSON, which the
//! caller validates like any other import.

pub mod path;

use std::collections::{BTreeMap, BTreeSet};

use evalsig_core::config::{Import, ImportAs};
use evalsig_core::records::OutputType;
use serde::Deserialize;
use serde_json::{Map, Value as Json, json};

use path::{Path, PathError, Scope, Template, text};

use crate::BodySink;
use crate::limits::Limits;
use crate::source;
pub use crate::source::Numbered;

pub const SCHEMA: &str = "evalsig.mapping/1";
pub const DECIDE: &str = "DECIDE";

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
    /// Per feature ID, its `expected` class and `lineage` (D-049).
    pub features: Option<BTreeMap<String, evalsig_core::records::FeatureMeta>>,
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
    pub filter: Option<Condition>,
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
    pub path: Option<String>,
    pub key: Option<JoinKey>,
}

/// Matches label rows to source rows by a key (D-047).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinKey {
    /// The key of a label row, over `@`.
    pub label: String,
    /// The key of a source row, over `$`.
    pub row: String,
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
    features: Option<BTreeMap<String, evalsig_core::records::FeatureMeta>>,
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
    /// Keeps the exploded elements or keys the condition accepts (D-046).
    filter: Option<Cond>,
    features: Option<(Path, Expr, Expr, Option<Expr>)>,
    bodies: Vec<(&'static str, Expr)>,
    cost: Vec<(String, Expr)>,
    missing_if: Vec<(Cond, Expr, Option<Expr>)>,
}

/// How a joined file's label is found for a row.
#[derive(Debug, Clone)]
enum Lookup {
    /// A path through the joined file, its insertions reading the row.
    Path(Path),
    /// Every label row whose key equals the row's (D-047).
    Key { label: Template, row: Template },
}

#[derive(Debug, Clone)]
struct Label {
    join: Option<(Expr, Format, Lookup)>,
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

    fn template(&self, at: &str, s: &str) -> Result<Template, MapError> {
        Template::parse(s).map_err(|source| MapError::Path {
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
            Src::Template(self.template(at, s)?)
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
    compile_parts(m, true)
}

/// Compile a mapping; without `judgments`, only its labels are mapped.
fn compile_parts(m: &Mapping, judgments: bool) -> Result<Compiled, MapError> {
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
        features: r.features.clone(),
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
        emit: judgments,
    };
    let judgment = m
        .judgment
        .as_ref()
        .map(|j| -> Result<Judgment, MapError> {
            let filter = j
                .explode
                .as_ref()
                .and_then(|e| e.filter.as_ref())
                .map(|f| c.cond("judgment.explode.filter", f))
                .transpose()?;
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
                filter,
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
                    let file = c.expr("label.join.file", &j.file)?;
                    // Source rows must not choose which local file is read.
                    if !matches!(file.src, Src::Arg(_) | Src::Const(_)) {
                        return Err(MapError::Invalid {
                            at: "label.join.file".into(),
                            message: "must be an arg or a const".into(),
                        });
                    }
                    let lookup = match (&j.path, &j.key) {
                        (Some(p), None) => Lookup::Path(c.path("label.join.path", p)?),
                        (None, Some(k)) => {
                            // One index serves every row, so the label key reads the label
                            // row alone; the row key reads the source row alone.
                            let key = |at: &str, s: &str, row: bool, what: &str| {
                                let t = c.template(at, s)?;
                                if t.reads_only(row) {
                                    Ok(t)
                                } else {
                                    Err(MapError::Invalid {
                                        at: at.into(),
                                        message: format!("may insert only {what}"),
                                    })
                                }
                            };
                            Lookup::Key {
                                label: key(
                                    "label.join.key.label",
                                    &k.label,
                                    false,
                                    "@ paths of the label row",
                                )?,
                                row: key(
                                    "label.join.key.row",
                                    &k.row,
                                    true,
                                    "$ paths of the source row",
                                )?,
                            }
                        }
                        _ => {
                            return Err(MapError::Invalid {
                                at: "label.join".into(),
                                message: "give path or key".into(),
                            });
                        }
                    };
                    Ok((file, j.format.unwrap_or(Format::Json), lookup))
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
        run: judgments.then_some(run),
        item,
        judgment,
        label,
        undecided: c.undecided,
    })
}

/// The serde name of a unit enum variant.
fn unit_name(v: &impl serde::Serialize) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
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
        // A column name is a key of the row, whatever characters it has.
        Format::Csv => format!("$[{}]", json!(p)),
        _ => p.to_owned(),
    };
    let def = |p: &Option<String>| p.as_deref().map(|p| ExprDef::Text(path(p)));
    let need = |field: &str, p: &Option<String>| {
        def(p).ok_or_else(|| MapError::Invalid {
            at: at(field),
            message: "is required".into(),
        })
    };
    let value_map = (!import.value_map.is_empty()).then(|| import.value_map_json());
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
    let semantics = import
        .value_semantics
        .unwrap_or_default()
        .as_str()
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
        output_type: m.output_type.map(|o| unit_name(&o)),
        levels: m.levels,
        features: None,
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
                source: constant(json!(unit_name(&m.source.ok_or_else(|| {
                    MapError::Invalid {
                        at: at("source"),
                        message: "is required".into(),
                    }
                })?))),
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
    // A label block maps labels only: bare item records would replace richer ones
    // imported before, and there is no run.
    compile_parts(&mapping, import.import_as == ImportAs::Judgment)
}

// ---- Applying ------------------------------------------------------------------

impl Compiled {
    /// Whether the mapping maps reference labels.
    pub fn maps_labels(&self) -> bool {
        self.label.is_some()
    }

    /// Every path the mapping reads from source rows, insertions and filter tests
    /// included. The join path reads the joined file, not the row, and is left out.
    pub fn read_paths(&self) -> Vec<&Path> {
        fn expr<'c>(e: &'c Expr, out: &mut Vec<&'c Path>) {
            match &e.src {
                Src::Path(p) => out.push(p),
                Src::Template(t) => out.extend(t.paths()),
                _ => {}
            }
        }
        let mut out = Vec::new();
        let mut exprs: Vec<&Expr> = Vec::new();
        if let Some(r) = &self.run {
            exprs.extend([&r.run_id, &r.system.0, &r.system.1]);
            exprs.extend(r.grader.iter().map(|(_, e)| e));
        }
        let i = &self.item;
        exprs.push(&i.id);
        exprs.extend(i.fields.iter().chain(&i.bodies).map(|(_, e)| e));
        exprs.extend(i.strata.iter().map(|(_, e)| e));
        if let Some(e) = &self.expect {
            out.push(&e.path);
        }
        if let Some(j) = &self.judgment {
            if let Spread::Path(p) = &j.spread {
                out.push(p);
            }
            exprs.extend(j.replicates.iter().chain(&j.bodies).map(|(_, e)| e));
            exprs.extend(j.cost.iter().map(|(_, e)| e));
            exprs.extend(j.task.iter());
            exprs.push(&j.value);
            out.extend(j.filter.iter().map(|c| &c.path));
            if let Some((p, id, fired, strength)) = &j.features {
                out.push(p);
                exprs.extend([id, fired].into_iter().chain(strength));
            }
            for (cond, reason, detail) in &j.missing_if {
                out.push(&cond.path);
                exprs.extend(std::iter::once(reason).chain(detail));
            }
        }
        if let Some(l) = &self.label {
            exprs.extend(l.fields.iter().map(|(_, e)| e).chain([&l.value]));
            if let Some((file, _, lookup)) = &l.join {
                exprs.push(file);
                match lookup {
                    Lookup::Path(path) => out.extend(path.inner()),
                    Lookup::Key { row, .. } => out.extend(row.paths()),
                }
            }
        }
        for e in exprs {
            expr(e, &mut out);
        }
        // Insertions and filter tests are paths too.
        let mut i = 0;
        while i < out.len() {
            let inner = out[i].inner();
            out.extend(inner);
            i += 1;
        }
        out
    }
}

/// What a mapping needs from the importer besides keeping bodies.
pub trait Host: BodySink {
    fn arg(&self, name: &str) -> Option<&str>;
    /// The bytes of a joined file.
    fn read(&mut self, file: &str) -> Result<Vec<u8>, String>;
    /// The limits joined files are parsed within.
    fn limits(&self) -> Limits {
        Limits::default()
    }
}

/// A problem with one source row; the import stops on any.
#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub row: usize,
    pub at: String,
    pub message: String,
}

/// A problem found while mapping a row: where in the mapping, and what.
struct RowError {
    at: String,
    message: String,
}

fn row_error(at: &str, message: impl Into<String>) -> RowError {
    RowError {
        at: at.to_owned(),
        message: message.into(),
    }
}

/// The source rows of a file.
pub fn rows(c: &Compiled, bytes: &[u8], limits: &Limits) -> Result<Numbered, MapError> {
    let rows = source::parse(c.format, bytes, limits).map_err(MapError::Read)?;
    if c.format != Format::Json {
        return Ok(rows);
    }
    let root = rows
        .into_iter()
        .next()
        .map(|(_, v)| v)
        .unwrap_or(Json::Null);
    Ok(match &c.records_path {
        Some(p) => {
            let vars = BTreeMap::new();
            let found = p.eval(Scope {
                row: &root,
                current: None,
                vars: &vars,
            });
            source::elements(Json::Array(found.into_iter().cloned().collect()))
        }
        None => source::elements(root),
    })
}

/// Why an expression gave no value.
#[derive(Debug)]
enum Miss {
    /// The value is absent and there is no default.
    Absent,
    /// The value cannot be mapped (not in `value_map`, wrong type).
    Unmapped(String),
}

fn shown(v: &Json) -> String {
    text(v).unwrap_or_else(|| v.to_string())
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
        let owned;
        let raw: Option<&Json> = match &self.src {
            Src::Path(p) => p.first(scope),
            Src::Template(t) => {
                owned = t.render(scope).map(Json::String);
                owned.as_ref()
            }
            Src::Arg(a) => {
                owned = host.arg(a).map(|v| Json::String(v.to_owned()));
                owned.as_ref()
            }
            Src::Const(c) => Some(c),
            Src::Current => scope.current,
        };
        let Some(raw) = raw.filter(|v| !v.is_null()) else {
            return self.default.clone().ok_or(Miss::Absent);
        };
        let unmapped = || Miss::Unmapped(shown(raw));
        let mut v = match &self.value_map {
            Some(m) => match text(raw).and_then(|k| m.get(&k)) {
                Some(mapped) => mapped.clone(),
                None => return self.default.clone().ok_or_else(unmapped),
            },
            None => raw.clone(),
        };
        if let Some(o) = self.offset {
            v = number(&v)
                .map(|x| x - o)
                .and_then(|x| int(x).or_else(|| serde_json::Number::from_f64(x).map(Json::Number)))
                .ok_or_else(unmapped)?;
        }
        match self.ty {
            None => Ok(v),
            Some(Ty::Boolean) => match &v {
                Json::Bool(_) => Ok(v),
                Json::String(s) if s == "true" || s == "false" => Ok(json!(s == "true")),
                _ => Err(unmapped()),
            },
            Some(Ty::Integer) => number(&v).and_then(int).ok_or_else(unmapped),
            Some(Ty::Number) => number(&v)
                .and_then(serde_json::Number::from_f64)
                .map(Json::Number)
                .ok_or_else(unmapped),
            Some(Ty::String) => text(&v).map(Json::String).ok_or_else(unmapped),
        }
    }

    /// A value the record cannot do without.
    fn need(&self, scope: Scope<'_>, host: &dyn Host) -> Result<Json, RowError> {
        self.eval(scope, host).map_err(|m| match m {
            Miss::Absent => row_error(&self.at, "no value"),
            Miss::Unmapped(v) => row_error(&self.at, format!("cannot map {v:?}")),
        })
    }

    /// An optional value: absent is `None`, unmappable is an error.
    fn maybe(&self, scope: Scope<'_>, host: &dyn Host) -> Result<Option<Json>, RowError> {
        match self.eval(scope, host) {
            Ok(v) => Ok(Some(v)),
            Err(Miss::Absent) => Ok(None),
            Err(Miss::Unmapped(v)) => Err(row_error(&self.at, format!("cannot map {v:?}"))),
        }
    }

    fn id(&self, scope: Scope<'_>, host: &dyn Host) -> Result<String, RowError> {
        let v = self.need(scope, host)?;
        text(&v).ok_or_else(|| row_error(&self.at, format!("{v} is not text")))
    }

    /// An integer that a numeric string may also give (CSV cells).
    fn int(&self, scope: Scope<'_>, host: &dyn Host) -> Result<Option<Json>, RowError> {
        self.maybe(scope, host)?
            .map(|v| {
                number(&v)
                    .and_then(int)
                    .ok_or_else(|| row_error(&self.at, format!("{v} is not an integer")))
            })
            .transpose()
    }
}

/// The values of these named expressions that are present.
fn collect<K: Clone + Into<String>>(
    fields: &[(K, Expr)],
    scope: Scope<'_>,
    host: &dyn Host,
) -> Result<Map<String, Json>, RowError> {
    let mut out = Map::new();
    for (k, e) in fields {
        if let Some(v) = e.maybe(scope, host)? {
            out.insert(k.clone().into(), v);
        }
    }
    Ok(out)
}

fn insert_nonempty(out: &mut Map<String, Json>, key: &str, m: Map<String, Json>) {
    if !m.is_empty() {
        out.insert(key.into(), Json::Object(m));
    }
}

impl Cond {
    fn holds(&self, scope: Scope<'_>) -> bool {
        let got = self.path.first(scope).filter(|v| !v.is_null());
        if let Some(e) = &self.equals {
            return got.is_some_and(|g| same(g, e));
        }
        if let Some(xs) = &self.one_of {
            return got.is_some_and(|g| xs.iter().any(|x| same(g, x)));
        }
        got.is_some() == self.exists.unwrap_or(true)
    }
}

/// Equal as JSON, as numbers (`1` and `1.0`, or a CSV cell `"2"` and `2`), or as text.
fn same(a: &Json, b: &Json) -> bool {
    a == b
        || matches!((number(a), number(b)), (Some(x), Some(y)) if x == y)
        || matches!((text(a), text(b)), (Some(x), Some(y)) if x == y)
}

/// A judgment value in the run's output type, or `None` if it is not one: text cells
/// (CSV) convert as in a plain CSV import, and a value of the wrong type is unmapped.
fn typed_value(v: &Json, output_type: Option<OutputType>, levels: Option<u32>) -> Option<Json> {
    let levels = levels.map(u64::from);
    match v {
        Json::String(s) => crate::table::map_value(s, output_type, levels).ok(),
        Json::Bool(_) | Json::Number(_) => {
            // An integral float is the integer (`3.0` is level 3), as in `records/1`.
            let text = number(v)
                .and_then(int)
                .map_or_else(|| v.to_string(), |i| i.to_string());
            crate::table::map_value(&text, output_type, levels).ok()
        }
        _ => None,
    }
}

fn invalid_output(e: &Expr, m: Miss) -> Json {
    let detail = match m {
        Miss::Absent => format!("{}: no value", e.at),
        Miss::Unmapped(v) => format!("{}: {v:?} is not mapped", e.at),
    };
    json!({"reason": "invalid_output", "detail": detail})
}

struct Applier<'h> {
    host: &'h mut dyn Host,
    joined: BTreeMap<String, Json>,
    /// Per joined file read by key: the positions of its label rows by key.
    keyed: BTreeMap<String, BTreeMap<String, Vec<usize>>>,
    /// The (file, key, item) triples already joined, so rows sharing them label once.
    keys_joined: BTreeSet<String>,
    labelled: BTreeSet<String>,
    /// Run and item records already emitted; rows repeat them. Differing ones are
    /// emitted so the importer reports the conflict.
    emitted: BTreeSet<String>,
    records: Vec<(usize, Json)>,
    /// Elements `explode.filter` left out.
    filtered: usize,
}

impl Applier<'_> {
    fn push_once(&mut self, n: usize, record: Json) {
        if self.emitted.insert(record.to_string()) {
            self.records.push((n, record));
        }
    }

    fn bodies(
        &mut self,
        fields: &[(&'static str, Expr)],
        scope: Scope<'_>,
        out: &mut Map<String, Json>,
    ) -> Result<(), RowError> {
        for (name, e) in fields {
            if let Some(v) = e.maybe(scope, &*self.host)? {
                let r = self.host.body(&v).map_err(|m| row_error(&e.at, m))?;
                out.insert((*name).into(), json!(r));
            }
        }
        Ok(())
    }

    /// Read and parse a joined file once.
    fn load(&mut self, file: &str, format: Format) -> Result<(), RowError> {
        if self.joined.contains_key(file) {
            return Ok(());
        }
        let failed = |m: String| row_error("label.join.file", format!("{file}: {m}"));
        let bytes = self.host.read(file).map_err(failed)?;
        let rows = source::parse(format, &bytes, &self.host.limits()).map_err(failed)?;
        let root = match format {
            Format::Json => rows
                .into_iter()
                .next()
                .map(|(_, v)| v)
                .unwrap_or(Json::Null),
            Format::Jsonl | Format::Csv => Json::Array(rows.into_iter().map(|(_, v)| v).collect()),
        };
        self.joined.insert(file.to_owned(), root);
        Ok(())
    }

    fn run(&mut self, r: &Run, n: usize, scope: Scope<'_>) -> Result<String, RowError> {
        let host = &*self.host;
        let id = r.run_id.id(scope, host)?;
        let mut run = Map::new();
        run.insert("record".into(), json!("run"));
        run.insert("run_id".into(), json!(id));
        run.insert(
            "system".into(),
            json!({"id": r.system.0.id(scope, host)?, "version": r.system.1.id(scope, host)?}),
        );
        insert_nonempty(&mut run, "grader", collect(&r.grader, scope, host)?);
        run.insert("value_semantics".into(), json!(r.value_semantics));
        if let Some(o) = &r.output_type {
            run.insert("output_type".into(), json!(o));
        }
        if let Some(l) = r.levels {
            run.insert("levels".into(), json!(l));
        }
        if let Some(f) = &r.features {
            run.insert("features".into(), json!(f));
        }
        self.push_once(n, Json::Object(run));
        Ok(id)
    }

    fn item(&mut self, i: &Item, n: usize, scope: Scope<'_>, id: &str) -> Result<(), RowError> {
        let host = &*self.host;
        let mut item = collect(&i.fields, scope, host)?;
        item.insert("record".into(), json!("item"));
        item.insert("item_id".into(), json!(id));
        insert_nonempty(&mut item, "strata", collect(&i.strata, scope, host)?);
        self.bodies(&i.bodies, scope, &mut item)?;
        self.push_once(n, Json::Object(item));
        Ok(())
    }

    fn row(&mut self, c: &Compiled, n: usize, row: &Json) -> Result<(), RowError> {
        let vars = BTreeMap::new();
        let scope = Scope {
            row,
            current: None,
            vars: &vars,
        };
        let run_id = match &c.run {
            Some(r) => Some(self.run(r, n, scope)?),
            None => None,
        };
        let item_id = c.item.id.id(scope, &*self.host)?;
        if c.item.emit {
            self.item(&c.item, n, scope, &item_id)?;
        }
        if let Some(l) = &c.label {
            self.label(l, n, scope, &item_id)?;
        }
        if let (Some(j), Some(r)) = (&c.judgment, &c.run) {
            self.judgments(
                j,
                r,
                n,
                scope,
                &item_id,
                run_id.as_deref().unwrap_or_default(),
            )?;
        }
        Ok(())
    }

    fn label(
        &mut self,
        l: &Label,
        n: usize,
        scope: Scope<'_>,
        item_id: &str,
    ) -> Result<(), RowError> {
        let file = match &l.join {
            None => None,
            Some((file, format, lookup)) => {
                // A path join labels an item once; a key join once per key, below.
                if matches!(lookup, Lookup::Path(_)) && self.labelled.contains(item_id) {
                    return Ok(());
                }
                let file = file.id(scope, &*self.host)?;
                self.load(&file, *format)?;
                Some(file)
            }
        };
        let scopes: Vec<Scope<'_>> = match (&l.join, &file) {
            // The path walks the joined file; its insertions read the row.
            (Some((_, _, Lookup::Path(path))), Some(f)) => {
                match path.first_from(&self.joined[f], scope) {
                    Some(v) => vec![scope.with_current(v)],
                    None => return Ok(()),
                }
            }
            (Some((_, _, Lookup::Key { label, row })), Some(f)) => {
                let Some(rows) = self.joined[f].as_array() else {
                    return Err(row_error(
                        "label.join.file",
                        format!("{f}: a key join needs JSONL, CSV or a JSON array"),
                    ));
                };
                if !self.keyed.contains_key(f) {
                    let mut index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
                    for (i, r) in rows.iter().enumerate() {
                        if let Some(k) = label.render(scope.with_current(r)) {
                            index.entry(k).or_default().push(i);
                        }
                    }
                    self.keyed.insert(f.clone(), index);
                }
                let Some(k) = row.render(scope) else {
                    return Ok(());
                };
                if !self.keys_joined.insert(format!("{f}\n{k}\n{item_id}")) {
                    return Ok(());
                }
                let Some(at) = self.keyed[f].get(&k) else {
                    return Ok(());
                };
                at.iter().map(|i| scope.with_current(&rows[*i])).collect()
            }
            _ => vec![scope],
        };
        for scope in scopes {
            let host = &*self.host;
            let mut label = Map::new();
            label.insert("record".into(), json!("label"));
            label.insert("item_id".into(), json!(item_id));
            for (k, e) in &l.fields {
                let v = match *k {
                    "rater_id" | "source" => Some(e.need(scope, host)?),
                    "version" => e.int(scope, host)?,
                    _ => e.maybe(scope, host)?,
                };
                if let Some(v) = v {
                    label.insert((*k).into(), v);
                }
            }
            label.entry("version").or_insert(json!(1));
            match l.value.eval(scope, host).and_then(|v| match v {
                // Text cells (CSV) become booleans or numbers as in a plain CSV import.
                Json::String(s) => {
                    crate::table::map_value(&s, None, None).map_err(|_| Miss::Unmapped(s))
                }
                other => Ok(other),
            }) {
                Ok(v) => label.insert("value".into(), v),
                Err(m) => label.insert("missing".into(), invalid_output(&l.value, m)),
            };
            self.records.push((n, Json::Object(label)));
        }
        self.labelled.insert(item_id.to_owned());
        Ok(())
    }

    /// The judgment's value, or the missing value it is instead.
    fn value_or_missing(
        j: &Judgment,
        (output_type, levels): (Option<OutputType>, Option<u32>),
        scope: Scope<'_>,
        host: &dyn Host,
    ) -> Result<Result<Json, Json>, RowError> {
        for (cond, reason, detail) in &j.missing_if {
            if cond.holds(scope) {
                let mut m = Map::new();
                m.insert("reason".into(), reason.need(scope, host)?);
                if let Some(v) = detail
                    .as_ref()
                    .map(|d| d.maybe(scope, host))
                    .transpose()?
                    .flatten()
                {
                    m.insert("detail".into(), v);
                }
                return Ok(Err(Json::Object(m)));
            }
        }
        Ok(match j.value.eval(scope, host) {
            Ok(v) => match typed_value(&v, output_type, levels) {
                Some(v) => Ok(v),
                None => Err(invalid_output(&j.value, Miss::Unmapped(shown(&v)))),
            },
            Err(m) => Err(invalid_output(&j.value, m)),
        })
    }

    fn features(
        (path, id, fired, strength): &(Path, Expr, Expr, Option<Expr>),
        scope: Scope<'_>,
        host: &dyn Host,
    ) -> Result<Json, RowError> {
        let mut list = Vec::new();
        for f in path.eval(scope) {
            let s = scope.with_current(f);
            let mut feature = Map::new();
            feature.insert("feature_id".into(), json!(id.id(s, host)?));
            let on = fired.need(s, host)?;
            let on = on
                .as_bool()
                .ok_or_else(|| row_error(&fired.at, format!("{on} is not a boolean")))?;
            feature.insert("fired".into(), json!(on));
            if let Some(v) = strength
                .as_ref()
                .map(|st| st.maybe(s, host))
                .transpose()?
                .flatten()
            {
                feature.insert("strength".into(), v);
            }
            list.push(Json::Object(feature));
        }
        Ok(Json::Array(list))
    }

    fn judgments(
        &mut self,
        j: &Judgment,
        run: &Run,
        n: usize,
        scope: Scope<'_>,
        item_id: &str,
        run_id: &str,
    ) -> Result<(), RowError> {
        // Each element: the variable's value and the element itself.
        let elements: Vec<(Option<String>, Option<&Json>)> = match &j.spread {
            Spread::None => vec![(None, None)],
            Spread::Keys(keys) => keys.iter().map(|k| (Some(k.clone()), None)).collect(),
            Spread::Path(p) => match p.first(scope) {
                Some(Json::Array(xs)) => xs
                    .iter()
                    .enumerate()
                    .map(|(i, x)| (Some(i.to_string()), Some(x)))
                    .collect(),
                Some(Json::Object(m)) => {
                    m.iter().map(|(k, x)| (Some(k.clone()), Some(x))).collect()
                }
                _ => {
                    return Err(row_error(
                        "judgment.explode.path",
                        "no array or object to split into judgments",
                    ));
                }
            },
        };
        // Replicates are numbered among the elements the filter keeps, so dropping a
        // copy leaves no gap.
        let mut kept = 0;
        for (var, element) in elements {
            let mut vars = BTreeMap::new();
            if let (Some(name), Some(v)) = (&j.var, var) {
                vars.insert(name.clone(), v);
            }
            let mut scope = Scope {
                vars: &vars,
                ..scope
            };
            if let Some(e) = element {
                scope = scope.with_current(e);
            }
            if j.filter.as_ref().is_some_and(|c| !c.holds(scope)) {
                self.filtered += 1;
                continue;
            }
            let index = kept;
            kept += 1;
            let host = &*self.host;
            let mut out = Map::new();
            out.insert("record".into(), json!("judgment"));
            out.insert("item_id".into(), json!(item_id));
            out.insert("run_id".into(), json!(run_id));
            out.insert("gen_replicate".into(), json!(0));
            for (k, e) in &j.replicates {
                if let Some(v) = e.int(scope, host)? {
                    out.insert((*k).into(), v);
                }
            }
            if let Some(field) = &j.index {
                out.insert(field.clone(), json!(index));
            }
            if let Some(v) = j
                .task
                .as_ref()
                .map(|t| t.maybe(scope, host))
                .transpose()?
                .flatten()
            {
                out.insert("task".into(), v);
            }
            if let Some(o) = &run.output_type {
                out.insert("output_type".into(), json!(o));
            }
            if let Some(l) = run.levels {
                out.insert("levels".into(), json!(l));
            }
            let kind = run
                .output_type
                .as_deref()
                .and_then(|o| serde_json::from_value(json!(o)).ok());
            match Self::value_or_missing(j, (kind, run.levels), scope, host)? {
                Ok(v) => {
                    out.insert("value".into(), v);
                    if let Some(f) = &j.features {
                        out.insert("features".into(), Self::features(f, scope, host)?);
                    }
                }
                Err(m) => {
                    out.insert("missing".into(), m);
                }
            }
            insert_nonempty(&mut out, "cost", collect(&j.cost, scope, host)?);
            self.bodies(&j.bodies, scope, &mut out)?;
            self.records.push((n, Json::Object(out)));
        }
        Ok(())
    }
}

/// What applying a mapping gave.
#[derive(Debug, Clone, Default)]
pub struct Applied {
    /// The records, each with its row number, in row order.
    pub records: Numbered,
    /// The problems, per row.
    pub issues: Vec<Issue>,
    /// Exploded elements `explode.filter` left out (D-046).
    pub filtered: usize,
}

/// The records of the source rows. Stops at the first row that `source.expect`
/// rejects; other problems are listed per row.
pub fn apply(
    c: &Compiled,
    rows: &[(usize, Json)],
    host: &mut dyn Host,
) -> Result<Applied, MapError> {
    if let Some(at) = c.undecided.first() {
        return Err(MapError::Invalid {
            at: at.clone(),
            message: "is still DECIDE".into(),
        });
    }
    let mut a = Applier {
        host,
        joined: BTreeMap::new(),
        keyed: BTreeMap::new(),
        keys_joined: BTreeSet::new(),
        labelled: BTreeSet::new(),
        emitted: BTreeSet::new(),
        records: Vec::new(),
        filtered: 0,
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
        let filtered = a.filtered;
        if let Err(e) = a.row(c, *n, row) {
            // A dropped row is counted as dropped, not by its elements.
            a.filtered = filtered;
            issues.push(Issue {
                row: *n,
                at: e.at,
                message: e.message,
            });
        }
    }
    Ok(Applied {
        records: a.records,
        issues,
        filtered: a.filtered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testing::Recorder;

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
        e.eval(
            scope,
            &Recorder {
                args: BTreeMap::from([("id".into(), "r-1".into())]),
                ..Recorder::default()
            },
        )
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
        assert!(apply(&c, &rows, &mut Recorder::default()).is_err());
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
