//! `evalsig skill install|check` and `evalsig agent-guide` (agent-interface spec §3):
//! the Skill, the AGENTS.md snippet and the permission template, generated from the
//! copies embedded in the binary, stamped with its version.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output, Verdict, VerdictCode, Warning};
use crate::project::io_error;

const SKILL: [(&str, &str); 3] = [
    ("SKILL.md", include_str!("../../skill/evalsig/SKILL.md")),
    (
        "references/improve-loop.md",
        include_str!("../../skill/evalsig/references/improve-loop.md"),
    ),
    (
        "references/onboarding.md",
        include_str!("../../skill/evalsig/references/onboarding.md"),
    ),
];
const SNIPPET: &str = include_str!("../../skill/AGENTS.md.snippet");
const SETTINGS: &str = include_str!("../../skill/claude-settings.json");

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn stamp(text: &str) -> String {
    text.replace("{{version}}", VERSION)
}

/// The Skill's files as this binary writes them.
pub fn files() -> Vec<(&'static str, String)> {
    SKILL.iter().map(|(n, t)| (*n, stamp(t))).collect()
}

pub fn snippet() -> String {
    stamp(SNIPPET)
}

pub fn settings() -> String {
    stamp(SETTINGS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Target {
    Claude,
    Codex,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Scope {
    Project,
    User,
}

#[derive(Debug, Clone, clap::Args)]
pub struct Place {
    #[arg(long, value_enum, default_value_t = Target::Claude)]
    pub target: Target,
    #[arg(long, value_enum, default_value_t = Scope::Project)]
    pub scope: Scope,
    /// Write to (or check) `<DIR>/evalsig` instead; required with --target generic.
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
}

#[derive(Debug, clap::Subcommand)]
pub enum SkillCommand {
    /// Write the evalsig Skill where the agent reads skills.
    Install {
        #[command(flatten)]
        place: Place,
        /// Replace an installed copy that differs.
        #[arg(long)]
        force: bool,
    },
    /// Compare an installed Skill with the one this binary writes.
    Check {
        #[command(flatten)]
        place: Place,
    },
}

/// The Skill's directory: Claude Code reads `.claude/skills/` in the project and in
/// the home directory; Codex documents `$CODEX_HOME/skills` (default `~/.codex/skills`)
/// only, so it has no project scope here.
fn skill_dir(root: &Path, place: &Place) -> Result<PathBuf, CliError> {
    if let Some(d) = &place.dir {
        let dir = d.join("evalsig");
        crate::project::outside_store(&dir)?;
        return Ok(dir);
    }
    let home = || {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| CliError::usage("HOME is not set; pass --dir"))
    };
    match (place.target, place.scope) {
        (Target::Claude, Scope::Project) => Ok(root.join(".claude/skills/evalsig")),
        (Target::Claude, Scope::User) => Ok(home()?.join(".claude/skills/evalsig")),
        (Target::Codex, Scope::User) => Ok(std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .map_or_else(|| home().map(|h| h.join(".codex")), Ok)?
            .join("skills/evalsig")),
        (Target::Codex, Scope::Project) => Err(CliError::usage(
            "Codex reads skills from $CODEX_HOME/skills; use --scope user or --dir",
        )),
        (Target::Generic, _) => Err(CliError::usage("--target generic needs --dir")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Missing,
    Differs,
    Current,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            State::Missing => "missing",
            State::Differs => "differs",
            State::Current => "current",
        }
    }
}

/// Each embedded file against the installed one.
fn compare(dir: &Path, files: &[(&'static str, String)]) -> Vec<(&'static str, State)> {
    files
        .iter()
        .map(|(name, want)| {
            let state = match std::fs::read_to_string(dir.join(name)) {
                Err(_) => State::Missing,
                Ok(have) if have == *want => State::Current,
                Ok(_) => State::Differs,
            };
            (*name, state)
        })
        .collect()
}

pub fn run(root: &Path, cmd: &SkillCommand) -> Result<Output, CliError> {
    let files = files();
    match cmd {
        SkillCommand::Install { place, force } => {
            let dir = skill_dir(root, place)?;
            if !force
                && compare(&dir, &files)
                    .iter()
                    .any(|(_, s)| *s == State::Differs)
            {
                return Err(CliError::usage(format!(
                    "{} holds a Skill that differs from this evalsig's",
                    dir.display()
                ))
                .with_hint(
                    "see the differences with evalsig skill check; pass --force to replace it",
                ));
            }
            for (name, text) in &files {
                let p = dir.join(name);
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| io_error(parent, &e))?;
                }
                std::fs::write(&p, text).map_err(|e| io_error(&p, &e))?;
            }
            Ok(Output::success(Envelope::new(
                "evalsig.skill/1",
                json!({
                    "dir": dir.display().to_string(),
                    "version": VERSION,
                    "files": files.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
                }),
            )))
        }
        SkillCommand::Check { place } => {
            let dir = skill_dir(root, place)?;
            let states = compare(&dir, &files);
            // The stamp `for evalsig <version>` in the installed SKILL.md.
            let version = std::fs::read_to_string(dir.join("SKILL.md"))
                .ok()
                .and_then(|t| {
                    t.split_once("for evalsig ").and_then(|(_, r)| {
                        r.split_whitespace()
                            .next()
                            .map(|v| v.trim_end_matches('.').to_owned())
                    })
                });
            let current = states.iter().all(|(_, s)| *s == State::Current);
            let mut e = Envelope::new(
                "evalsig.skill/1",
                json!({
                    "dir": dir.display().to_string(),
                    "version": VERSION,
                    "installed_version": version,
                    "files": states.iter().map(|(n, s)| json!({"file": n, "state": s.as_str()})).collect::<Vec<_>>(),
                }),
            );
            if !current {
                let (code, message) = if version.as_deref().is_some_and(|v| v != VERSION) {
                    (
                        "skill_version_drift",
                        "the installed Skill was written by another evalsig version; run evalsig skill install --force",
                    )
                } else {
                    (
                        "skill_modified",
                        "the installed Skill is missing files or was edited; run evalsig skill install --force to restore it",
                    )
                };
                e.warnings.push(Warning {
                    code: code.into(),
                    message: message.into(),
                });
            }
            let (code, text, exit) = if current {
                (
                    VerdictCode::Pass,
                    "the installed Skill matches this evalsig",
                    Exit::Success,
                )
            } else {
                (
                    VerdictCode::Fail,
                    "the installed Skill does not match this evalsig",
                    Exit::Failed,
                )
            };
            e.verdict = Some(Verdict {
                code,
                text: text.into(),
            });
            Ok(Output { envelope: e, exit })
        }
    }
}
