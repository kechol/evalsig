//! `docs/cli.md` is rendered from `help --json`, so every option, verdict code and exit
//! code the binary has is documented. `EVALSIG_UPDATE_DOCS=1` rewrites the file.

use std::fmt::Write;

use serde_json::Value;

mod common;
use common::*;

const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cli.md");

fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn option(a: &Value) -> String {
    let name = a["long"].as_str().unwrap_or(a["name"].as_str().unwrap());
    let mut s = if a["positional"].as_bool().unwrap() {
        format!("`<{}>`", a["value_name"].as_str().unwrap_or(name))
    } else {
        let mut s = format!("`--{name}");
        if a["takes_value"].as_bool().unwrap() {
            write!(s, " <{}>", a["value_name"].as_str().unwrap()).unwrap();
        }
        s.push('`');
        if let Some(short) = a["short"].as_str() {
            write!(s, ", `-{short}`").unwrap();
        }
        s
    };
    if a["required"].as_bool().unwrap() {
        s.push_str(" (required)");
    }
    s
}

fn args_table(out: &mut String, args: &[Value]) {
    if args.is_empty() {
        out.push_str("No options.\n\n");
        return;
    }
    out.push_str("| Option | Description | Values | Default |\n|---|---|---|---|\n");
    for a in args {
        let values = a["possible_values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| format!("`{}`", v.as_str().unwrap()))
            .collect::<Vec<_>>()
            .join(", ");
        let default = match a["default"].as_str() {
            Some(d) if a["takes_value"].as_bool().unwrap() => format!("`{d}`"),
            _ => String::new(),
        };
        writeln!(
            out,
            "| {} | {} | {values} | {default} |",
            option(a),
            cell(a["help"].as_str().unwrap_or_default())
        )
        .unwrap();
    }
    out.push('\n');
}

fn command(out: &mut String, c: &Value, path: &str) {
    let path = format!("{path} {}", c["name"].as_str().unwrap());
    writeln!(out, "### `{path}`\n").unwrap();
    if let Some(about) = c["about"].as_str() {
        writeln!(out, "{about}\n").unwrap();
    }
    args_table(out, c["args"].as_array().unwrap());
    for g in c["required_groups"].as_array().unwrap() {
        let names = g
            .as_array()
            .unwrap()
            .iter()
            .map(|n| format!("`--{}`", n.as_str().unwrap().replace('_', "-")))
            .collect::<Vec<_>>()
            .join(" or ");
        writeln!(out, "One of {names} is required.\n").unwrap();
    }
    for s in c["subcommands"].as_array().unwrap() {
        command(out, s, &path);
    }
}

fn render(help: &Value) -> String {
    let r = &help["results"];
    let mut out = String::from(
        "# CLI reference\n\n\
         This page is generated from `evalsig help --json` by\n\
         `crates/evalsig-cli/tests/cli_reference.rs`; a test fails when it differs from the\n\
         binary. Rewrite it with `EVALSIG_UPDATE_DOCS=1 cargo test -p evalsig-cli --test\n\
         cli_reference`. The terms are those of [the glossary](glossary.md); how agents\n\
         read the output is in [the agent interface](agent-interface.md).\n\n\
         ## Global options\n\n\
         These apply to every command.\n\n",
    );
    args_table(&mut out, r["global_args"].as_array().unwrap());
    out.push_str("## Commands\n\n");
    for c in r["commands"].as_array().unwrap() {
        command(&mut out, c, "evalsig");
    }
    out.push_str("## Verdict codes\n\n| Code | Meaning |\n|---|---|\n");
    for v in r["verdict_codes"].as_array().unwrap() {
        writeln!(
            out,
            "| `{}` | {} |",
            v["code"].as_str().unwrap(),
            cell(v["meaning"].as_str().unwrap())
        )
        .unwrap();
    }
    out.push_str("\n## Exit codes\n\n| Code | Meaning |\n|---|---|\n");
    for e in r["exit_codes"].as_array().unwrap() {
        writeln!(
            out,
            "| {} | {} |",
            e["code"],
            cell(e["meaning"].as_str().unwrap())
        )
        .unwrap();
    }
    out
}

#[test]
fn the_cli_reference_matches_the_binary() {
    let dir = tempfile::tempdir().unwrap();
    let out = evalsig(dir.path())
        .args(["help", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let page = render(&serde_json::from_slice(&out.stdout).unwrap());
    if std::env::var("EVALSIG_UPDATE_DOCS").is_ok_and(|v| v == "1") {
        std::fs::write(PATH, &page).unwrap();
    }
    let written = std::fs::read_to_string(PATH).unwrap_or_default();
    assert!(
        written == page,
        "docs/cli.md differs from `help --json`; rewrite it with EVALSIG_UPDATE_DOCS=1"
    );
}
