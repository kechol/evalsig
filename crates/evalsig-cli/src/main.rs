//! The `evalsig` command-line interface.

mod analysis;
mod checks;
mod commands;
mod data;
mod error;
mod exit;
mod mapped;
mod markdown;
mod output;
mod project;
mod provenance;
mod schemas;
mod validate;

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
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
    /// Compare a baseline run with one or more candidates on their common items.
    Compare(commands::compare::Args),
    /// Create `.evalsig/` and a commented configuration template.
    Init {
        /// Replace an existing configuration file with the template.
        #[arg(long)]
        force: bool,
    },
    /// Read records into a snapshot and record the import in the ledger.
    Import(commands::import::Args),
    /// List the imported items.
    Items,
    /// Read, verify or anchor a ledger.
    #[command(subcommand)]
    Ledger(commands::ledger::LedgerCommand),
    /// Onboard records of an unknown shape: probe them and write a mapping.
    #[command(subcommand)]
    Map(commands::map::MapCommand),
    /// Rewrite snapshots in an earlier record format in the current one.
    Migrate(commands::migrate::Args),
    /// Remove a content body and the free-text fields of the items that name it.
    Purge(commands::purge::Args),
    /// Recreate the SQLite working copy from the canonical snapshots.
    Rebuild,
    /// The default-tier metrics of one run.
    Report(commands::report::Args),
    /// List the imported runs.
    Runs,
    /// Print a published JSON Schema (`records`, `ledger-entry`, `mapping`).
    Schema {
        name: String,
        /// Print only the schema document, without the envelope.
        #[arg(long)]
        raw: bool,
    },
    /// Run validity checks only: on a records file, or on the imported data.
    Validate {
        /// Check this `records/1` file instead of the project's data.
        #[arg(long, value_name = "FILE")]
        records: Option<PathBuf>,
        /// List every check with its default action and override flag.
        #[arg(long)]
        list_checks: bool,
        #[command(flatten)]
        checks: checks::DatasetArgs,
    },
    /// Recompute a saved result from its snapshots and configuration.
    Verify(commands::verify::Args),
    /// Show the tool, record-format and output-schema versions.
    Version,
}

pub(crate) fn tool_version() -> String {
    format!("evalsig {}", env!("CARGO_PKG_VERSION"))
}

fn run(cli: &Cli) -> Result<Output, CliError> {
    if cli.format == Format::Jsonl && !matches!(&cli.command, Command::Compare(a) if a.items) {
        return Err(CliError::usage(
            "--format jsonl prints per-item rows and needs `compare --items`",
        ));
    }
    match cli.command {
        Command::Init { force } => commands::init::run(&cli.project, force),
        Command::Compare(ref args) => commands::compare::run(&cli.project, args),
        Command::Import(ref args) => commands::import::run(&cli.project, args),
        Command::Items => commands::list::items(&cli.project),
        Command::Ledger(ref cmd) => commands::ledger::run(&cli.project, cmd),
        Command::Map(ref cmd) => commands::map::run(cmd),
        Command::Migrate(ref args) => commands::migrate::run(&cli.project, args),
        Command::Purge(ref args) => commands::purge::run(&cli.project, args),
        Command::Rebuild => commands::rebuild::run(&cli.project),
        Command::Report(ref args) => commands::report::run(&cli.project, args),
        Command::Runs => commands::list::runs(&cli.project),
        Command::Schema { ref name, raw: _ } => commands::schema::run(name),
        Command::Validate {
            records: Some(ref records),
            list_checks,
            checks,
        } => {
            if list_checks || checks.allow_unexplained_missing || checks.fail_on_secret {
                return Err(CliError::usage(
                    "--records checks the file's format only; --list-checks and check flags apply to the project's data",
                ));
            }
            commands::validate::run(records)
        }
        Command::Validate {
            records: None,
            list_checks,
            checks,
        } => commands::validate::project(&cli.project, checks, list_checks),
        Command::Verify(ref args) => commands::verify::run(&cli.project, args),
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
fn requested_format(args: &[std::ffi::OsString]) -> Format {
    let mut it = args.iter().map(|a| a.to_string_lossy());
    while let Some(a) = it.next() {
        let value = match a.strip_prefix("--format=") {
            Some(v) => Some(v.to_owned()),
            None if a == "--format" => it.next().map(|v| v.into_owned()),
            None => None,
        };
        if let Some(v) = value {
            return Format::from_str(&v, false).unwrap_or_default();
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
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
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
