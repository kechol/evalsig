//! `evalsig split` (dev / validation, the agent's authority) and `evalsig splits`
//! (SPEC §8.1, D-022).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use evalsig_core::records::Item;
use evalsig_core::split::{self, Assignment, Method, Part};
use evalsig_store::canonical::to_canonical_bytes;
use evalsig_store::hash::ContentHash;
use evalsig_store::ledger::Kind;
use serde_json::{Value, json};

use evalsig_project::Error;
use evalsig_project::data;
use evalsig_project::envelope::{Envelope, Output, Schema};
use evalsig_project::project::{Project, hex, io_error};
use evalsig_project::provenance::item_set_hash;
use evalsig_project::split::{split_entries, split_error};
use evalsig_referee::holdout;

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
    /// Report the parts the split would record without writing anything.
    #[arg(long, conflicts_with = "holdout")]
    pub dry_run: bool,
}

fn read_json(path: &Path) -> Result<Value, Error> {
    let text = std::fs::read_to_string(path).map_err(|e| io_error(path, &e))?;
    serde_json::from_str(&text).map_err(|e| Error::usage(format!("{}: {e}", path.display())))
}

/// The items and holdout hash of a remaining.json (`evalsig.remaining/1`).
fn remaining(path: &Path) -> Result<(BTreeSet<String>, Option<String>), Error> {
    let v = read_json(path)?;
    if v["schema"] != Schema::Remaining.name() {
        return Err(Error::usage(format!(
            "{}: not an evalsig.remaining/1 file",
            path.display()
        )));
    }
    let items = v["items"]
        .as_array()
        .ok_or_else(|| Error::usage(format!("{}: needs an items array", path.display())))?
        .iter()
        .map(|i| {
            i.as_str()
                .map(str::to_owned)
                .ok_or_else(|| Error::usage(format!("{}: item IDs are strings", path.display())))
        })
        .collect::<Result<_, _>>()?;
    Ok((items, v["holdout"].as_str().map(str::to_owned)))
}

fn assignment_file(path: &Path) -> Result<BTreeMap<String, Part>, Error> {
    let v = read_json(path)?;
    let obj = v
        .as_object()
        .ok_or_else(|| Error::usage(format!("{}: needs an object", path.display())))?;
    obj.iter()
        .map(|(id, side)| {
            let part = match side.as_str() {
                Some("dev") => Part::Dev,
                Some("validation") => Part::Validation,
                _ => {
                    return Err(Error::usage(format!(
                        "{}: item {id:?} must be \"dev\" or \"validation\"",
                        path.display()
                    )));
                }
            };
            Ok((id.clone(), part))
        })
        .collect()
}

pub fn run(root: &Path, args: &Args) -> Result<Output, Error> {
    if args.holdout {
        return holdout::cut(
            root,
            &holdout::Request {
                method: args.method.map(|m| match m {
                    MethodArg::Time => holdout::Method::Time,
                    MethodArg::Random => holdout::Method::Random,
                }),
                share: args.share,
                stratum: args.stratum.clone(),
                seed: args.seed,
                out: args.out.clone().expect("clap requires --out"),
                referee: args.referee.clone(),
            },
        );
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
        return Err(Error::usage_code(
            "conflicting_arguments",
            "--stratum and --seed apply to random splits",
        ));
    }
    let seed = random.then_some(seed);
    let a: Assignment =
        split::split(&items, within.as_ref(), &method, args.validation).map_err(split_error)?;
    let part = |ids: &[String]| -> Result<Value, Error> {
        Ok(json!({ "hash": item_set_hash(ids.iter())?, "items": ids.len() }))
    };
    let summary = json!({
        "method": name,
        "seed": seed,
        "dev": part(&a.dev)?,
        "validation": part(&a.validation)?,
        "holdout": holdout,
    });
    if args.dry_run {
        let mut results = summary;
        results["dry_run"] = json!(true);
        results["holdout_excluded"] = json!(within.is_some());
        return Ok(Output::success(Envelope::new(Schema::Split, results)));
    }
    // The file is named by both parts, so two splits sharing one part stay apart.
    let name_hash = ContentHash::blake3(
        &to_canonical_bytes(&json!({"dev": a.dev, "validation": a.validation}))
            .map_err(Error::store)?,
    )
    .to_string();
    let dir = project.splits_dir();
    std::fs::create_dir_all(&dir).map_err(|e| io_error(&dir, &e))?;
    let file_name = format!("{}.json", hex(&name_hash));
    let path = dir.join(&file_name);
    let file = json!({
        "schema": Schema::Split.name(),
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
    let mut envelope = Envelope::new(Schema::Split, results);
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

pub fn list(root: &Path) -> Result<Output, Error> {
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
        Schema::Splits,
        json!({ "count": splits.len(), "splits": splits }),
    )))
}
