//! `evalsig help --json` (agent-interface spec §2.6): every command, argument and
//! code, read from the clap definitions themselves so the description cannot drift
//! from the parser.

use clap::Command;
use serde_json::{Value, json};

use crate::exit::Exit;
use crate::output::{Envelope, VerdictCode};

fn args(cmd: &Command) -> Vec<Value> {
    cmd.get_arguments()
        .filter(|a| !a.is_hide_set() && a.get_id() != "help" && a.get_id() != "version")
        .map(|a| {
            json!({
                "name": a.get_id().as_str(),
                "long": a.get_long(),
                "short": a.get_short().map(String::from),
                "positional": a.is_positional(),
                "required": a.is_required_set(),
                "global": a.is_global_set(),
                "takes_value": a.get_action().takes_values(),
                "value_name": a.get_value_names().and_then(|v| v.first()).map(|v| v.as_str()),
                "possible_values": a.get_possible_values().iter().map(|p| p.get_name()).collect::<Vec<_>>(),
                "default": a.get_default_values().first().map(|v| v.to_string_lossy()),
                "help": a.get_long_help().or(a.get_help()).map(|h| h.to_string()),
            })
        })
        .collect()
}

fn command(cmd: &Command) -> Value {
    json!({
        "name": cmd.get_name(),
        "about": cmd.get_about().map(|a| a.to_string()),
        "args": args(cmd),
        "subcommands": cmd
            .get_subcommands()
            .filter(|c| c.get_name() != "help")
            .map(command)
            .collect::<Vec<_>>(),
    })
}

pub fn envelope(root: &Command) -> Envelope {
    let mut root = root.clone();
    root.build();
    Envelope::new(
        "evalsig.help/1",
        json!({
            "tool": root.get_name(),
            "global_args": args(&root),
            "commands": root
                .get_subcommands()
                .filter(|c| c.get_name() != "help")
                .map(command)
                .collect::<Vec<_>>(),
            "exit_codes": Exit::ALL.iter().map(|e| json!({"code": e.code(), "meaning": e.meaning()})).collect::<Vec<_>>(),
            "verdict_codes": VerdictCode::ALL.iter().map(|c| json!({"code": c.as_str(), "meaning": c.meaning()})).collect::<Vec<_>>(),
        }),
    )
}
