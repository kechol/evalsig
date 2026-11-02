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
    pub root: PathBuf,
}

impl Project {
    pub fn at(root: &Path) -> Self {
        Project {
            root: root.join(DIR),
        }
    }

    /// The project, which must have been initialized.
    pub fn open(root: &Path) -> Result<Self, CliError> {
        let p = Project::at(root);
        if !p.config_path().is_file() {
            return Err(CliError::new(
                "not_initialized",
                format!("no evalsig project at {}", p.root.display()),
                Exit::Usage,
            )
            .with_hint("run `evalsig init` first, or pass --project <dir>"));
        }
        Ok(p)
    }

    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG)
    }

    pub fn ledger_path(&self) -> PathBuf {
        self.root.join(LEDGER)
    }

    pub fn objects(&self) -> ObjectStore {
        ObjectStore::new(self.root.join("objects"))
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
