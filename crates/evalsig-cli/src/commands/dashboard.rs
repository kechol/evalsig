//! `evalsig dashboard`: the local web dashboard (SPEC §11, D-019). It serves pages
//! rendered from the same results the CLI prints, on 127.0.0.1 only, behind a
//! per-launch token, and reads the project without writing to it.

use std::path::{Path, PathBuf};

use evalsig_dashboard::html;
use evalsig_dashboard::server::{self, Page};
use serde_json::{Value, json};

use crate::commands::list;
use crate::error::CliError;
use crate::output::Output;
use crate::project::Project;

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// The port on 127.0.0.1 (0 picks a free one).
    #[arg(long, default_value_t = 0)]
    pub port: u16,
}

pub const NAV: [(&str, &str); 1] = [("./", "Overview")];

fn cell(v: &Value) -> String {
    match v {
        Value::Null => "–".into(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// The overview: every run with its counts.
fn overview(root: &Path) -> Result<String, CliError> {
    let runs = list::runs(root)?.envelope;
    let rows: Vec<Vec<String>> = runs.results["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| {
            [
                "run_id",
                "system",
                "version",
                "output_type",
                "items",
                "judgments",
                "missing",
            ]
            .iter()
            .map(|k| cell(&r[*k]))
            .collect()
        })
        .collect();
    let body = html::table(
        &[
            "Run",
            "System",
            "Version",
            "Output type",
            "Items",
            "Judgments",
            "Missing",
        ],
        &rows,
    );
    let warnings: Vec<String> = runs.warnings.iter().map(|w| w.message.clone()).collect();
    Ok(html::page("Overview", &NAV, &warnings, &body))
}

fn route(root: &Path, path: &str) -> Page {
    let result = match path {
        "" => overview(root),
        _ => return Page::not_found(),
    };
    match result {
        Ok(body) => Page::html(body),
        Err(e) => Page::html(html::page("Error", &NAV, &[e.message], "")),
    }
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    Project::open(root)?;
    let token = evalsig_store::seal::random_token();
    let srv = server::bind(args.port, token)
        .map_err(|e| CliError::usage(format!("cannot listen on 127.0.0.1:{}: {e}", args.port)))?;
    println!("{}", json!({ "url": srv.url() }));
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    let root: PathBuf = root.to_path_buf();
    srv.serve(move |path| route(&root, path));
    Err(CliError::usage("the dashboard stopped"))
}
