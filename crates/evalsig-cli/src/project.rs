//! The `.evalsig/` working directory of a project (audit spec §2).

use std::path::{Path, PathBuf};

use evalsig_core::config::Config;
use evalsig_core::time::Instant;
use evalsig_store::objects::ObjectStore;

use crate::error::CliError;
use crate::exit::Exit;

pub const DIR: &str = ".evalsig";
pub const CONFIG: &str = "config.toml";
pub const LEDGER: &str = "ledger.jsonl";
pub const SUBDIRS: [&str; 3] = ["objects", "cache", "reports"];

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
    pub fn open(root: &Path) -> Result<Self, CliError> {
        let p = Project::at(root);
        if !p.config_path().is_file() {
            return Err(CliError::new(
                "not_initialized",
                format!("no evalsig project at {}", p.dir.display()),
                Exit::Usage,
            )
            .with_hint("run `evalsig init` first, or pass --project <dir>"));
        }
        Ok(p)
    }

    pub fn config_path(&self) -> PathBuf {
        self.dir.join(CONFIG)
    }

    /// The SQLite working copy (derived; `rebuild` recreates it).
    pub fn cache_path(&self) -> PathBuf {
        self.dir.join("cache").join("evalsig.db")
    }

    /// Agent-side splits, named by the hash of their validation items.
    pub fn splits_dir(&self) -> PathBuf {
        self.dir.join("splits")
    }

    /// Saved results, named by their output hash.
    pub fn reports_dir(&self) -> PathBuf {
        self.dir.join("reports")
    }

    /// Recreate the working copy from the data in force.
    pub fn rebuild_cache(&self, d: &crate::data::Data) -> Result<(), CliError> {
        evalsig_store::cache::rebuild(&self.cache_path(), &d.records, &d.snapshots)
            .map_err(CliError::store)
    }

    pub fn ledger_path(&self) -> PathBuf {
        self.dir.join(LEDGER)
    }

    pub fn objects(&self) -> ObjectStore {
        ObjectStore::new(self.dir.join("objects"))
    }

    /// Append an agent-side ledger line for a command's output and note it in the
    /// envelope's provenance.
    pub fn record(
        &self,
        envelope: &mut crate::output::Envelope,
        kind: evalsig_store::ledger::Kind,
        inputs: Vec<String>,
        outputs: Vec<String>,
        summary: serde_json::Value,
    ) -> Result<(), CliError> {
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
        .map_err(CliError::store)?;
        envelope.provenance.ledger = Some("agent".into());
        envelope.provenance.ledger_seq = Some(entry.seq);
        Ok(())
    }

    /// Keep the configuration (with defaults filled in) as an object named by its
    /// hash, so `verify` can recompute with exactly the settings a result used.
    pub fn store_config(
        &self,
        config: &Config,
    ) -> Result<evalsig_store::hash::ContentHash, CliError> {
        let bytes = evalsig_store::canonical::to_canonical_bytes(&config.to_json())
            .map_err(CliError::store)?;
        self.objects()
            .put(&bytes)
            .map(|(h, _)| h)
            .map_err(CliError::store)
    }

    /// A stored configuration by its hash.
    pub fn stored_config(
        &self,
        hash: &evalsig_store::hash::ContentHash,
    ) -> Result<Config, CliError> {
        let bytes = self.objects().get(hash).map_err(CliError::store)?;
        serde_json::from_slice(&bytes)
            .map_err(|e| CliError::new("invalid_config", e.to_string(), Exit::Rejected))
    }

    pub fn config(&self) -> Result<Config, CliError> {
        let path = self.config_path();
        let text = std::fs::read_to_string(&path).map_err(|e| io_error(&path, &e))?;
        Config::from_toml(&text).map_err(|e| {
            CliError::new(
                "invalid_config",
                format!("{}: {e}", path.display()),
                Exit::Rejected,
            )
        })
    }
}

pub fn io_error(path: &Path, e: &std::io::Error) -> CliError {
    CliError::new("io", format!("{}: {e}", path.display()), Exit::Blocked)
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
