//! The `.evalsig/` working directory of a project (audit spec §2).

use std::path::{Path, PathBuf};

use evalsig_core::config::Config;
use evalsig_core::time::Instant;
use evalsig_store::objects::ObjectStore;

use crate::Error;
use crate::exit::Exit;

pub const DIR: &str = ".evalsig";
pub const CONFIG: &str = "config.toml";
pub const LEDGER: &str = "ledger.jsonl";
pub const SUBDIRS: [&str; 2] = ["objects", "reports"];

#[derive(Debug, Clone)]
pub struct Project {
    pub dir: PathBuf,
}

impl Project {
    pub fn at(root: &Path) -> Self {
        Project {
            dir: root.join(DIR),
        }
    }

    /// The project, which must have been initialized.
    pub fn open(root: &Path) -> Result<Self, Error> {
        let p = Project::at(root);
        if !p.config_path().is_file() {
            return Err(Error::new(
                "not_initialized",
                format!("no evalsig project at {}", p.dir.display()),
                Exit::Usage,
            )
            .with_hint("create the project first, or pass --project <dir>")
            .with_next("evalsig init", "create .evalsig/ here"));
        }
        Ok(p)
    }

    pub fn config_path(&self) -> PathBuf {
        self.dir.join(CONFIG)
    }

    /// Agent-side splits, named by the hash of their validation items.
    pub fn splits_dir(&self) -> PathBuf {
        self.dir.join("splits")
    }

    /// Saved results, named by their output hash.
    pub fn reports_dir(&self) -> PathBuf {
        self.dir.join("reports")
    }

    pub fn ledger_path(&self) -> PathBuf {
        self.dir.join(LEDGER)
    }

    /// The agent-side ledger's lines.
    pub fn entries(&self) -> Result<Vec<evalsig_store::ledger::Entry>, Error> {
        evalsig_store::ledger::read(&self.ledger_path()).map_err(Error::store)
    }

    pub fn objects(&self) -> ObjectStore {
        ObjectStore::new(self.dir.join("objects"))
    }

    /// Append an agent-side ledger line for a command's output and note it in the
    /// envelope's provenance.
    pub fn record(
        &self,
        envelope: &mut crate::envelope::Envelope,
        kind: evalsig_store::ledger::Kind,
        inputs: Vec<String>,
        outputs: Vec<String>,
        summary: serde_json::Value,
    ) -> Result<(), Error> {
        let serde_json::Value::Object(summary) = summary else {
            panic!("a ledger summary is a JSON object");
        };
        let entry = evalsig_store::ledger::append(
            &self.ledger_path(),
            evalsig_store::ledger::LedgerSide::Agent,
            kind,
            evalsig_store::ledger::NewEntry {
                ts: now(),
                actor: actor(),
                inputs,
                outputs,
                summary,
                tool: crate::tool_version(),
            },
        )
        .map_err(Error::store)?;
        envelope.provenance.ledger = Some("agent".into());
        envelope.provenance.ledger_seq = Some(entry.seq);
        Ok(())
    }

    /// Keep the configuration (with defaults filled in) as an object named by its
    /// hash, so `verify` can recompute with exactly the settings a result used.
    pub fn store_config(&self, config: &Config) -> Result<evalsig_store::hash::ContentHash, Error> {
        let bytes = evalsig_store::canonical::to_canonical_bytes(&config.to_json())
            .map_err(Error::store)?;
        self.objects()
            .put(&bytes)
            .map(|(h, _)| h)
            .map_err(Error::store)
    }

    /// A stored configuration by its hash.
    pub fn stored_config(&self, hash: &evalsig_store::hash::ContentHash) -> Result<Config, Error> {
        let bytes = self.objects().get(hash).map_err(Error::store)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| Error::new("invalid_config", e.to_string(), Exit::Rejected))
    }

    pub fn config(&self) -> Result<Config, Error> {
        let path = self.config_path();
        let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, &e))?;
        Config::from_toml(&text).map_err(|e| {
            Error::new(
                "invalid_config",
                format!("{}: {e}", path.display()),
                Exit::Rejected,
            )
        })
    }
}

/// Refuse an output path in a `.evalsig` directory, as written or once every symbolic
/// link on the way is resolved (the destination itself included): the store, the
/// ledger and the configuration are never the destination of a file a command writes.
pub fn outside_store(path: &Path) -> Result<(), Error> {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let resolved = abs.canonicalize().ok().or_else(|| {
        abs.parent()
            .and_then(|p| p.canonicalize().ok())
            .map(|p| p.join(abs.file_name().unwrap_or_default()))
    });
    let inside = [Some(abs), resolved].into_iter().flatten().any(|p| {
        p.components().any(|c| {
            c.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(".evalsig")
        })
    });
    if inside {
        return Err(Error::usage(format!(
            "{} is inside .evalsig/, which holds the store, the ledger and the configuration",
            path.display()
        ))
        .with_hint("write the file outside .evalsig/"));
    }
    Ok(())
}

/// The strings of a JSON array, such as a file's item IDs.
pub fn str_set(v: &serde_json::Value) -> std::collections::BTreeSet<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i.as_str().map(str::to_owned))
        .collect()
}

/// The hex digest of a `blake3:` reference, as files named by the hash use it.
pub fn hex(h: &str) -> &str {
    h.strip_prefix("blake3:").unwrap_or(h)
}

pub fn io_error(path: &Path, e: &std::io::Error) -> Error {
    Error::new("io", format!("{}: {e}", path.display()), Exit::Blocked)
}

/// Who is acting, for ledger lines. Self-reported: an audit hint, not a control.
pub fn actor() -> String {
    let var = |k| std::env::var(k).ok().filter(|v: &String| !v.is_empty());
    if let Some(a) = var("EVALSIG_ACTOR") {
        return a;
    }
    if let Some(a) = var("AI_AGENT") {
        return format!("agent:{a}");
    }
    format!(
        "user:{}",
        var("USER")
            .or_else(|| var("USERNAME"))
            .unwrap_or_else(|| "unknown".into())
    )
}

pub fn now() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as i128);
    Instant::from_unix_nanos(nanos).to_rfc3339()
}
