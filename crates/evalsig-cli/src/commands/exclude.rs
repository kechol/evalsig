//! `evalsig exclude`: leave items out of every analysis, with a reason code (D-027).
//! The agent excludes the items it may use by replacing snapshots, like `purge`; the
//! referee excludes holdout items in its own home and ledger.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use evalsig_core::records::Record;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use evalsig_store::snapshot::{self, Replacement};
use serde_json::{Value, json};

use evalsig_project::Error;
use evalsig_project::data;
use evalsig_project::envelope::{Envelope, Output, Schema, Warning};
use evalsig_project::exit::Exit;
use evalsig_project::project::{Project, hex, io_error, str_set};
use evalsig_project::provenance;
use evalsig_referee::home::Referee;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, serde::Serialize)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub enum Reason {
    Duplicate,
    InvalidInput,
    OutOfScope,
    Privacy,
    Other,
}

#[derive(Debug, clap::Args)]
pub struct Args {
    /// The items to exclude.
    #[arg(required = true, value_name = "ITEM")]
    pub items: Vec<String>,
    #[arg(long, value_enum)]
    pub reason: Reason,
    /// Free text kept in the ledger; required with `--reason other`.
    #[arg(long)]
    pub detail: Option<String>,
    /// As the referee: exclude holdout items in this referee home (default:
    /// EVALSIG_REFEREE when set).
    #[arg(long, value_name = "DIR")]
    pub referee: Option<PathBuf>,
}

fn item_of(r: &Record) -> Option<&str> {
    match r {
        Record::Item(x) => Some(&x.item_id),
        Record::Label(x) => Some(&x.item_id),
        Record::Judgment(x) => Some(&x.item_id),
        Record::Preference(x) => Some(&x.item_id),
        Record::Run(_) | Record::Content(_) => None,
    }
}

/// The items and their set hash, the input of the `exclude` line.
fn item_set(items: &[String]) -> Result<(BTreeSet<String>, Vec<u8>, String), Error> {
    let set: BTreeSet<String> = items.iter().cloned().collect();
    let bytes = provenance::item_set_bytes(set.iter())?;
    let hash = ContentHash::blake3(&bytes).to_string();
    Ok((set, bytes, hash))
}

pub fn run(root: &Path, args: &Args) -> Result<Output, Error> {
    if args.reason == Reason::Other && args.detail.as_deref().is_none_or(|d| d.trim().is_empty()) {
        return Err(Error::usage("--reason other needs --detail"));
    }
    let referee_home = args.referee.is_some()
        || std::env::var_os(evalsig_referee::home::ENV).is_some_and(|v| !v.is_empty());
    if referee_home {
        referee_side(args)
    } else {
        agent_side(root, args)
    }
}

/// The items the agent may use, when a split was made within the referee's
/// remaining items: the parts of the latest such split.
fn remaining(project: &Project) -> Result<Option<BTreeSet<String>>, Error> {
    let entries = project.entries()?;
    for e in entries.iter().rev().filter(|e| e.kind == Kind::Split) {
        if e.summary.get("holdout").is_none_or(Value::is_null) {
            continue;
        }
        let Some(name) = e.outputs.first() else {
            continue;
        };
        let p = project.splits_dir().join(format!("{}.json", hex(name)));
        let text = std::fs::read_to_string(&p).map_err(|e| io_error(&p, &e))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| Error::invalid_input(&p, e))?;
        let mut ids = str_set(&v["dev"]);
        ids.extend(str_set(&v["validation"]));
        return Ok(Some(ids));
    }
    Ok(None)
}

fn agent_side(root: &Path, args: &Args) -> Result<Output, Error> {
    let project = Project::open(root)?;
    let (items, bytes, set_hash) = item_set(&args.items)?;
    if let Some(mine) = remaining(&project)?
        && let Some(held) = items.iter().find(|i| !mine.contains(*i))
    {
        return Err(Error::new(
            "holdout_item",
            format!(
                "{held} is not among the referee's remaining items; only the referee can exclude holdout items"
            ),
            Exit::Rejected,
        ));
    }
    let store = project.objects();
    let mut sets = Vec::new();
    for x in data::in_force(&project)? {
        let records = snapshot::read(&store, &x.snapshot).map_err(Error::store)?;
        sets.push((x, records));
    }
    let known: BTreeSet<&str> = sets
        .iter()
        .flat_map(|(_, rs)| rs.iter().filter_map(item_of))
        .collect();
    if let Some(unknown) = items.iter().find(|i| !known.contains(i.as_str())) {
        return Err(Error::usage(format!("no item {unknown:?}"))
            .with_hint("list items with `evalsig items`"));
    }
    store.put(&bytes).map_err(Error::store)?;
    let mut replaced = Vec::new();
    let mut removed = 0;
    for (x, mut records) in sets {
        let before = records.len();
        records.retain(|r| item_of(r).is_none_or(|i| !items.contains(i)));
        if records.len() == before {
            continue;
        }
        removed += before - records.len();
        let prov = data::provenance_of(&store, &x)?;
        let r = snapshot::replace(&store, x.snapshot, &mut records, prov, |_| {})
            .map_err(Error::store)?;
        replaced.push(r);
    }
    let summary = json!({
        "reason": args.reason,
        "detail": args.detail,
        "count": items.len(),
        "replaced": replaced,
    });
    let mut results = summary.clone();
    results["items"] = json!(items);
    results["records_removed"] = json!(removed);
    let mut envelope = Envelope::new(Schema::Exclude, results);
    envelope.warnings.push(Warning {
        code: "reimport_restores".into(),
        message: "records of these items imported later come back in; exclude them again after such an import".into(),
    });
    project.record(
        &mut envelope,
        Kind::Exclude,
        std::iter::once(set_hash)
            .chain(replaced.iter().map(|r| r.from.to_string()))
            .collect(),
        Replacement::outputs(&replaced),
        summary,
    )?;
    Ok(Output::success(envelope))
}

fn referee_side(args: &Args) -> Result<Output, Error> {
    let (referee, _lock) = Referee::open_exclusive(args.referee.as_deref())?;
    let (items, bytes, set_hash) = item_set(&args.items)?;
    let held = referee.all_holdout_items()?;
    if let Some(other) = items.iter().find(|i| !held.contains(*i)) {
        return Err(
            Error::usage(format!("{other} is in no holdout of this referee"))
                .with_hint("the agent excludes the items it may use, without --referee"),
        );
    }
    let p = referee.file("exclusions", &set_hash, "json");
    std::fs::write(&p, &bytes).map_err(|e| io_error(&p, &e))?;
    let summary = json!({ "reason": args.reason, "detail": args.detail, "count": items.len() });
    referee.record(Kind::Exclude, vec![set_hash], vec![], summary.clone())?;
    let mut results = summary;
    results["items"] = json!(items);
    let mut envelope = Envelope::new(Schema::Exclude, results);
    envelope.provenance.ledger = Some("referee".into());
    if referee
        .entries()?
        .iter()
        .any(|e| e.kind == Kind::PreregisterApprove)
    {
        envelope.warnings.push(Warning {
            code: "reapproval_needed".into(),
            message: "approvals pin the holdout's items; gate refuses them, without spending budget, until the conditions are approved again".into(),
        });
    }
    Ok(Output::success(envelope))
}
