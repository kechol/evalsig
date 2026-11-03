//! The referee's home (D-014): its age identity, its ledger and its state. Holding a
//! readable identity is the referee's authority; commands that need it fail with
//! `referee_only` otherwise.

use std::path::{Path, PathBuf};

use evalsig_store::ledger::{self, Entry, Kind, LedgerSide, NewEntry};
use evalsig_store::seal;
use serde_json::{Value, json};

use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output};
use crate::project::{actor, io_error, now};

pub const ENV: &str = "EVALSIG_REFEREE";
const IDENTITY: &str = "identity.txt";
const RECIPIENT: &str = "recipient.txt";
const SUBDIRS: [&str; 5] = ["holdouts", "sealed", "approved", "results", "state"];

/// An opened referee home.
#[derive(Debug, Clone)]
pub struct Referee {
    pub dir: PathBuf,
}

fn referee_only(message: impl Into<String>) -> CliError {
    CliError::new("referee_only", message, Exit::Rejected).with_hint(
        "run this as the referee, with --referee <dir> or EVALSIG_REFEREE naming its home",
    )
}

#[cfg(unix)]
fn private(path: &Path) -> Result<(), CliError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| io_error(path, &e))
}

#[cfg(not(unix))]
fn private(_: &Path) -> Result<(), CliError> {
    Ok(())
}

/// `evalsig referee init <dir>`: a new home with a fresh identity.
pub fn init(dir: &Path) -> Result<Output, CliError> {
    if dir.join(IDENTITY).exists() {
        return Err(CliError::usage(format!(
            "{} already holds a referee identity",
            dir.display()
        )));
    }
    for sub in SUBDIRS {
        let p = dir.join(sub);
        std::fs::create_dir_all(&p).map_err(|e| io_error(&p, &e))?;
    }
    let (secret, recipient) = seal::generate();
    let id = dir.join(IDENTITY);
    std::fs::write(&id, format!("# public key: {recipient}\n{secret}\n"))
        .map_err(|e| io_error(&id, &e))?;
    private(&id)?;
    let r = dir.join(RECIPIENT);
    std::fs::write(&r, format!("{recipient}\n")).map_err(|e| io_error(&r, &e))?;
    Ok(Output::success(Envelope::new(
        "evalsig.referee/1",
        json!({ "home": dir.display().to_string(), "recipient": recipient }),
    )))
}

impl Referee {
    /// The home named by `arg` or `EVALSIG_REFEREE`, if its identity can be read.
    pub fn open(arg: Option<&Path>) -> Result<Referee, CliError> {
        let dir = match arg {
            Some(p) => p.to_path_buf(),
            None => std::env::var_os(ENV)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| referee_only("this command needs the referee's authority"))?,
        };
        let id = dir.join(IDENTITY);
        let text = std::fs::read_to_string(&id).map_err(|_| {
            referee_only(format!("no readable referee identity at {}", id.display()))
        })?;
        seal::recipient_of(&text)
            .map_err(|_| referee_only(format!("{} is not an age identity", id.display())))?;
        Ok(Referee { dir })
    }

    pub fn recipient(&self) -> Result<String, CliError> {
        let p = self.dir.join(RECIPIENT);
        Ok(std::fs::read_to_string(&p)
            .map_err(|e| io_error(&p, &e))?
            .trim()
            .to_owned())
    }

    /// The items of a holdout this referee cut, by its hash.
    pub fn holdout_items(
        &self,
        hash: &str,
    ) -> Result<std::collections::BTreeSet<String>, CliError> {
        let p = self.path("holdouts").join(format!(
            "{}.json",
            hash.strip_prefix("blake3:").unwrap_or(hash)
        ));
        let text = std::fs::read_to_string(&p)
            .map_err(|_| CliError::usage(format!("this referee has cut no holdout {hash}")))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| CliError::invalid_input(&p, e))?;
        Ok(v["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i.as_str().map(str::to_owned))
            .collect())
    }

    pub fn path(&self, sub: &str) -> PathBuf {
        self.dir.join(sub)
    }

    pub fn ledger_path(&self) -> PathBuf {
        self.dir.join("ledger.jsonl")
    }

    pub fn entries(&self) -> Result<Vec<Entry>, CliError> {
        if !self.ledger_path().exists() {
            return Ok(Vec::new());
        }
        ledger::read(&self.ledger_path()).map_err(CliError::store)
    }

    /// Append a referee ledger line.
    pub fn record(
        &self,
        kind: Kind,
        inputs: Vec<String>,
        outputs: Vec<String>,
        summary: Value,
    ) -> Result<Entry, CliError> {
        let Value::Object(summary) = summary else {
            panic!("a ledger summary is a JSON object");
        };
        ledger::append(
            &self.ledger_path(),
            LedgerSide::Referee,
            kind,
            NewEntry {
                ts: now(),
                actor: actor(),
                inputs,
                outputs,
                summary,
                tool: crate::tool_version(),
            },
        )
        .map_err(CliError::store)
    }
}
