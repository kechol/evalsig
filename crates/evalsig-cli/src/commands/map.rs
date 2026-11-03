//! `evalsig map`: onboarding records whose shape evalsig does not know (SPEC §5.3).

use std::path::{Path, PathBuf};

use clap::Subcommand;
use evalsig_adapters::limits::Limits;
use evalsig_adapters::probe::{self, Shape};
use serde_json::json;

use crate::commands::import::parse_size;
use crate::error::CliError;
use crate::output::{Envelope, Next, Output};

#[derive(Debug, Subcommand)]
pub enum MapCommand {
    /// Show the shape of a file: per path, types, counts and distinct values, with key
    /// and replicate candidates. Free text and secret-like values are never printed.
    Probe {
        file: PathBuf,
        /// List no values at all, not even short categorical ones.
        #[arg(long)]
        no_values: bool,
        /// Refuse a file, or decompressed archive, larger than this.
        #[arg(long, value_name = "SIZE", default_value = "1G", value_parser = parse_size)]
        max_bytes: u64,
    },
}

fn shape_of(path: &Path) -> Result<Shape, CliError> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("eval" | "zip") => Ok(Shape::Zip),
        Some("json") => Ok(Shape::Json),
        Some("jsonl" | "ndjson") => Ok(Shape::Jsonl),
        Some("csv") => Ok(Shape::Csv),
        _ => Err(
            CliError::usage(format!("cannot tell the format of {}", path.display()))
                .with_hint("name the file .json, .jsonl, .csv or .eval"),
        ),
    }
}

pub fn run(cmd: &MapCommand) -> Result<Output, CliError> {
    match cmd {
        MapCommand::Probe {
            file,
            no_values,
            max_bytes,
        } => {
            let shape = shape_of(file)?;
            let limits = Limits {
                max_bytes: *max_bytes,
                ..Limits::default()
            };
            let bytes = evalsig_adapters::limits::read_file(file, &limits)
                .map_err(|e| CliError::invalid_input(file, e))?;
            let rows = probe::rows(shape, &bytes, &limits)
                .map_err(|e| CliError::invalid_input(file, e))?;
            let p = probe::probe(shape, &rows, *no_values);
            let mut envelope = Envelope::new(
                "evalsig.map.probe/1",
                json!({ "file": file.display().to_string(), "probe": p }),
            );
            if let Some(k) = &p.known
                && let Some(a) = k.adapter
            {
                envelope.next = vec![Next {
                    command: format!("evalsig import --adapter {a} {}", file.display()),
                    why: k.hint.into(),
                }];
            }
            Ok(Output::success(envelope))
        }
    }
}
