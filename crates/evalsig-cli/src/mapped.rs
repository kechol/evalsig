//! Imports through a declarative mapping or a harness adapter: `import --mapping
//! <file>`, `import --from <name>` (D-011) and Inspect logs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use evalsig_adapters::inspect;
use evalsig_adapters::limits::{self, Limits};
use evalsig_adapters::mapping::{self, Compiled, Host, MapError};
use evalsig_core::config::{Adapter, Config, ContentMode};
use evalsig_core::records::Record;
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::content::{self, Redaction, Redactor};
use evalsig_store::hash::ContentHash;
use evalsig_store::objects::ObjectStore;
use evalsig_store::snapshot::SourceFile;
use serde_json::Value;

use crate::error::CliError;
use crate::exit::Exit;
use crate::project::io_error;
use crate::validate::Issue;

/// How the files of an import become records.
pub enum Reader {
    Mapping(Box<Compiled>),
    Inspect(inspect::Options),
}

/// The mapping or adapter an import goes through.
pub struct Via {
    pub reader: Reader,
    /// The provenance `mapping`: the file's hash, or the canonical `[[import]]` block's.
    pub hash: ContentHash,
    pub adapter: String,
    /// The files an `[[import]]` block names, for when none are given.
    pub files: Vec<PathBuf>,
}

fn invalid(e: impl std::fmt::Display) -> CliError {
    CliError::new("invalid_mapping", e.to_string(), Exit::Rejected)
}

/// The files a block's `path` names, relative to the project root. `*` may appear
/// in the last component.
fn expand(root: &Path, pattern: &str) -> Result<Vec<PathBuf>, CliError> {
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
    Ok(out)
}

/// The mapping requested on the command line, if any. A mapping with `DECIDE`
/// left stops the import.
pub fn resolve(
    root: &Path,
    config: &Config,
    file: Option<&Path>,
    from: Option<&str>,
) -> Result<Option<Via>, CliError> {
    let via = match (file, from) {
        (Some(path), _) => {
            let bytes = std::fs::read(path).map_err(|e| io_error(path, &e))?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| CliError::invalid_input(path, "not UTF-8"))?;
            let compiled = mapping::parse(text)
                .and_then(|m| mapping::compile(&m))
                .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
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
                    CliError::usage(format!("no [[import]] block named {name:?}")).with_hint(
                        format!(
                            "blocks: {}",
                            config
                                .imports
                                .iter()
                                .map(|i| i.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    )
                })?;
            let reader = match block.adapter {
                Adapter::Inspect => Reader::Inspect(inspect::Options {
                    score: block.score.clone(),
                    value_map: block
                        .value_map
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::to_value(v).expect("serializes")))
                        .collect(),
                    value_semantics: block
                        .value_semantics
                        .and_then(|v| serde_json::to_value(v).ok())
                        .and_then(|v| v.as_str().map(str::to_owned)),
                }),
                _ => Reader::Mapping(Box::new(mapping::from_import(block).map_err(invalid)?)),
            };
            let canonical = to_canonical_bytes(&serde_json::to_value(block).expect("serializes"))
                .map_err(CliError::store)?;
            Via {
                adapter: format!("import:{name}"),
                reader,
                hash: ContentHash::blake3(&canonical),
                files: expand(root, &block.path)?,
            }
        }
        (None, None) => return Ok(None),
    };
    if let Reader::Mapping(c) = &via.reader
        && !c.undecided.is_empty()
    {
        return Err(CliError::new(
            "undecided_mapping",
            format!(
                "the mapping still says DECIDE at {}",
                c.undecided.join(", ")
            ),
            Exit::Blocked,
        )
        .with_hint("replace each DECIDE with a choice before importing"));
    }
    Ok(Some(via))
}

/// What a mapping reads from the importer: `--arg` values, joined files and the
/// bodies it names.
pub struct Bodies<'s> {
    args: BTreeMap<String, String>,
    /// `None` on a dry run: bodies are hashed, not kept.
    store: Option<&'s ObjectStore>,
    mode: ContentMode,
    redactor: Redactor,
    limits: Limits,
    pub joined: Vec<SourceFile>,
    pub contents: Vec<Value>,
    pub redactions: Vec<Redaction>,
}

impl<'s> Bodies<'s> {
    pub fn new(
        args: &[String],
        store: Option<&'s ObjectStore>,
        config: &Config,
        limits: Limits,
    ) -> Result<Bodies<'s>, CliError> {
        let args = args
            .iter()
            .map(|a| {
                a.split_once('=')
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .ok_or_else(|| CliError::usage(format!("--arg takes NAME=VALUE, not {a:?}")))
            })
            .collect::<Result<_, _>>()?;
        let redactor = match config.store.content {
            ContentMode::Redact => Redactor::new(&config.store.redact_patterns)
                .map_err(|e| CliError::new("invalid_config", e.to_string(), Exit::Rejected))?,
            _ => Redactor::default(),
        };
        Ok(Bodies {
            args,
            store,
            mode: config.store.content,
            redactor,
            limits,
            joined: Vec::new(),
            contents: Vec::new(),
            redactions: Vec::new(),
        })
    }
}

impl Host for Bodies<'_> {
    fn arg(&self, name: &str) -> Option<&str> {
        self.args.get(name).map(String::as_str)
    }

    fn read(&mut self, file: &str) -> Result<Vec<u8>, String> {
        let bytes = limits::read_file(Path::new(file), &self.limits)
            .and_then(|b| limits::check_json_depth(&b, &self.limits).map(|()| b))
            .map_err(|e| format!("{file}: {e}"))?;
        if !self.joined.iter().any(|s| s.path == file) {
            self.joined.push(SourceFile {
                path: file.to_owned(),
                hash: ContentHash::blake3(&bytes),
            });
        }
        Ok(bytes)
    }

    fn body(&mut self, value: &Value) -> Result<String, String> {
        let (bytes, media_type) = match value {
            Value::String(s) => (s.as_bytes().to_vec(), "text/plain"),
            other => (
                to_canonical_bytes(other).map_err(|e| e.to_string())?,
                "application/json",
            ),
        };
        let Some(store) = self.store else {
            return Ok(ContentHash::blake3(&bytes).to_string());
        };
        let (record, redaction) =
            content::store_body(store, self.mode, &self.redactor, &bytes, media_type)
                .map_err(|e| e.to_string())?;
        let reference = record.content_ref.clone();
        self.contents
            .push(serde_json::to_value(Record::Content(record)).expect("records serialize"));
        self.redactions.extend(redaction);
        Ok(reference)
    }
}

/// Map one file; returns record JSON numbered by source row, and the problems.
pub fn apply(
    via: &Via,
    path: &Path,
    bytes: &[u8],
    host: &mut Bodies<'_>,
) -> Result<(mapping::Numbered, Vec<Issue>), CliError> {
    let compiled = match &via.reader {
        Reader::Mapping(c) => c,
        Reader::Inspect(options) => {
            return Ok((inspect_log(path, bytes, options, host)?, Vec::new()));
        }
    };
    let rows = mapping::rows(compiled, bytes).map_err(|e| CliError::invalid_input(path, e))?;
    let (records, issues) = mapping::apply(compiled, &rows, host).map_err(|e| match e {
        MapError::Unexpected { .. } => CliError::new(
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
    Ok((records, issues))
}

impl Via {
    /// Whether the files are JSON to check for depth before reading (archives and
    /// Inspect logs check their own entries).
    pub fn reads_json(&self) -> bool {
        match &self.reader {
            Reader::Mapping(c) => c.reads_json(),
            Reader::Inspect(_) => false,
        }
    }
}

/// The records of an Inspect log: a `.eval` archive, or a `.json` log otherwise.
pub fn inspect_log(
    path: &Path,
    bytes: &[u8],
    options: &inspect::Options,
    host: &mut Bodies<'_>,
) -> Result<mapping::Numbered, CliError> {
    let zip = path.extension().is_some_and(|x| x == "eval");
    let failed = |e: inspect::InspectError| CliError::invalid_input(path, e);
    let (log, samples) = inspect::read_log(bytes, zip, &host.limits).map_err(failed)?;
    let records = inspect::records(&log, &samples, options, host).map_err(failed)?;
    Ok(records
        .into_iter()
        .enumerate()
        .map(|(i, r)| (i + 1, r))
        .collect())
}
