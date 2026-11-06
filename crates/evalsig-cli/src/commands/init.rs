//! `evalsig init`: create `.evalsig/` and a commented configuration template.

use std::path::Path;

use serde_json::json;

use evalsig_project::Error;
use evalsig_project::envelope::{Envelope, Output};
use evalsig_project::project::{self, Project, io_error};

pub const TEMPLATE: &str = include_str!("../config_template.toml");

pub fn run(root: &Path, force: bool) -> Result<Output, Error> {
    let p = Project::at(root);
    let mut created = Vec::new();
    for dir in std::iter::once(p.dir.clone()).chain(project::SUBDIRS.iter().map(|d| p.dir.join(d)))
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
            "project": p.dir.display().to_string(),
            "created": created,
            "config": { "path": config.display().to_string(), "state": config_state },
        }),
    )))
}
