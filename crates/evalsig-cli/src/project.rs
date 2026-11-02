//! The `.evalsig/` working directory of a project (audit spec §2).

use std::path::{Path, PathBuf};

use crate::error::CliError;
use crate::exit::Exit;

pub const DIR: &str = ".evalsig";
pub const CONFIG: &str = "config.toml";
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

    pub fn config_path(&self) -> PathBuf {
        self.root.join(CONFIG)
    }
}

pub fn io_error(path: &Path, e: &std::io::Error) -> CliError {
    CliError::new("io", format!("{}: {e}", path.display()), Exit::Blocked)
}
