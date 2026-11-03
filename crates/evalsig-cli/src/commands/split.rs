//! `evalsig split` (dev / validation, the agent's authority) and `evalsig splits`
//! (SPEC §8.1, D-022).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use evalsig_core::records::Item;
use evalsig_core::split::{self, Assignment, Method, Part, SplitError};
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::{self, Entry, Kind};
use serde_json::{Value, json};

use crate::data;
use crate::error::CliError;
use crate::exit::Exit;
use crate::output::{Envelope, Output};
use crate::project::{Project, io_error};
use crate::provenance::item_set_hash;
use crate::referee::Referee;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum MethodArg {
    /// The latest clusters by occurred_at go to validation.
    Time,
    /// Clusters at random (seeded), within each value of --stratum.
    Random,
}

#[derive(Debug, Clone, clap::Args)]
pub struct Args {
    /// How to assign clusters (default: time when every item has occurred_at, else
    /// random). --assign gives the assignment instead.
    #[arg(long, value_enum, conflicts_with = "assign")]
    pub method: Option<MethodArg>,
    /// Share of the items, counted by whole clusters, that goes to validation.
    #[arg(long, default_value_t = 0.3, value_name = "SHARE")]
    pub validation: f64,
    /// For random splits: keep this item stratum's shares on both sides.
    #[arg(long, value_name = "STRATUM")]
    pub stratum: Option<String>,
    /// Seed of a random split (default: the configured seed).
    #[arg(long)]
    pub seed: Option<u64>,
    /// A JSON object mapping each item ID to "dev" or "validation".
    #[arg(long, value_name = "FILE")]
    pub assign: Option<PathBuf>,
    /// The referee's remaining.json: split only these items, leaving the sealed
    /// holdout out.
    #[arg(long, value_name = "FILE")]
    pub within: Option<PathBuf>,
    /// As the referee: cut the holdout from all items and write the remaining item
    /// IDs, without labels, for the agent.
    #[arg(long, conflicts_with_all = ["within", "assign"], requires = "out")]
    pub holdout: bool,
    /// With --holdout: the share of the items, by whole clusters, held out.
    #[arg(long, default_value_t = 0.2, value_name = "SHARE")]
    pub share: f64,
    /// With --holdout: where to write remaining.json for the agent.
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
    /// The referee's home (default: EVALSIG_REFEREE).
    #[arg(long, value_name = "DIR")]
    pub referee: Option<PathBuf>,
}

fn rejected(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::new(code, message, Exit::Rejected)
}

fn read_json(path: &Path) -> Result<Value, CliError> {
    let text = std::fs::read_to_string(path).map_err(|e| io_error(path, &e))?;
    serde_json::from_str(&text).map_err(|e| CliError::usage(format!("{}: {e}", path.display())))
}

/// The items and holdout hash of a remaining.json (`evalsig.remaining/1`).
fn remaining(path: &Path) -> Result<(BTreeSet<String>, Option<String>), CliError> {
    let v = read_json(path)?;
    if v["schema"] != "evalsig.remaining/1" {
        return Err(CliError::usage(format!(
            "{}: not an evalsig.remaining/1 file",
            path.display()
        )));
    }
    let items = v["items"]
        .as_array()
        .ok_or_else(|| CliError::usage(format!("{}: needs an items array", path.display())))?
        .iter()
        .map(|i| {
            i.as_str()
                .map(str::to_owned)
                .ok_or_else(|| CliError::usage(format!("{}: item IDs are strings", path.display())))
        })
        .collect::<Result<_, _>>()?;
    Ok((items, v["holdout"].as_str().map(str::to_owned)))
}

fn assignment_file(path: &Path) -> Result<BTreeMap<String, Part>, CliError> {
    let v = read_json(path)?;
    let obj = v
        .as_object()
        .ok_or_else(|| CliError::usage(format!("{}: needs an object", path.display())))?;
    obj.iter()
        .map(|(id, side)| {
            let part = match side.as_str() {
                Some("dev") => Part::Dev,
                Some("validation") => Part::Validation,
                _ => {
                    return Err(CliError::usage(format!(
                        "{}: item {id:?} must be \"dev\" or \"validation\"",
                        path.display()
                    )));
                }
            };
            Ok((id.clone(), part))
        })
        .collect()
}

pub fn run(root: &Path, args: &Args) -> Result<Output, CliError> {
    if args.holdout {
        return holdout(root, args);
    }
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let items: Vec<&Item> = d.items().into_values().collect();
    let (within, holdout) = match &args.within {
        Some(p) => {
            let (ids, h) = remaining(p)?;
            (Some(ids), h)
        }
        None => (None, None),
    };
    let use_time = match args.method {
        Some(MethodArg::Time) => true,
        Some(MethodArg::Random) => false,
        // The items the split may use decide; sealed holdout items do not.
        None => items
            .iter()
            .filter(|i| within.as_ref().is_none_or(|w| w.contains(&i.item_id)))
            .all(|i| i.occurred_at.is_some()),
    };
    let seed = args.seed.or(config.stats.seed).unwrap_or(0);
    let (method, name) = match &args.assign {
        Some(p) => (Method::Assign(assignment_file(p)?), "assign"),
        None if use_time => (Method::Time, "time"),
        None => (
            Method::Random {
                seed,
                stratum: args.stratum.clone(),
            },
            "random",
        ),
    };
    let random = name == "random";
    if !random && (args.stratum.is_some() || args.seed.is_some()) {
        return Err(CliError::usage(
            "--stratum and --seed apply to random splits",
        ));
    }
    let seed = random.then_some(seed);
    let a: Assignment =
        split::split(&items, within.as_ref(), &method, args.validation).map_err(|e| match e {
            SplitError::CrossesCluster { .. } => rejected("split_crosses_cluster", e.to_string()),
            SplitError::NotAvailable(_) => rejected("holdout_item", e.to_string()),
            SplitError::Share => CliError::usage(e.to_string()),
            _ => rejected("split", e.to_string()),
        })?;
    let part = |ids: &[String]| -> Result<Value, CliError> {
        Ok(json!({ "hash": item_set_hash(ids.iter())?, "items": ids.len() }))
    };
    let summary = json!({
        "method": name,
        "seed": seed,
        "dev": part(&a.dev)?,
        "validation": part(&a.validation)?,
        "holdout": holdout,
    });
    // The file is named by both parts, so two splits sharing one part stay apart.
    let name_hash = ContentHash::blake3(
        &to_canonical_bytes(&json!({"dev": a.dev, "validation": a.validation}))
            .map_err(CliError::store)?,
    )
    .to_string();
    let dir = project.splits_dir();
    std::fs::create_dir_all(&dir).map_err(|e| io_error(&dir, &e))?;
    let file_name = format!(
        "{}.json",
        name_hash.strip_prefix("blake3:").unwrap_or(&name_hash)
    );
    let path = dir.join(&file_name);
    let file = json!({
        "schema": "evalsig.split/1",
        "method": name,
        "seed": seed,
        "stratum": random.then_some(&args.stratum),
        "validation_share": args.validation,
        "holdout": holdout,
        "dev": a.dev,
        "validation": a.validation,
    });
    let mut text = serde_json::to_string_pretty(&file).expect("splits serialize");
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| io_error(&path, &e))?;
    let mut results = summary.clone();
    results["holdout_excluded"] = json!(within.is_some());
    results["file"] = json!(format!(".evalsig/splits/{file_name}"));
    let mut envelope = Envelope::new("evalsig.split/1", results);
    let snapshots = d.snapshot_ids();
    envelope.provenance.snapshots = snapshots.clone();
    project.record(
        &mut envelope,
        Kind::Split,
        snapshots,
        vec![name_hash],
        summary,
    )?;
    Ok(Output::success(envelope))
}

/// `split --holdout`, the referee's carve-out (SPEC §8.1, D-022): whole clusters to
/// the holdout, recorded in the referee ledger by the holdout's item-set hash, which
/// names the holdout and its budget; the agent gets the other item IDs only.
fn holdout(root: &Path, args: &Args) -> Result<Output, CliError> {
    let referee = Referee::open(args.referee.as_deref())?;
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let items: Vec<&Item> = d.items().into_values().collect();
    let use_time = match args.method {
        Some(MethodArg::Time) => true,
        Some(MethodArg::Random) => false,
        None => items.iter().all(|i| i.occurred_at.is_some()),
    };
    let seed = args.seed.or(config.stats.seed).unwrap_or(0);
    let (method, name) = if use_time {
        (Method::Time, "time")
    } else {
        (
            Method::Random {
                seed,
                stratum: args.stratum.clone(),
            },
            "random",
        )
    };
    let a = split::split(&items, None, &method, args.share)
        .map_err(|e| rejected("split", e.to_string()))?;
    let (remaining, held) = (a.dev, a.validation);
    let hash = item_set_hash(held.iter())?;
    let file = referee.path("holdouts").join(format!(
        "{}.json",
        hash.strip_prefix("blake3:").unwrap_or(&hash)
    ));
    let mut text = serde_json::to_string_pretty(&json!({
        "schema": "evalsig.holdout/1",
        "holdout": hash,
        "method": name,
        "seed": (name == "random").then_some(seed),
        "share": args.share,
        "items": held,
    }))
    .expect("holdouts serialize");
    text.push('\n');
    std::fs::write(&file, text).map_err(|e| io_error(&file, &e))?;
    let out = args.out.as_ref().expect("clap requires --out");
    let mut text = serde_json::to_string_pretty(&json!({
        "schema": "evalsig.remaining/1",
        "holdout": hash,
        "items": remaining,
    }))
    .expect("item IDs serialize");
    text.push('\n');
    std::fs::write(out, text).map_err(|e| io_error(out, &e))?;
    let summary = json!({
        "holdout": { "hash": hash, "items": held.len() },
        "remaining": { "hash": item_set_hash(remaining.iter())?, "items": remaining.len() },
        "method": name,
        "seed": (name == "random").then_some(seed),
        // Ledger summaries hold no floating-point numbers (D-007).
        "share": args.share.to_string(),
    });
    let snapshots = d.snapshot_ids();
    referee.record(
        Kind::Split,
        snapshots.clone(),
        vec![hash.clone()],
        summary.clone(),
    )?;
    let mut envelope = Envelope::new("evalsig.holdout/1", summary);
    envelope.provenance.snapshots = snapshots;
    envelope.provenance.ledger = Some("referee".into());
    Ok(Output::success(envelope))
}

/// The recorded agent-side splits, oldest first.
pub fn split_entries(project: &Project) -> Result<Vec<Entry>, CliError> {
    Ok(ledger::read(&project.ledger_path())
        .map_err(CliError::store)?
        .into_iter()
        .filter(|e| e.kind == Kind::Split)
        .collect())
}

/// A part of a recorded split: its hash and items.
#[derive(Debug, Clone)]
pub struct Scope {
    pub hash: String,
    pub items: BTreeSet<String>,
}

/// One part of the latest recorded split, or of the split whose part has `hash`.
pub fn scope(project: &Project, part: Part, hash: Option<&str>) -> Result<Scope, CliError> {
    let key = match part {
        Part::Dev => "dev",
        Part::Validation => "validation",
    };
    let part_hash = |e: &Entry| {
        e.summary
            .get(key)
            .and_then(|p| p["hash"].as_str())
            .map(str::to_owned)
    };
    let entry = split_entries(project)?
        .into_iter()
        .rev()
        .find(|e| hash.is_none_or(|h| part_hash(e).as_deref() == Some(h)))
        .ok_or_else(|| {
            CliError::usage(match hash {
                Some(h) => format!("no recorded split has a {key} part {h}"),
                None => "no split is recorded".to_owned(),
            })
            .with_hint("run `evalsig split` first")
        })?;
    let name = entry.outputs.first().cloned().unwrap_or_default();
    let path = project.splits_dir().join(format!(
        "{}.json",
        name.strip_prefix("blake3:").unwrap_or(&name)
    ));
    let items = read_json(&path)?[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| i.as_str().map(str::to_owned))
        .collect();
    Ok(Scope {
        hash: part_hash(&entry).unwrap_or_default(),
        items,
    })
}

pub fn list(root: &Path) -> Result<Output, CliError> {
    let project = Project::open(root)?;
    let splits: Vec<Value> = split_entries(&project)?
        .into_iter()
        .map(|e| {
            json!({
                "seq": e.seq,
                "ts": e.ts,
                "method": e.summary.get("method"),
                "seed": e.summary.get("seed"),
                "dev": e.summary.get("dev"),
                "validation": e.summary.get("validation"),
                "holdout": e.summary.get("holdout"),
            })
        })
        .collect();
    Ok(Output::success(Envelope::new(
        "evalsig.splits/1",
        json!({ "count": splits.len(), "splits": splits }),
    )))
}
