//! Append-only, hash-chained ledgers (D-007, audit spec §4).
//!
//! Each line is a JSON object whose `hash` covers the canonical bytes of the line
//! without `hash`, and whose `prev` is the previous line's `hash` (`genesis` first).
//! The agent-side ledger and the referee ledger are separate chains with separate
//! sets of allowed kinds. Appends hold an exclusive file lock, so concurrent writers
//! from several processes still produce one valid chain.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

use crate::canonical::{CanonicalError, to_canonical_bytes};
use crate::hash::{ContentHash, HashRefError};

pub const GENESIS: &str = "genesis";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerSide {
    Agent,
    Referee,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Import,
    Migrate,
    MappingDecision,
    LabelRevision,
    Adjudication,
    Exclude,
    Split,
    Seal,
    PreregisterDraft,
    PreregisterApprove,
    Report,
    Compare,
    Gate,
    HoldoutAccess,
    Purge,
    Verify,
    Anchor,
}

impl Kind {
    pub const ALL: [Kind; 17] = [
        Kind::Import,
        Kind::Migrate,
        Kind::MappingDecision,
        Kind::LabelRevision,
        Kind::Adjudication,
        Kind::Exclude,
        Kind::Split,
        Kind::Seal,
        Kind::PreregisterDraft,
        Kind::PreregisterApprove,
        Kind::Report,
        Kind::Compare,
        Kind::Gate,
        Kind::HoldoutAccess,
        Kind::Purge,
        Kind::Verify,
        Kind::Anchor,
    ];

    /// Whether this kind may be written to the given ledger (audit spec §4 table).
    /// `split`, `exclude`, `label_revision` and `adjudication` appear in both: the
    /// agent splits dev/validation and handles non-holdout items, the referee carves
    /// out the holdout and handles holdout items.
    pub fn allowed_in(self, side: LedgerSide) -> bool {
        use Kind::*;
        match side {
            LedgerSide::Agent => matches!(
                self,
                Import
                    | MappingDecision
                    | Migrate
                    | Report
                    | Compare
                    | PreregisterDraft
                    | LabelRevision
                    | Adjudication
                    | Exclude
                    | Split
                    | Purge
                    | Verify
            ),
            LedgerSide::Referee => matches!(
                self,
                Split
                    | Seal
                    | PreregisterApprove
                    | Gate
                    | HoldoutAccess
                    | Exclude
                    | LabelRevision
                    | Adjudication
                    | Anchor
            ),
        }
    }
}

/// One ledger line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub seq: u64,
    pub ts: String,
    pub actor: String,
    pub kind: Kind,
    pub ledger: LedgerSide,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub summary: Map<String, Json>,
    pub tool: String,
    pub prev: String,
    pub hash: String,
}

/// What a caller supplies; the ledger fills in `seq`, `prev` and `hash`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewEntry {
    pub ts: String,
    pub actor: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub summary: Map<String, Json>,
    pub tool: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("line {line}: {message}")]
    Parse { line: usize, message: String },
    #[error("kind `{kind:?}` is not allowed in the {side:?} ledger")]
    KindNotAllowed { kind: Kind, side: LedgerSide },
    #[error(
        "ledger summaries hold no floating-point numbers; write rounded strings (found at `{0}`)"
    )]
    FloatInSummary(String),
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    #[error(transparent)]
    HashRef(#[from] HashRefError),
}

/// A problem found by [`verify`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    Seq { line: usize, found: u64 },
    Prev { line: usize },
    Hash { line: usize },
    Side { line: usize },
    Unparsable { line: usize, message: String },
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::Seq { line, found } => {
                write!(f, "line {line}: seq is {found}, expected {}", line - 1)
            }
            Problem::Prev { line } => write!(
                f,
                "line {line}: prev does not match the previous line's hash"
            ),
            Problem::Hash { line } => write!(
                f,
                "line {line}: hash does not match the content (rewritten?)"
            ),
            Problem::Side { line } => write!(
                f,
                "line {line}: entry belongs to the other ledger or kind not allowed here"
            ),
            Problem::Unparsable { line, message } => write!(f, "line {line}: {message}"),
        }
    }
}

/// The hash of an entry: BLAKE3 of the canonical entry without `hash`, unless the
/// entry's existing hash names sha256 (ledgers written by other tools).
pub fn entry_hash(entry: &Entry) -> Result<String, LedgerError> {
    let mut v = serde_json::to_value(entry).map_err(CanonicalError::from)?;
    let algorithm = entry
        .hash
        .parse::<ContentHash>()
        .map(|h| h.algorithm())
        .unwrap_or(crate::hash::Algorithm::Blake3);
    if let Json::Object(m) = &mut v {
        m.remove("hash");
    }
    let bytes = to_canonical_bytes(&v)?;
    Ok(ContentHash::of(algorithm, &bytes).to_string())
}

fn check_no_floats(v: &Json, path: &str) -> Result<(), LedgerError> {
    match v {
        Json::Number(n) if n.is_f64() => Err(LedgerError::FloatInSummary(path.to_owned())),
        Json::Array(xs) => xs
            .iter()
            .enumerate()
            .try_for_each(|(i, x)| check_no_floats(x, &format!("{path}[{i}]"))),
        Json::Object(m) => m
            .iter()
            .try_for_each(|(k, x)| check_no_floats(x, &format!("{path}.{k}"))),
        _ => Ok(()),
    }
}

/// Append an entry under an exclusive lock and return it as written.
pub fn append(
    path: &Path,
    side: LedgerSide,
    kind: Kind,
    new: NewEntry,
) -> Result<Entry, LedgerError> {
    if !kind.allowed_in(side) {
        return Err(LedgerError::KindNotAllowed { kind, side });
    }
    for (k, v) in &new.summary {
        check_no_floats(v, &format!("summary.{k}"))?;
    }
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)?;
    file.lock()?;
    let result = append_locked(&mut file, side, kind, new);
    // The entry is on disk once append_locked returns; closing the file releases the
    // lock even if an explicit unlock fails, so that failure must not fail the append.
    let _ = file.unlock();
    result
}

fn append_locked(
    file: &mut File,
    side: LedgerSide,
    kind: Kind,
    new: NewEntry,
) -> Result<Entry, LedgerError> {
    let last = last_entry(file)?;
    let mut entry = Entry {
        seq: last.as_ref().map_or(0, |e| e.seq + 1),
        ts: new.ts,
        actor: new.actor,
        kind,
        ledger: side,
        inputs: new.inputs,
        outputs: new.outputs,
        summary: new.summary,
        tool: new.tool,
        prev: last.map_or_else(|| GENESIS.to_owned(), |e| e.hash),
        hash: String::new(),
    };
    entry.hash = entry_hash(&entry)?;
    let mut line = serde_json::to_vec(&entry).map_err(CanonicalError::from)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.sync_data()?;
    Ok(entry)
}

/// The last entry, read from the end of the file. A ledger whose last line has no
/// newline was torn by an interrupted write or edited by hand; appending to it would
/// glue two entries together, so it is refused.
fn last_entry(file: &mut File) -> Result<Option<Entry>, LedgerError> {
    const CHUNK: u64 = 64 * 1024;
    let len = file.seek(SeekFrom::End(0))?;
    if len == 0 {
        return Ok(None);
    }
    let mut tail = Vec::new();
    let mut start = len;
    loop {
        let from = start.saturating_sub(CHUNK);
        let mut buf = vec![0; (start - from) as usize];
        file.seek(SeekFrom::Start(from))?;
        file.read_exact(&mut buf)?;
        buf.extend_from_slice(&tail);
        tail = buf;
        start = from;
        if tail.last() != Some(&b'\n') {
            return Err(LedgerError::Parse {
                line: 0,
                message: "the last line is incomplete (interrupted write?); repair the file before appending".into(),
            });
        }
        let body = &tail[..tail.len() - 1];
        let trimmed = body
            .iter()
            .rposition(|b| !b.is_ascii_whitespace())
            .map_or(&body[..0], |e| &body[..=e]);
        if let Some(nl) = trimmed.iter().rposition(|b| *b == b'\n') {
            return parse_bytes(&trimmed[nl + 1..]).map(Some);
        }
        if start == 0 {
            return if trimmed.is_empty() {
                Ok(None)
            } else {
                parse_bytes(trimmed).map(Some)
            };
        }
    }
}

fn parse_bytes(line: &[u8]) -> Result<Entry, LedgerError> {
    serde_json::from_slice(line).map_err(|e| LedgerError::Parse {
        line: 0,
        message: format!("last line: {e}"),
    })
}

fn parse_line(line: &str, n: usize) -> Result<Entry, LedgerError> {
    serde_json::from_str(line).map_err(|e| LedgerError::Parse {
        line: n,
        message: e.to_string(),
    })
}

/// Read every entry of a ledger file; a missing file is an empty ledger.
pub fn read(path: &Path) -> Result<Vec<Entry>, LedgerError> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if !line.trim().is_empty() {
            out.push(parse_line(&line, i + 1)?);
        }
    }
    Ok(out)
}

/// Check the chain. An empty result means every line is intact and linked. A
/// truncated tail or a rebuilt chain is only detectable against an external anchor
/// (the head hash recorded elsewhere).
pub fn verify(text: &str, side: Option<LedgerSide>) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut prev = GENESIS.to_owned();
    for (i, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let n = i + 1;
        let entry = match parse_line(line, n) {
            Ok(e) => e,
            Err(e) => {
                problems.push(Problem::Unparsable {
                    line: n,
                    message: e.to_string(),
                });
                prev = String::new();
                continue;
            }
        };
        if entry.seq != i as u64 {
            problems.push(Problem::Seq {
                line: n,
                found: entry.seq,
            });
        }
        if entry.prev != prev {
            problems.push(Problem::Prev { line: n });
        }
        if entry_hash(&entry).ok().as_deref() != Some(entry.hash.as_str()) {
            problems.push(Problem::Hash { line: n });
        }
        if let Some(side) = side
            && (entry.ledger != side || !entry.kind.allowed_in(side))
        {
            problems.push(Problem::Side { line: n });
        }
        prev = entry.hash;
    }
    problems
}

/// The head hash to anchor externally; `genesis` for an empty ledger.
pub fn head(entries: &[Entry]) -> &str {
    entries.last().map_or(GENESIS, |e| e.hash.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(actor: &str) -> NewEntry {
        NewEntry {
            ts: "2026-11-03T00:00:00Z".into(),
            actor: actor.into(),
            tool: "evalsig 0.1.0".into(),
            ..NewEntry::default()
        }
    }

    #[test]
    fn appends_chain_and_verify() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let a = append(&path, LedgerSide::Agent, Kind::Import, new("user:a")).unwrap();
        let b = append(&path, LedgerSide::Agent, Kind::Report, new("user:a")).unwrap();
        assert_eq!(a.seq, 0);
        assert_eq!(a.prev, GENESIS);
        assert_eq!(b.seq, 1);
        assert_eq!(b.prev, a.hash);
        assert!(a.hash.starts_with("blake3:"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(verify(&text, Some(LedgerSide::Agent)).is_empty());
        assert_eq!(head(&read(&path).unwrap()), b.hash);
    }

    #[test]
    fn kinds_are_restricted_per_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        for (kind, side) in [
            (Kind::Gate, LedgerSide::Agent),
            (Kind::Seal, LedgerSide::Agent),
            (Kind::HoldoutAccess, LedgerSide::Agent),
            (Kind::Import, LedgerSide::Referee),
            (Kind::PreregisterDraft, LedgerSide::Referee),
        ] {
            assert!(matches!(
                append(&path, side, kind, new("x")),
                Err(LedgerError::KindNotAllowed { .. })
            ));
        }
        for kind in Kind::ALL {
            assert!(
                kind.allowed_in(LedgerSide::Agent) || kind.allowed_in(LedgerSide::Referee),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn floats_are_refused_in_summaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let mut e = new("x");
        e.summary
            .insert("auc".into(), serde_json::json!({"value": 0.81}));
        assert!(matches!(
            append(&path, LedgerSide::Agent, Kind::Report, e),
            Err(LedgerError::FloatInSummary(p)) if p == "summary.auc.value"
        ));
        let mut ok = new("x");
        ok.summary.insert("auc".into(), serde_json::json!("0.81"));
        ok.summary.insert("items".into(), serde_json::json!(1200));
        assert!(append(&path, LedgerSide::Agent, Kind::Report, ok).is_ok());
    }

    #[test]
    fn a_torn_last_line_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        append(&path, LedgerSide::Agent, Kind::Import, new("x")).unwrap();
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("{\"seq\":1,\"ts\"");
        std::fs::write(&path, &text).unwrap();
        assert!(matches!(
            append(&path, LedgerSide::Agent, Kind::Report, new("x")),
            Err(LedgerError::Parse { .. })
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[test]
    fn long_ledgers_append_from_the_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let mut big = new("x");
        big.summary
            .insert("note".into(), serde_json::json!("y".repeat(70_000)));
        append(&path, LedgerSide::Agent, Kind::Import, big).unwrap();
        for _ in 0..3 {
            append(&path, LedgerSide::Agent, Kind::Report, new("x")).unwrap();
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(verify(&text, Some(LedgerSide::Agent)).is_empty());
        assert_eq!(read(&path).unwrap().len(), 4);
    }

    #[test]
    fn concurrent_appends_from_two_processes_form_one_chain() {
        const PER_PROCESS: usize = 40;
        if let Ok(path) = std::env::var("EVALSIG_LEDGER_CHILD") {
            for _ in 0..PER_PROCESS {
                append(
                    Path::new(&path),
                    LedgerSide::Agent,
                    Kind::Report,
                    new("child"),
                )
                .unwrap();
            }
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.jsonl");
        let exe = std::env::current_exe().unwrap();
        let children: Vec<_> = (0..2)
            .map(|_| {
                std::process::Command::new(&exe)
                    .args([
                        "--exact",
                        "ledger::tests::concurrent_appends_from_two_processes_form_one_chain",
                    ])
                    .env("EVALSIG_LEDGER_CHILD", &path)
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for mut c in children {
            assert!(c.wait().unwrap().success());
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2 * PER_PROCESS);
        assert_eq!(verify(&text, Some(LedgerSide::Agent)), []);
    }
}
