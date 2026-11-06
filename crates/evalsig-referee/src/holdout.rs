//! `split --holdout`, the referee's carve-out (SPEC §8.1, D-022): whole clusters to
//! the holdout, recorded in the referee ledger by the holdout's item-set hash, which
//! names the holdout and its budget; the agent gets the other item IDs only.

use std::path::{Path, PathBuf};

use evalsig_core::records::Item;
use evalsig_core::split::{self, Method as SplitMethod};
use evalsig_project::Error;
use evalsig_project::data;
use evalsig_project::envelope::{Envelope, Output, Schema};
use evalsig_project::project::{Project, io_error};
use evalsig_project::provenance::item_set_hash;
use evalsig_project::split::split_error;
use evalsig_store::ledger::Kind;
use serde_json::json;

use crate::home::Referee;

/// How clusters are assigned to the holdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// The latest clusters by occurred_at.
    Time,
    /// Clusters at random (seeded), within each value of the stratum.
    Random,
}

/// A holdout to cut from the referee's project.
#[derive(Debug, Clone)]
pub struct Request {
    /// How to assign clusters; time when every item has occurred_at, else random.
    pub method: Option<Method>,
    /// The share of the items, by whole clusters, held out.
    pub share: f64,
    /// For random cuts: keep this item stratum's shares on both sides.
    pub stratum: Option<String>,
    /// Seed of a random cut (default: the configured seed).
    pub seed: Option<u64>,
    /// Where to write remaining.json for the agent.
    pub out: PathBuf,
    /// The referee's home (default: EVALSIG_REFEREE).
    pub referee: Option<PathBuf>,
}

pub fn cut(root: &Path, args: &Request) -> Result<Output, Error> {
    let (referee, _lock) = Referee::open_exclusive(args.referee.as_deref())?;
    let project = Project::open(root)?;
    let config = project.config()?;
    let d = data::load(&project)?;
    let items: Vec<&Item> = d.items().into_values().collect();
    let use_time = match args.method {
        Some(Method::Time) => true,
        Some(Method::Random) => false,
        None => items.iter().all(|i| i.occurred_at.is_some()),
    };
    let seed = args.seed.or(config.stats.seed).unwrap_or(0);
    let (method, name) = if use_time {
        (SplitMethod::Time, "time")
    } else {
        (
            SplitMethod::Random {
                seed,
                stratum: args.stratum.clone(),
            },
            "random",
        )
    };
    let a = split::split(&items, None, &method, args.share).map_err(split_error)?;
    let (remaining, held) = (a.dev, a.validation);
    let hash = item_set_hash(held.iter())?;
    let file = referee.file("holdouts", &hash, "json");
    let mut text = serde_json::to_string_pretty(&json!({
        "schema": Schema::Holdout.name(),
        "holdout": hash,
        "method": name,
        "seed": (name == "random").then_some(seed),
        "share": args.share,
        "items": held,
    }))
    .expect("holdouts serialize");
    text.push('\n');
    std::fs::write(&file, text).map_err(|e| io_error(&file, &e))?;
    let out = &args.out;
    evalsig_project::project::outside_store(out)?;
    let mut text = serde_json::to_string_pretty(&json!({
        "schema": Schema::Remaining.name(),
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
    let mut envelope = Envelope::new(Schema::Holdout, summary);
    envelope.provenance.snapshots = snapshots;
    envelope.provenance.ledger = Some("referee".into());
    Ok(Output::success(envelope))
}
