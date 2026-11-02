//! The `evalsig` command-line interface.

mod commands;
mod error;
mod exit;
mod output;
mod project;
mod schemas;
mod validate;

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::json;

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Format, Output};

#[derive(Debug, Parser)]
#[command(name = "evalsig", version, about)]
struct Cli {
    /// Output format.
    #[arg(long, global = true, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Project root holding `.evalsig/`.
    #[arg(long, global = true, value_name = "DIR", default_value = ".")]
    project: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create `.evalsig/` and a commented configuration template.
    Init {
        /// Replace an existing configuration file with the template.
        #[arg(long)]
        force: bool,
    },
    /// Print a published JSON Schema (`records`, `ledger-entry`).
    Schema {
        name: String,
        /// Print only the schema document, without the envelope.
        #[arg(long)]
        raw: bool,
    },
    /// Check a `records/1` JSONL file line by line.
    Validate {
        /// The records file.
        #[arg(long, value_name = "FILE")]
        records: PathBuf,
    },
    /// Show the tool, record-format and output-schema versions.
    Version,
}

pub(crate) fn tool_version() -> String {
    format!("evalsig {}", env!("CARGO_PKG_VERSION"))
}

fn run(cli: &Cli) -> Result<Output, CliError> {
    match cli.command {
        Command::Init { force } => commands::init::run(&cli.project, force),
        Command::Schema { ref name, raw: _ } => commands::schema::run(name),
        Command::Validate { ref records } => commands::validate::run(records),
        Command::Version => Ok(Output::success(Envelope::new(
            "evalsig.version/1",
            json!({
                "version": env!("CARGO_PKG_VERSION"),
                "records": evalsig_core::records::RECORDS_VERSION,
                "exit_codes": Exit::ALL.iter().map(|e| json!({"code": e.code(), "meaning": e.meaning()})).collect::<Vec<_>>(),
                "verdict_codes": output::VerdictCode::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            }),
        ))),
    }
}

/// The format requested on the command line, read before full parsing so that usage
/// errors can be reported in it too.
fn requested_format(args: &[String]) -> Format {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let value = match a.strip_prefix("--format=") {
            Some(v) => Some(v.to_owned()),
            None if a == "--format" => it.next().cloned(),
            None => None,
        };
        if let Some(v) = value {
            return match v.as_str() {
                "json" => Format::Json,
                "md" => Format::Md,
                "junit" => Format::Junit,
                _ => Format::Text,
            };
        }
    }
    Format::Text
}

fn report_error(err: &CliError, format: Format) -> ExitCode {
    let text = match format {
        Format::Json => err.to_json(),
        _ => err.to_text(),
    };
    let _ = std::io::stderr().write_all(text.as_bytes());
    ExitCode::from(err.exit.code())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let format = requested_format(&args);
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) => {
            use clap::error::ErrorKind;
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                let _ = e.print();
                return ExitCode::SUCCESS;
            }
            let message = e.render().to_string();
            let first = message
                .lines()
                .next()
                .unwrap_or("usage error")
                .trim_start_matches("error: ")
                .to_owned();
            let err = CliError::usage(first).with_hint("run `evalsig --help` for usage");
            if format == Format::Json {
                return report_error(&err, format);
            }
            let _ = std::io::stderr().write_all(message.as_bytes());
            return ExitCode::from(Exit::Usage.code());
        }
    };
    if let Command::Schema { name, raw: true } = &cli.command {
        return match commands::schema::raw(name) {
            Ok(text) => {
                let _ = std::io::stdout().write_all(text.as_bytes());
                ExitCode::SUCCESS
            }
            Err(err) => report_error(&err, cli.format),
        };
    }
    match run(&cli) {
        Ok(out) => {
            let _ = std::io::stdout().write_all(output::render(&out, cli.format).as_bytes());
            ExitCode::from(out.exit.code())
        }
        Err(err) => report_error(&err, cli.format),
    }
}
