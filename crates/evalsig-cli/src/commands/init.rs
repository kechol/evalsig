//! `evalsig init`: create `.evalsig/` and a commented configuration template.

use std::path::Path;

use serde_json::json;

use crate::error::CliError;
use crate::output::{Envelope, Output};
use crate::project::{self, Project, io_error};

pub const TEMPLATE: &str = include_str!("../config_template.toml");

pub fn run(root: &Path, force: bool) -> Result<Output, CliError> {
    let p = Project::at(root);
    let mut created = Vec::new();
    for dir in
        std::iter::once(p.root.clone()).chain(project::SUBDIRS.iter().map(|d| p.root.join(d)))
    {
        if !dir.is_dir() {
            std::fs::create_dir_all(&dir).map_err(|e| io_error(&dir, &e))?;
            created.push(dir.display().to_string());
        }
    }
    let config = p.config_path();
    let config_state = if config.exists() && !force {
        "kept"
    } else {
        let state = if config.exists() {
            "overwritten"
        } else {
            "written"
        };
        std::fs::write(&config, TEMPLATE).map_err(|e| io_error(&config, &e))?;
        state
    };
    Ok(Output::success(Envelope::new(
        "evalsig.init/1",
        json!({
            "project": p.root.display().to_string(),
            "created": created,
            "config": { "path": config.display().to_string(), "state": config_state },
        }),
    )))
}
