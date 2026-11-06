//! `exclude --referee`: the referee leaves holdout items out in its own home and
//! ledger (D-027). From then on the holdout's items, for `seal`, `preregister approve`
//! and `gate`, leave those items out.

use std::path::Path;

use evalsig_project::Error;
use evalsig_project::envelope::{Envelope, Output, Schema, Warning};
use evalsig_project::project::io_error;
use evalsig_project::provenance::item_set;
use evalsig_store::ledger::Kind;
use serde::Serialize;
use serde_json::json;

use crate::home::Referee;

pub fn exclude(
    home: Option<&Path>,
    items: &[String],
    reason: &impl Serialize,
    detail: Option<&str>,
) -> Result<Output, Error> {
    let (referee, _lock) = Referee::open_exclusive(home)?;
    let (items, bytes, set_hash) = item_set(items)?;
    let held = referee.all_holdout_items()?;
    if let Some(other) = items.iter().find(|i| !held.contains(*i)) {
        return Err(
            Error::usage(format!("{other} is in no holdout of this referee"))
                .with_hint("the agent excludes the items it may use, without --referee"),
        );
    }
    let p = referee.file("exclusions", &set_hash, "json");
    std::fs::write(&p, &bytes).map_err(|e| io_error(&p, &e))?;
    let summary = json!({ "reason": reason, "detail": detail, "count": items.len() });
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
