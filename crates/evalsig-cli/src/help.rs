//! `evalsig help --json` (agent-interface spec §2.6): every command, argument and
//! code, read from the clap definitions themselves so the description cannot drift
//! from the parser.

use clap::{Arg, Command};
use serde_json::{Value, json};

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, VerdictCode};

fn arg(a: &Arg) -> Value {
    json!({
        "name": a.get_id().as_str(),
        "long": a.get_long(),
        "short": a.get_short().map(String::from),
        "positional": a.is_positional(),
        "required": a.is_required_set(),
        "takes_value": a.get_action().takes_values(),
        "value_name": a.get_value_names().and_then(|v| v.first()).map(|v| v.as_str()),
        "possible_values": a.get_possible_values().iter().map(|p| p.get_name()).collect::<Vec<_>>(),
        "default": a.get_default_values().first().map(|v| v.to_string_lossy()),
        "help": a.get_long_help().or(a.get_help()).map(|h| h.to_string()),
    })
}

/// A command's arguments, global ones apart: those are listed once, at the top.
fn args(cmd: &Command, global: bool) -> Vec<Value> {
    cmd.get_arguments()
        .filter(|a| {
            !a.is_hide_set()
                && a.is_global_set() == global
                && a.get_id() != "help"
                && a.get_id() != "version"
        })
        .map(arg)
        .collect()
}

/// The subcommands, without clap's own `help`.
fn subcommands(cmd: &Command) -> Vec<Value> {
    cmd.get_subcommands()
        .filter(|c| c.get_name() != "help")
        .map(command)
        .collect()
}

fn command(cmd: &Command) -> Value {
    json!({
        "name": cmd.get_name(),
        "about": cmd.get_about().map(|a| a.to_string()),
        "args": args(cmd, false),
        // Sets of arguments of which one is required, such as agent-guide's.
        "required_groups": cmd
            .get_groups()
            .filter(|g| g.is_required_set())
            .map(|g| g.get_args().map(|a| a.as_str()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        "subcommands": subcommands(cmd),
    })
}

/// The whole tree, or only the command at `path` (`help --json compare`).
pub fn envelope(root: &Command, path: &[String]) -> Result<Envelope, CliError> {
    let mut root = root.clone();
    root.build();
    if !path.is_empty() {
        let mut cmd = &root;
        for name in path {
            cmd = cmd.find_subcommand(name).ok_or_else(|| {
                CliError::usage(format!("no command {:?}", path.join(" ")))
                    .with_hint("list the commands with evalsig help --json")
            })?;
        }
        return Ok(Envelope::new(
            "evalsig.help/1",
            json!({ "tool": root.get_name(), "global_args": args(&root, true), "commands": [command(cmd)] }),
        ));
    }
    Ok(Envelope::new(
        "evalsig.help/1",
        json!({
            "tool": root.get_name(),
            "global_args": args(&root, true),
            "commands": subcommands(&root),
            "exit_codes": Exit::ALL.iter().map(|e| json!({"code": e.code(), "meaning": e.meaning()})).collect::<Vec<_>>(),
            "verdict_codes": VerdictCode::ALL.iter().map(|c| json!({"code": c.as_str(), "meaning": c.meaning()})).collect::<Vec<_>>(),
        }),
    ))
}
