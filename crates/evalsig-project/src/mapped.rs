//! How import reads files: canonical records, CSV, a declarative mapping (`--mapping`,
//! `--from`, D-011) or a harness adapter, all into record candidates.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use evalsig_adapters::limits::{self, Limits};
use evalsig_adapters::mapping::{self, Compiled, Host, MapError};
use evalsig_adapters::{BodySink, csv, inspect, promptfoo, table};
use evalsig_core::config::{Adapter, Config, ContentMode};
use evalsig_core::records::Record;
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::content::{self, Redaction, Redactor};
use evalsig_store::hash::ContentHash;
use evalsig_store::objects::ObjectStore;
use evalsig_store::snapshot::SourceFile;
use serde_json::Value;

use crate::Error;
use crate::exit::Exit;
use crate::project::io_error;
use crate::validate::Issue;

/// How the files of an import become records.
pub enum Reader {
    /// Canonical `records/1` JSONL.
    Records,
    /// A CSV whose columns are record field names.
    Csv(table::RowKind),
    Mapping(Box<Compiled>),
    Inspect(inspect::Options),
    Promptfoo,
}

/// A record candidate before validation: a canonical line is checked as written, so
/// integer literals beyond the exact range are caught; other values as built.
pub enum Candidate<'a> {
    Line(&'a str),
    Value(Value),
}

/// Record candidates of one file, each with its line or row number.
pub type Candidates<'a> = Vec<(usize, Candidate<'a>)>;

/// The mapping or adapter an import goes through.
pub struct Via {
    pub reader: Reader,
    /// The provenance `mapping`: the file's hash, or the canonical `[[import]]` block's.
    pub hash: ContentHash,
    pub adapter: String,
    /// The files an `[[import]]` block names, for when none are given.
    pub files: Vec<PathBuf>,
}

fn invalid(e: impl std::fmt::Display) -> Error {
    Error::new("invalid_mapping", e.to_string(), Exit::Rejected)
}

/// Parse and check a mapping file's text.
pub fn compile_text(path: &Path, text: &str) -> Result<Compiled, Error> {
    mapping::parse(text)
        .and_then(|m| mapping::compile(&m))
        .map_err(|e| invalid(format!("{}: {e}", path.display())))
}

/// Refuse a mapping that still says DECIDE anywhere.
pub fn require_decided(c: &Compiled) -> Result<(), Error> {
    if c.undecided.is_empty() {
        return Ok(());
    }
    Err(Error::new(
        "undecided_mapping",
        format!(
            "the mapping still says DECIDE at {}",
            c.undecided.join(", ")
        ),
        Exit::Blocked,
    )
    .with_hint("record each choice with `evalsig map decide`"))
}

/// The files a block's `path` names, relative to the project root. `*` may appear
/// in the last component.
fn expand(root: &Path, pattern: &str) -> Result<Vec<PathBuf>, Error> {
    let full = root.join(pattern);
    let name = full
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    if !name.contains('*') {
        return Ok(vec![full]);
    }
    let dir = full.parent().unwrap_or(root);
    let (head, tail) = name.split_once('*').expect("contains *");
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| io_error(dir, &e))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n.len() >= head.len() + tail.len() && n.starts_with(head) && n.ends_with(tail)
            })
        })
        .collect();
    out.sort();
    if out.is_empty() {
        return Err(Error::usage(format!("no file matches {pattern}"))
            .with_hint("check the block's path, relative to the project root"));
    }
    Ok(out)
}

/// The mapping requested on the command line, if any. A mapping with `DECIDE`
/// left stops the import.
pub fn resolve(
    root: &Path,
    config: &Config,
    file: Option<&Path>,
    from: Option<&str>,
) -> Result<Option<Via>, Error> {
    let via = match (file, from) {
        (Some(path), _) => {
            let bytes = std::fs::read(path).map_err(|e| io_error(path, &e))?;
            let text =
                std::str::from_utf8(&bytes).map_err(|_| Error::invalid_input(path, "not UTF-8"))?;
            let compiled = compile_text(path, text)?;
            Via {
                adapter: format!("mapping:{}", compiled.name),
                reader: Reader::Mapping(Box::new(compiled)),
                hash: ContentHash::blake3(&bytes),
                files: Vec::new(),
            }
        }
        (None, Some(name)) => {
            let block = config
                .imports
                .iter()
                .find(|i| i.name == name)
                .ok_or_else(|| {
                    Error::usage(format!("no [[import]] block named {name:?}")).with_hint(format!(
                        "blocks: {}",
                        config
                            .imports
                            .iter()
                            .map(|i| i.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                })?;
            let reader = match block.adapter {
                Adapter::Inspect => Reader::Inspect(inspect::Options {
                    score: block.score.clone(),
                    value_map: block.value_map_json(),
                    value_semantics: block.value_semantics.map(|v| v.as_str().to_owned()),
                }),
                Adapter::Promptfoo => Reader::Promptfoo,
                _ => Reader::Mapping(Box::new(mapping::from_import(block).map_err(invalid)?)),
            };
            let canonical = to_canonical_bytes(&serde_json::to_value(block).expect("serializes"))
                .map_err(Error::store)?;
            Via {
                adapter: format!("import:{name}"),
                reader,
                hash: ContentHash::blake3(&canonical),
                files: expand(root, &block.path)?,
            }
        }
        (None, None) => return Ok(None),
    };
    if let Reader::Mapping(c) = &via.reader {
        require_decided(c)?;
    }
    Ok(Some(via))
}

/// What a mapping reads from the importer: `--arg` values, joined files and the
/// bodies it names.
pub struct Bodies {
    args: BTreeMap<String, String>,
    mode: ContentMode,
    redactor: Redactor,
    limits: Limits,
    pub joined: Vec<SourceFile>,
    /// A `content` record per distinct body.
    pub contents: Vec<Record>,
    pub redactions: Vec<Redaction>,
    /// Bytes to store once the import is accepted.
    objects: Vec<Vec<u8>>,
    seen: BTreeSet<String>,
}

impl Bodies {
    pub fn new(args: &[String], config: &Config, limits: Limits) -> Result<Bodies, Error> {
        let args = args
            .iter()
            .map(|a| {
                a.split_once('=')
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .ok_or_else(|| Error::usage(format!("--arg takes NAME=VALUE, not {a:?}")))
            })
            .collect::<Result<_, _>>()?;
        let redactor = match config.store.content {
            ContentMode::Redact => Redactor::new(&config.store.redact_patterns)
                .map_err(|e| Error::new("invalid_config", e.to_string(), Exit::Rejected))?,
            _ => Redactor::default(),
        };
        Ok(Bodies {
            args,
            mode: config.store.content,
            redactor,
            limits,
            joined: Vec::new(),
            contents: Vec::new(),
            redactions: Vec::new(),
            objects: Vec::new(),
            seen: BTreeSet::new(),
        })
    }

    /// Store the bodies once the import is accepted.
    pub fn commit(&mut self, store: &ObjectStore) -> Result<(), Error> {
        for bytes in self.objects.drain(..) {
            store.put(&bytes).map_err(Error::store)?;
        }
        Ok(())
    }
}

impl Host for Bodies {
    fn arg(&self, name: &str) -> Option<&str> {
        self.args.get(name).map(String::as_str)
    }

    fn read(&mut self, file: &str) -> Result<Vec<u8>, String> {
        let bytes =
            limits::read_file(Path::new(file), &self.limits).map_err(|e| format!("{file}: {e}"))?;
        if !self.joined.iter().any(|s| s.path == file) {
            self.joined.push(SourceFile::of(file, &bytes));
        }
        Ok(bytes)
    }

    fn limits(&self) -> Limits {
        self.limits
    }
}

impl BodySink for Bodies {
    fn body(&mut self, value: &Value) -> Result<String, String> {
        let (bytes, media_type) = match value {
            Value::String(s) => (s.as_bytes().to_vec(), "text/plain"),
            other => (
                to_canonical_bytes(other).map_err(|e| e.to_string())?,
                "application/json",
            ),
        };
        let reference = ContentHash::blake3(&bytes).to_string();
        // A body many records name is kept, and listed, once.
        if !self.seen.insert(reference.clone()) {
            return Ok(reference);
        }
        let prepared = content::prepare_body(self.mode, &self.redactor, &bytes, media_type);
        self.contents.push(Record::Content(prepared.content));
        self.redactions.extend(prepared.redaction);
        self.objects.extend(prepared.object);
        Ok(reference)
    }
}

fn numbered(values: Vec<Value>) -> Candidates<'static> {
    values
        .into_iter()
        .enumerate()
        .map(|(i, v)| (i + 1, Candidate::Value(v)))
        .collect()
}

impl Reader {
    /// The adapter name kept in provenance.
    pub fn name(&self) -> &'static str {
        match self {
            Reader::Records => "records",
            Reader::Csv(_) => "csv",
            Reader::Mapping(_) => "mapping",
            Reader::Inspect(_) => "inspect",
            Reader::Promptfoo => "promptfoo",
        }
    }

    /// Read one file into numbered record candidates and the problems found.
    pub fn read<'a>(
        &self,
        path: &Path,
        bytes: &'a [u8],
        host: &mut Bodies,
    ) -> Result<(Candidates<'a>, Vec<Issue>), Error> {
        let bad = |e: &dyn std::fmt::Display| Error::invalid_input(path, e);
        match self {
            Reader::Records => {
                limits::check_jsonl_depth(bytes, &host.limits).map_err(|e| {
                    Error::new(
                        "input_limit",
                        format!("{}: {e}", path.display()),
                        Exit::Rejected,
                    )
                })?;
                let text = std::str::from_utf8(bytes).map_err(|_| bad(&"not UTF-8"))?;
                let lines = text
                    .lines()
                    .enumerate()
                    .filter(|(_, l)| !l.trim().is_empty())
                    .map(|(i, l)| (i + 1, Candidate::Line(l)))
                    .collect();
                Ok((lines, Vec::new()))
            }
            Reader::Csv(kind) => {
                let text = std::str::from_utf8(bytes).map_err(|_| bad(&"not UTF-8"))?;
                let table = csv::parse(text).map_err(|e| bad(&e))?;
                let rows = table::to_records(&table, *kind).map_err(|e| bad(&e))?;
                Ok((
                    rows.into_iter()
                        .map(|(n, v)| (n, Candidate::Value(v)))
                        .collect(),
                    Vec::new(),
                ))
            }
            Reader::Mapping(compiled) => {
                let rows = mapping::rows(compiled, bytes, &host.limits).map_err(|e| bad(&e))?;
                let (records, issues) =
                    mapping::apply(compiled, &rows, host).map_err(|e| match e {
                        MapError::Unexpected { .. } => Error::new(
                            "unexpected_source",
                            format!("{}: {e}", path.display()),
                            Exit::Rejected,
                        ),
                        other => invalid(other),
                    })?;
                let issues = issues
                    .into_iter()
                    .map(|i| Issue {
                        line: i.row,
                        code: "mapping.row".into(),
                        path: i.at,
                        message: i.message,
                    })
                    .collect();
                Ok((
                    records
                        .into_iter()
                        .map(|(n, v)| (n, Candidate::Value(v)))
                        .collect(),
                    issues,
                ))
            }
            Reader::Inspect(options) => {
                let zip = path.extension().is_some_and(|x| x == "eval");
                let (log, samples) =
                    inspect::read_log(bytes, zip, &host.limits).map_err(|e| bad(&e))?;
                let records =
                    inspect::records(&log, &samples, options, host).map_err(|e| bad(&e))?;
                Ok((numbered(records), Vec::new()))
            }
            Reader::Promptfoo => {
                // A local database (`.db`): the evaluation named by `--arg eval=ID`, or
                // the most recent; `-o out.json` output otherwise.
                let eval = if path.extension().is_some_and(|x| x == "db") {
                    promptfoo::read_database(path, host.arg("eval"), &host.limits)
                        .map_err(|e| bad(&e))?
                } else {
                    promptfoo::parse(bytes, &host.limits).map_err(|e| bad(&e))?
                };
                let records = promptfoo::records(&eval, host).map_err(|e| bad(&e))?;
                Ok((numbered(records), Vec::new()))
            }
        }
    }
}
