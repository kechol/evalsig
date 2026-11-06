//! `evalsig migrate`: rewrite snapshots in an earlier canonical record format as
//! new snapshots in the current one, keeping the originals.

use std::path::Path;

use evalsig_store::ledger::Kind;
use evalsig_store::migrate::{self, MIGRATIONS, Migration};
use evalsig_store::snapshot::{self, Replacement};
use serde_json::json;

use evalsig_project::Error;
use evalsig_project::data;
use evalsig_project::envelope::{Envelope, Output, Schema};
use evalsig_project::exit::Exit;
use evalsig_project::project::Project;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// List the snapshots that would be migrated without writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(root: &Path, args: &Args) -> Result<Output, Error> {
    migrate_with(root, args, MIGRATIONS)
}

fn migrate_with(root: &Path, args: &Args, migrations: &[Migration]) -> Result<Output, Error> {
    let project = Project::open(root)?;
    let store = project.objects();
    let mut rows = Vec::new();
    let mut replaced = Vec::new();
    for x in data::in_force(&project)? {
        let old = x.snapshot;
        let lines = migrate::read_lines(&store, &old).map_err(Error::store)?;
        let (mut records, path) = migrate::upgrade(lines, migrations).map_err(|e| {
            Error::new(
                "unknown_format",
                format!("snapshot {old}: {e}"),
                Exit::Rejected,
            )
        })?;
        if path.len() == 1 {
            continue;
        }
        let mut row = json!({
            "from": old,
            "from_version": path[0],
            "to_version": path[path.len() - 1],
        });
        if !args.dry_run {
            let prov = data::provenance_of(&store, &x)?;
            let r =
                snapshot::replace(&store, old, &mut records, prov, |_| {}).map_err(Error::store)?;
            row["to"] = json!(r.to);
            row["provenance"] = json!(r.provenance);
            replaced.push(r);
        }
        rows.push(row);
    }
    let mut envelope = Envelope::new(
        Schema::Migrate,
        json!({ "dry_run": args.dry_run, "migrated": rows }),
    );
    if !replaced.is_empty() {
        project.record(
            &mut envelope,
            Kind::Migrate,
            replaced.iter().map(|r| r.from.to_string()).collect(),
            Replacement::outputs(&replaced),
            json!({ "replaced": rows }),
        )?;
    }
    Ok(Output::success(envelope))
}

#[cfg(test)]
mod tests {
    use evalsig_core::records::RECORDS_VERSION;
    use evalsig_store::canonical::to_canonical_bytes;
    use evalsig_store::ledger::{self, LedgerSide, NewEntry};
    use evalsig_store::snapshot::{PROVENANCE_SCHEMA, Provenance};
    use serde_json::Value;

    use super::*;

    /// A fictional `records/0` that named the record kind `type`.
    const RECORDS_0: Migration = Migration {
        from: "records/0",
        to: RECORDS_VERSION,
        detect: |l| l.get("type").is_some() && l.get("record").is_none(),
        apply: |mut l| {
            let m = l.as_object_mut().ok_or("not an object")?;
            let kind = m.remove("type").ok_or("no type")?;
            m.insert("record".into(), kind);
            Ok(l)
        },
    };

    /// A project whose only snapshot is in `records/0`, as an old import left it.
    fn old_project() -> (tempfile::TempDir, evalsig_store::hash::ContentHash) {
        let dir = tempfile::tempdir().unwrap();
        crate::commands::init::run(dir.path(), false).unwrap();
        let project = Project::open(dir.path()).unwrap();
        let store = project.objects();
        let lines: Vec<Vec<u8>> = [
            json!({"type": "run", "run_id": "r", "system": {"id": "s", "version": "1"}, "output_type": "binary"}),
            json!({"type": "item", "item_id": "a"}),
            json!({"type": "judgment", "item_id": "a", "run_id": "r", "gen_replicate": 0, "output_type": "binary", "value": true}),
        ]
        .iter()
        .map(|v| to_canonical_bytes(v).unwrap())
        .collect();
        let (snap, _) = store.put(&lines.join(&b'\n')).unwrap();
        let prov = Provenance {
            schema: PROVENANCE_SCHEMA.into(),
            snapshot: snap,
            sources: vec![],
            adapter: "records".into(),
            adapter_version: None,
            mapping: None,
            mapping_decisions: vec![],
            tool: "evalsig 0.0.0".into(),
            imported_at: "2026-11-03T00:00:00Z".into(),
            actor: "user:test".into(),
            redactions: vec![],
        };
        let (prov, _) = store.put(&to_canonical_bytes(&prov).unwrap()).unwrap();
        ledger::append(
            &project.ledger_path(),
            LedgerSide::Agent,
            Kind::Import,
            NewEntry {
                ts: "2026-11-03T00:00:00Z".into(),
                actor: "user:test".into(),
                inputs: vec![],
                outputs: vec![snap.to_string(), prov.to_string()],
                summary: serde_json::Map::new(),
                tool: "evalsig 0.0.0".into(),
            },
        )
        .unwrap();
        (dir, snap)
    }

    #[test]
    fn a_records_0_snapshot_is_migrated_once() {
        let (dir, old) = old_project();
        let project = Project::open(dir.path()).unwrap();
        assert!(data::load(&project).is_err());
        assert!(run(dir.path(), &Args { dry_run: false }).is_err());

        let dry = migrate_with(dir.path(), &Args { dry_run: true }, &[RECORDS_0]).unwrap();
        assert_eq!(
            dry.envelope.results["migrated"][0]["from_version"],
            "records/0"
        );
        assert_eq!(ledger::read(&project.ledger_path()).unwrap().len(), 1);

        let out = migrate_with(dir.path(), &Args { dry_run: false }, &[RECORDS_0]).unwrap();
        let row = &out.envelope.results["migrated"][0];
        assert_eq!(row["to_version"], RECORDS_VERSION);
        let entries = ledger::read(&project.ledger_path()).unwrap();
        let last = entries.last().unwrap();
        assert_eq!(last.kind, Kind::Migrate);
        assert_eq!(last.inputs, [old.to_string()]);
        assert_eq!(
            Value::Array(vec![last.summary["replaced"][0].clone()]),
            out.envelope.results["migrated"]
        );
        assert!(
            ledger::verify(
                &std::fs::read_to_string(project.ledger_path()).unwrap(),
                Some(LedgerSide::Agent)
            )
            .is_empty()
        );

        let d = data::load(&project).unwrap();
        assert_eq!(d.records.len(), 3);
        assert_eq!(d.snapshots[0].to_string(), row["to"]);
        assert!(project.objects().contains(&old));

        let again = migrate_with(dir.path(), &Args { dry_run: false }, &[RECORDS_0]).unwrap();
        assert_eq!(again.envelope.results["migrated"], json!([]));
        assert_eq!(ledger::read(&project.ledger_path()).unwrap().len(), 2);
    }
}
