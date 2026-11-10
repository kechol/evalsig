//! The `evalsig` command-line interface.

mod agent;
mod args;
mod commands;
mod help;
mod markdown;
mod output;

use std::io::{IsTerminal as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use evalsig_project::Error;
use evalsig_project::envelope::{Envelope, Output, Schema, VerdictCode};
use evalsig_project::exit::Exit;

use crate::output::Format;

#[derive(Debug, Parser)]
#[command(name = "evalsig", version, about, disable_help_subcommand = true)]
struct Cli {
    /// Output format.
    #[arg(long, global = true, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Project root holding `.evalsig/`.
    #[arg(long, global = true, value_name = "DIR", default_value = ".")]
    project: PathBuf,

    /// Agent mode: one JSON envelope sized for an agent's context (D-018). Also on
    /// when AI_AGENT is set, the output is not a terminal and no --format is given.
    #[arg(long, global = true, conflicts_with = "no_agent")]
    agent: bool,

    /// Never use agent mode (as EVALSIG_AGENT_MODE=0).
    #[arg(long, global = true)]
    no_agent: bool,

    /// In agent mode: the first row of each list to show.
    #[arg(long, global = true, value_name = "N", default_value_t = 0)]
    cursor: usize,

    /// In agent mode: return text from the evaluated data in `untrusted_content`.
    #[arg(long, global = true)]
    include_content: bool,

    /// Left out: the command index for programs, the usage for people (D-040).
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Agreement between raters, of graders and a run with people, and on preferences.
    Agree(commands::agree::Args),
    /// Check one run against interval-based absolute criteria (not preregistered).
    Check(commands::check::Args),
    /// Compare a baseline run with one or more candidates on their common items.
    Compare(commands::compare::Args),
    /// Create `.evalsig/` and a commented configuration template.
    Init {
        /// Replace an existing configuration file with the template.
        #[arg(long)]
        force: bool,
    },
    /// The analyses the imported data allow, and what to add for the others.
    Capabilities {
        /// Only this run.
        #[arg(long)]
        run: Option<String>,
    },
    /// Leave items out of every analysis, with a reason code (holdout items: the referee).
    Exclude(commands::exclude::Args),
    /// Serve the local web dashboard on 127.0.0.1, behind a per-launch token (D-019).
    Dashboard(commands::dashboard::Args),
    /// Print the AGENTS.md snippet or the Claude Code permission template for evalsig.
    #[command(group(clap::ArgGroup::new("guide").required(true).args(["snippet", "settings"])))]
    AgentGuide {
        /// The snippet for AGENTS.md or CLAUDE.md, for agents without Skills.
        #[arg(long)]
        snippet: bool,
        /// A `.claude/settings.json` permission template that guards against accidents.
        #[arg(long)]
        settings: bool,
    },
    /// What a term, metric, verdict, validity check or exit code means.
    Explain {
        /// A glossary identifier, verdict code, validity check or exit code.
        name: String,
    },
    /// Write the data as Parquet, CSV or canonical JSONL for analysis elsewhere.
    Export(commands::export::Args),
    /// As the referee: judge a candidate on the sealed holdout under approved conditions.
    Gate(commands::gate::Args),
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
    /// Sample sizes, the minimum detectable difference and a gate's pass probability.
    Power(commands::power::PowerArgs),
    /// Draft gate conditions: check them, name them by hash and record the draft.
    Preregister(commands::preregister::Args),
    /// Remove a content body and the free-text fields of the items that name it.
    Purge(commands::purge::Args),
    /// Create a referee home with a new age identity (D-014).
    #[command(subcommand)]
    Referee(RefereeCommand),
    /// The default-tier metrics of one run.
    Report(commands::report::Args),
    /// List the imported runs.
    Runs,
    /// As the referee: encrypt a holdout's labels to the referee's recipient.
    Seal(commands::seal::Args),
    /// Install the evalsig Agent Skill, or check an installed one against this binary.
    #[command(subcommand)]
    Skill(commands::skill::SkillCommand),
    /// Split the items into dev and validation by cluster, and record the split.
    Split(commands::split::Args),
    /// List the recorded dev / validation splits.
    Splits,
    /// Print a published JSON Schema (`records`, `ledger-entry`, `mapping`, `gate`,
    /// `envelope`, and the results of `report` and `compare`).
    Schema {
        name: String,
        /// Print only the schema document, without the envelope.
        #[arg(long)]
        raw: bool,
        /// Print a standard-library-only records/1 writer instead (python, typescript).
        #[arg(long, value_name = "LANG", conflicts_with = "raw")]
        emit: Option<String>,
    },
    /// The steps of common tasks, and how to get what evalsig needs from a source.
    Recipes {
        #[command(subcommand)]
        recipe: Option<commands::recipes::RecipesCommand>,
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
        checks: args::DatasetArgs,
    },
    /// The cost-optimal decision threshold of a run and its sensitivity to the cost ratio.
    Threshold(commands::power::ThresholdArgs),
    /// A run's metrics per period of occurred_at, compared with a baseline period.
    Trend(commands::trend::Args),
    /// Recompute a saved result from its snapshots and configuration.
    Verify(commands::verify::Args),
    /// Show the tool, record-format and output-schema versions.
    Version,
    /// Print the help of a command, or with --json describe every command.
    Help {
        /// The command to describe, such as `compare` or `map probe`.
        path: Vec<String>,
        /// Every command, argument, exit code and verdict code as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// What a command prints: an envelope in the requested format, or a document of its
/// own (help, a JSON Schema, a writer module, an agent guide) printed as it is.
enum Printed {
    Result(Box<Output>),
    Raw(String),
}

#[derive(Debug, Subcommand)]
enum RefereeCommand {
    /// Create the referee's home: an age identity, the referee ledger and its state.
    Init { dir: PathBuf },
}

fn run(cli: &Cli, command: &Command, agent: Option<agent::Mode>) -> Result<Printed, Error> {
    if cli.format == Format::Jsonl && !matches!(command, Command::Compare(a) if a.items) {
        return Err(Error::usage_code(
            "conflicting_arguments",
            "--format jsonl prints per-item rows and needs compare --items",
        )
        .with_next(
            "evalsig compare --items --format jsonl",
            "one JSON object per item",
        ));
    }
    match command {
        Command::Help { path, json: true } => {
            use clap::CommandFactory as _;
            let out = Output::success(help::envelope(&Cli::command(), path)?);
            return Ok(Printed::Raw(output::render(&out, Format::Json)));
        }
        Command::Help { path, json: false } => {
            use clap::CommandFactory as _;
            let mut cmd = Cli::command();
            cmd.build();
            let mut at = &mut cmd;
            for name in path {
                at = at.find_subcommand_mut(name).ok_or_else(|| {
                    Error::usage_code(
                        "unknown_command",
                        format!("no command {:?}", path.join(" ")),
                    )
                    .with_next("evalsig help --json", "every command")
                })?;
            }
            return Ok(Printed::Raw(at.render_long_help().to_string()));
        }
        Command::Schema {
            name,
            emit: Some(lang),
            ..
        } => return commands::schema::emit(name, lang).map(Printed::Raw),
        Command::Schema {
            name, raw: true, ..
        } => return commands::schema::raw(name).map(Printed::Raw),
        Command::AgentGuide { snippet, .. } => {
            return Ok(Printed::Raw(if *snippet {
                commands::skill::snippet()
            } else {
                commands::skill::settings()
            }));
        }
        _ => {}
    }
    let out = match *command {
        Command::Init { force } => commands::init::run(&cli.project, force),
        Command::Agree(ref args) => evalsig_project::agree::run(&cli.project, &args.into()),
        Command::Check(ref args) => commands::check::run(&cli.project, args),
        Command::Compare(ref args) => evalsig_project::compare::run(&cli.project, &args.into()),
        Command::Capabilities { ref run } => {
            evalsig_project::list::capabilities(&cli.project, run.as_deref())
        }
        Command::Exclude(ref args) => commands::exclude::run(&cli.project, args),
        Command::Explain { ref name } => evalsig_project::explain::run(name),
        Command::Dashboard(ref args) => commands::dashboard::run(&cli.project, args),
        Command::Skill(ref cmd) => commands::skill::run(&cli.project, cmd),
        Command::AgentGuide { .. } | Command::Help { .. } => unreachable!("printed above"),
        Command::Export(ref args) => commands::export::run(&cli.project, args),
        Command::Gate(ref args) => commands::gate::run(&cli.project, args),
        Command::Import(ref args) => commands::import::run(&cli.project, args),
        Command::Items => evalsig_project::list::items(&cli.project),
        Command::Ledger(ref cmd) => commands::ledger::run(&cli.project, cmd),
        Command::Map(ref cmd) => commands::map::run(&cli.project, cmd),
        Command::Migrate(ref args) => commands::migrate::run(&cli.project, args),
        Command::Power(ref args) => commands::power::power(args),
        Command::Preregister(ref args) => {
            commands::preregister::run(&cli.project, args, agent.is_some())
        }
        Command::Purge(ref args) => commands::purge::run(&cli.project, args),
        Command::Referee(RefereeCommand::Init { ref dir }) => evalsig_referee::home::init(dir),
        Command::Report(ref args) => evalsig_project::report::run(&cli.project, &args.into()),
        Command::Runs => evalsig_project::list::runs(&cli.project),
        Command::Schema { ref name, .. } => commands::schema::run(name),
        Command::Seal(ref args) => commands::seal::run(&cli.project, args),
        Command::Split(ref args) => commands::split::run(&cli.project, args),
        Command::Threshold(ref args) => commands::power::threshold(&cli.project, args),
        Command::Trend(ref args) => evalsig_project::trend::run(&cli.project, &args.into()),
        Command::Splits => commands::split::list(&cli.project),
        Command::Recipes { ref recipe } => commands::recipes::run(recipe.as_ref()),
        Command::Validate {
            records: Some(ref records),
            list_checks,
            checks,
        } => {
            if list_checks || checks.allow_unexplained_missing || checks.fail_on_secret {
                return Err(Error::usage(
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
            Schema::Version,
            json!({
                "version": env!("CARGO_PKG_VERSION"),
                "records": evalsig_core::records::RECORDS_VERSION,
                "exit_codes": Exit::ALL.iter().map(|e| json!({"code": e.code(), "meaning": e.meaning()})).collect::<Vec<_>>(),
                "verdict_codes": VerdictCode::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            }),
        ))),
    }?;
    Ok(Printed::Result(Box::new(out)))
}

/// Whether `--format` appears on the command line at all.
fn requested_format_given(args: &[std::ffi::OsString]) -> bool {
    args.iter().any(|a| {
        let a = a.to_string_lossy();
        a == "--format" || a.starts_with("--format=")
    })
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

/// How an error reaches standard error.
#[derive(Clone, Copy)]
enum ErrorStyle {
    /// Agent mode: compact JSON, quoted values withheld unless content was asked for.
    Agent {
        include_content: bool,
    },
    Json,
    Text,
}

impl ErrorStyle {
    fn of(agent: bool, include_content: bool, json: bool) -> Self {
        match (agent, json) {
            (true, _) => ErrorStyle::Agent { include_content },
            (false, true) => ErrorStyle::Json,
            (false, false) => ErrorStyle::Text,
        }
    }
}

fn emit_error(err: &Error, style: ErrorStyle) -> ExitCode {
    let text = match style {
        ErrorStyle::Agent { include_content } => agent::error_json(err, include_content),
        ErrorStyle::Json => err.to_json(),
        ErrorStyle::Text => err.to_text(),
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
            use clap::CommandFactory as _;
            let has = |flag: &str| args.iter().any(|a| a.to_string_lossy() == flag);
            let agent_mode = agent::wanted(
                has("--agent"),
                has("--no-agent"),
                requested_format_given(&args),
                std::io::stdout().is_terminal(),
            );
            let style =
                ErrorStyle::of(agent_mode, has("--include-content"), format == Format::Json);
            if matches!(style, ErrorStyle::Text) {
                // clap's own message, with the usage line it adds.
                let _ = std::io::stderr().write_all(e.render().to_string().as_bytes());
                return ExitCode::from(Exit::Usage.code());
            }
            let mut root = Cli::command();
            root.build();
            let (path, at) = command_path(&root, &args);
            return emit_error(&parse_error(&e, at, &path), style);
        }
    };
    let format_given = requested_format_given(&args);
    if cli.agent && format_given {
        return emit_error(
            &Error::usage_code(
                "conflicting_arguments",
                "--agent prints JSON; leave out --format",
            ),
            ErrorStyle::Json,
        );
    }
    let agent_mode = agent::wanted(
        cli.agent,
        cli.no_agent,
        format_given,
        std::io::stdout().is_terminal(),
    );
    let mode = agent_mode.then_some(agent::Mode {
        cursor: cli.cursor,
        include_content: cli.include_content,
    });
    let Some(command) = &cli.command else {
        return bare(mode, cli.format);
    };
    // `help --json` describes the parser for programs, so its errors are JSON too.
    let help_json = matches!(command, Command::Help { json: true, .. });
    let style = ErrorStyle::of(
        agent_mode,
        cli.include_content,
        help_json || cli.format == Format::Json,
    );
    match run(&cli, command, mode) {
        Ok(Printed::Raw(text)) => {
            let _ = std::io::stdout().write_all(text.as_bytes());
            ExitCode::SUCCESS
        }
        Ok(Printed::Result(out)) => print(*out, mode, cli.format),
        Err(err) => emit_error(&err, style),
    }
}

/// Print a result: shaped compact JSON in agent mode, else the requested format.
fn print(mut out: Output, mode: Option<agent::Mode>, format: Format) -> ExitCode {
    let text = match mode {
        // Compact JSON: the size limit is measured on what is printed.
        Some(m) => {
            agent::shape(&mut out.envelope, m);
            let mut t = serde_json::to_string(&out.envelope).expect("envelope serializes");
            t.push('\n');
            t
        }
        None => output::render(&out, format),
    };
    let _ = std::io::stdout().write_all(text.as_bytes());
    ExitCode::from(out.exit.code())
}

/// `evalsig` with no command: the command index for programs, the usage and exit 64 for
/// people (D-040).
fn bare(mode: Option<agent::Mode>, format: Format) -> ExitCode {
    use clap::CommandFactory as _;
    let mut root = Cli::command();
    root.build();
    if mode.is_some() || format == Format::Json {
        return print(Output::success(help::index(&root)), mode, format);
    }
    let _ = std::io::stderr().write_all(root.render_help().to_string().as_bytes());
    ExitCode::from(Exit::Usage.code())
}

/// The subcommands named on the command line before the parser stopped, such as
/// `["map"]` for `evalsig map prob`, and the command they lead to. Options and their
/// values are skipped.
fn command_path<'a>(
    root: &'a clap::Command,
    args: &[std::ffi::OsString],
) -> (Vec<String>, &'a clap::Command) {
    let mut cmd = root;
    let mut path = Vec::new();
    let mut it = args
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned());
    while let Some(a) = it.next() {
        if let Some(long) = a.strip_prefix("--") {
            let takes_value = cmd
                .get_arguments()
                .any(|x| x.get_long() == Some(long) && x.get_action().takes_values());
            if takes_value {
                it.next();
            }
            continue;
        }
        if a.starts_with('-') {
            continue;
        }
        match cmd.find_subcommand(&a) {
            Some(c) => {
                path.push(a);
                cmd = c;
            }
            None => break,
        }
    }
    (path, cmd)
}

/// A parse error as evalsig reports it (D-040): a code for the kind of mistake, every
/// paragraph of the parser's message but the usage line, and the commands to run next,
/// built from the parser's own names.
fn parse_error(e: &clap::Error, cmd: &clap::Command, path: &[String]) -> Error {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    let rendered = e.render().to_string();
    let message = rendered
        .split("\n\n")
        .map(|p| {
            p.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|p| {
            !p.is_empty() && !p.starts_with("Usage:") && !p.starts_with("For more information")
        })
        .collect::<Vec<_>>()
        .join("; ");
    let mut message = message.trim_start_matches("error: ").to_owned();
    let code = match e.kind() {
        ErrorKind::InvalidSubcommand => "unknown_command",
        ErrorKind::UnknownArgument => "unknown_argument",
        ErrorKind::MissingRequiredArgument => "missing_argument",
        ErrorKind::MissingSubcommand | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            "missing_command"
        }
        ErrorKind::InvalidValue
        | ErrorKind::ValueValidation
        | ErrorKind::InvalidUtf8
        | ErrorKind::NoEquals
        | ErrorKind::TooManyValues
        | ErrorKind::TooFewValues
        | ErrorKind::WrongNumberOfValues => "invalid_value",
        ErrorKind::ArgumentConflict => "conflicting_arguments",
        _ => "usage",
    };
    let strings = |k: ContextKind| match e.get(k) {
        Some(ContextValue::String(s)) => vec![s.clone()],
        Some(ContextValue::Strings(v)) => v.clone(),
        _ => Vec::new(),
    };
    let named = |rest: &[&str]| {
        std::iter::once("evalsig")
            .chain(path.iter().map(String::as_str))
            .chain(rest.iter().copied())
            .collect::<Vec<_>>()
            .join(" ")
    };
    if code == "missing_command" {
        // The parser prints the whole help here; name the subcommands instead.
        message = format!(
            "{} needs a subcommand: {}",
            path.join(" "),
            help::visible(cmd)
                .map(clap::Command::get_name)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let mut err = Error::usage_code(code, message);
    for s in strings(ContextKind::SuggestedSubcommand) {
        err = err.with_next(named(&[&s]), "the command of the nearest name");
    }
    let mut why = vec![if path.is_empty() {
        "every command".to_owned()
    } else {
        "the arguments of this command".to_owned()
    }];
    let valid = strings(ContextKind::ValidValue);
    if !valid.is_empty() {
        why.push(format!("valid values: {}", valid.join(", ")));
    }
    let suggested = strings(ContextKind::SuggestedArg);
    if !suggested.is_empty() {
        why.push(format!("the nearest option: {}", suggested.join(", ")));
    }
    let help = format!("evalsig help --json {}", path.join(" "));
    err.with_next(help.trim_end(), why.join("; "))
}
