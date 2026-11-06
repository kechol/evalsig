//! The referee's home (D-014): its age identity, its ledger and its state. Holding a
//! readable identity is the referee's authority; commands that need it fail with
//! `referee_only` otherwise.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use evalsig_store::ledger::{self, Entry, Kind, LedgerSide, NewEntry};
use evalsig_store::seal;
use serde_json::{Value, json};

use evalsig_project::Error;
use evalsig_project::envelope::{Envelope, Output};
use evalsig_project::exit::Exit;
use evalsig_project::project::{actor, hex, io_error, now, str_set};

pub const ENV: &str = "EVALSIG_REFEREE";
const IDENTITY: &str = "identity.txt";
const LEDGER: &str = "ledger.jsonl";
const RECIPIENT: &str = "recipient.txt";
const SUBDIRS: [&str; 6] = [
    "holdouts",
    "sealed",
    "approved",
    "results",
    "state",
    "exclusions",
];

/// An opened referee home.
#[derive(Debug, Clone)]
pub struct Referee {
    pub dir: PathBuf,
}

fn referee_only(message: impl Into<String>) -> Error {
    Error::new("referee_only", message, Exit::Rejected).with_hint(
        "run this as the referee, with --referee <dir> or EVALSIG_REFEREE naming its home",
    )
}

/// Only the referee's user may use `path`: files 0600, directories 0700. A group the
/// agent belongs to may then traverse the home to reach `gate.sock` and nothing else.
#[cfg(unix)]
pub fn private(path: &Path) -> Result<(), Error> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if path.is_dir() { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| io_error(path, &e))
}

#[cfg(not(unix))]
pub fn private(_: &Path) -> Result<(), Error> {
    Ok(())
}

/// Create `path` holding `bytes`, readable by the referee's user only from the
/// moment it exists.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(path)
        .and_then(|mut f| f.write_all(bytes))
        .map_err(|e| io_error(path, &e))
}

/// Replace `path` with `bytes`, readable by the referee's user only, so that a reader
/// sees the old file or the new one and never a part.
pub fn replace_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let tmp = path.with_extension("tmp");
    // A temporary file left by an interrupted write is stale; the home's lock is held.
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io_error(&tmp, &e)),
        _ => {}
    }
    write_private(&tmp, bytes)?;
    // On disk before it replaces the old file, so a crash cannot leave a short one.
    std::fs::File::open(&tmp)
        .and_then(|f| f.sync_all())
        .map_err(|e| io_error(&tmp, &e))?;
    std::fs::rename(&tmp, path).map_err(|e| io_error(path, &e))
}

/// `evalsig referee init <dir>`: a new home with a fresh identity.
pub fn init(dir: &Path) -> Result<Output, Error> {
    if dir.join(IDENTITY).exists() {
        return Err(Error::usage(format!(
            "{} already holds a referee identity",
            dir.display()
        )));
    }
    // The home is made private, so it must be the referee's own: new or empty.
    if std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
        return Err(Error::usage(format!(
            "{} is not empty; a referee home starts in a new or empty directory",
            dir.display()
        )));
    }
    std::fs::create_dir_all(dir).map_err(|e| io_error(dir, &e))?;
    private(dir)?;
    for sub in SUBDIRS {
        let p = dir.join(sub);
        std::fs::create_dir_all(&p).map_err(|e| io_error(&p, &e))?;
        private(&p)?;
    }
    let (secret, recipient) = seal::generate();
    let id = dir.join(IDENTITY);
    write_private(
        &id,
        format!("# public key: {recipient}\n{secret}\n").as_bytes(),
    )?;
    let r = dir.join(RECIPIENT);
    std::fs::write(&r, format!("{recipient}\n")).map_err(|e| io_error(&r, &e))?;
    // The home may be opened to a group for the socket, so the ledger is private from
    // the moment it exists.
    write_private(&dir.join(LEDGER), b"")?;
    Ok(Output::success(Envelope::new(
        "evalsig.referee/1",
        json!({ "home": dir.display().to_string(), "recipient": recipient }),
    )))
}

fn item_ids(p: &Path, text: &str) -> Result<BTreeSet<String>, Error> {
    let v: Value = serde_json::from_str(text).map_err(|e| Error::invalid_input(p, e))?;
    Ok(str_set(&v["items"]))
}

/// The items named by the `exclude` lines of the referee ledger.
fn excluded(referee: &Referee, entries: &[Entry]) -> Result<BTreeSet<String>, Error> {
    let mut out = BTreeSet::new();
    for e in entries.iter().filter(|e| e.kind == Kind::Exclude) {
        let Some(h) = e.inputs.first() else { continue };
        let p = referee.file("exclusions", h, "json");
        let text = std::fs::read_to_string(&p).map_err(|e| io_error(&p, &e))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| Error::invalid_input(&p, e))?;
        out.extend(str_set(&v));
    }
    Ok(out)
}

impl Referee {
    /// The home, as `open` finds it, locked against every other writer while the
    /// returned guard lives.
    pub fn open_exclusive(arg: Option<&Path>) -> Result<(Referee, std::fs::File), Error> {
        let referee = Referee::open(arg)?;
        let lock = referee.lock()?;
        Ok((referee, lock))
    }

    /// The home named by `arg` or `EVALSIG_REFEREE`, if its identity can be read.
    pub fn open(arg: Option<&Path>) -> Result<Referee, Error> {
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

    pub fn identity(&self) -> Result<String, Error> {
        let p = self.dir.join(IDENTITY);
        std::fs::read_to_string(&p).map_err(|e| io_error(&p, &e))
    }

    pub fn recipient(&self) -> Result<String, Error> {
        let p = self.dir.join(RECIPIENT);
        Ok(std::fs::read_to_string(&p)
            .map_err(|e| io_error(&p, &e))?
            .trim()
            .to_owned())
    }

    /// The items of a holdout this referee cut, by its hash, less the items the
    /// referee excluded (D-027).
    pub fn holdout_items(&self, hash: &str) -> Result<BTreeSet<String>, Error> {
        self.holdout_items_in(hash, &self.entries()?)
    }

    /// As `holdout_items`, with the referee ledger already read.
    pub fn holdout_items_in(
        &self,
        hash: &str,
        entries: &[Entry],
    ) -> Result<BTreeSet<String>, Error> {
        let p = self.file("holdouts", hash, "json");
        let text = std::fs::read_to_string(&p)
            .map_err(|_| Error::usage(format!("this referee has cut no holdout {hash}")))?;
        let excluded = excluded(self, entries)?;
        Ok(item_ids(&p, &text)?
            .into_iter()
            .filter(|i| !excluded.contains(i))
            .collect())
    }

    /// Every item of every holdout this referee cut.
    pub fn all_holdout_items(&self) -> Result<BTreeSet<String>, Error> {
        let dir = self.path("holdouts");
        let mut out = BTreeSet::new();
        for entry in std::fs::read_dir(&dir).map_err(|e| io_error(&dir, &e))? {
            let p = entry.map_err(|e| io_error(&dir, &e))?.path();
            let text = std::fs::read_to_string(&p).map_err(|e| io_error(&p, &e))?;
            out.extend(item_ids(&p, &text)?);
        }
        Ok(out)
    }

    pub fn path(&self, sub: &str) -> PathBuf {
        self.dir.join(sub)
    }

    /// `<sub>/<hex>.<ext>` in the home, for a file named by a hash.
    pub fn file(&self, sub: &str, hash: &str, ext: &str) -> PathBuf {
        self.dir.join(sub).join(format!("{}.{ext}", hex(hash)))
    }

    /// An exclusive lock on the home, held while the guard lives: one operation that
    /// writes the home at a time, whichever process runs it.
    pub fn lock(&self) -> Result<std::fs::File, Error> {
        let p = self.path("state").join("gate.lock");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&p)
            .map_err(|e| io_error(&p, &e))?;
        f.lock().map_err(|e| io_error(&p, &e))?;
        Ok(f)
    }

    pub fn ledger_path(&self) -> PathBuf {
        self.dir.join(LEDGER)
    }

    pub fn entries(&self) -> Result<Vec<Entry>, Error> {
        if !self.ledger_path().exists() {
            return Ok(Vec::new());
        }
        ledger::read(&self.ledger_path()).map_err(Error::store)
    }

    /// Append a referee ledger line.
    pub fn record(
        &self,
        kind: Kind,
        inputs: Vec<String>,
        outputs: Vec<String>,
        summary: Value,
    ) -> Result<Entry, Error> {
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
                tool: evalsig_project::tool_version(),
            },
        )
        .map_err(Error::store)
        .and_then(|e| private(&self.ledger_path()).map(|()| e))
    }

    /// Whether the agent could read the key: the home lies inside the project, or the
    /// identity is readable by group or others (D-014).
    pub fn seal_void(&self, project: &evalsig_project::project::Project) -> bool {
        let inside = match (
            self.dir.canonicalize(),
            project.dir.parent().map(Path::canonicalize),
        ) {
            (Ok(home), Some(Ok(root))) => home.starts_with(root),
            _ => false,
        };
        inside || shared(&self.dir.join(IDENTITY))
    }
}

#[cfg(unix)]
fn shared(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o077 != 0)
}

#[cfg(not(unix))]
fn shared(_: &Path) -> bool {
    false
}
