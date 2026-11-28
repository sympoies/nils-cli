//! Bounded outcome projection shared by board and glance.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

use super::{Receipt, Verification};
use crate::{CliContext, SessionRecord};

#[derive(Debug, Serialize)]
pub(crate) struct Outcome {
    account: String,
    generation: String,
    state: &'static str,
    provider_verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

impl Outcome {
    pub(crate) fn text(&self) -> String {
        let reason = self
            .reason
            .as_ref()
            .map_or_else(String::new, |reason| format!(" ({reason})"));
        format!(
            "auth recovery: {} generation {} {}{}\n",
            self.account, self.generation, self.state, reason
        )
    }
}

pub(crate) fn for_record(context: &CliContext, record: &SessionRecord) -> Option<Outcome> {
    super::super::projection(context, record)
        .0
        .and_then(|incident| incident.same_account_recovery.as_ref().and_then(project))
}

pub(crate) fn project(receipt: &Receipt) -> Option<Outcome> {
    let publication = receipt.replacement.as_ref()?;
    let verification = receipt.verification?;
    let state = match (verification, receipt.state.as_str()) {
        (Verification::Verified, "runtime_resumed") => "restarted",
        (Verification::Verified, "deferred_until_idle") => "deferred_until_idle",
        (Verification::Verified, "blocked") | (Verification::Failed(_), _) => "still_failing",
        (Verification::Verified, _) => "re_verified",
        (Verification::Started, _) => return None,
    };
    Some(Outcome {
        account: publication.account.clone(),
        generation: publication.version.generation.clone(),
        state,
        provider_verified: verification == Verification::Verified,
        reason: receipt.last_reason.clone(),
    })
}

pub(in crate::auth_incident) fn matches_replacement(
    receipt: &Receipt,
    record: &SessionRecord,
) -> bool {
    receipt.verification == Some(Verification::Verified)
        && record.runtime.as_ref().is_some_and(|runtime| {
            receipt.new_runtime_incarnation.as_deref() == Some(runtime.launch_id.as_str())
                && receipt.new_runtime_generation == Some(runtime.generation)
        })
}

/// One summary row per machine/account/publication across the visible records.
/// Deferred rows become restarted in the same summary when their idle gate opens.
pub(crate) fn summary(records: &[Value]) -> Option<Value> {
    let mut groups = BTreeMap::new();
    for record in records {
        if record["state"] == "closed" {
            continue;
        }
        let outcome = &record["auth_recovery"];
        let (Some(machine), Some(account), Some(generation)) = (
            record["machine"].as_str(),
            outcome["account"].as_str(),
            outcome["generation"].as_str(),
        ) else {
            continue;
        };
        let counts = groups
            .entry((machine, account, generation))
            .or_insert([0; 4]);
        if outcome["provider_verified"] == true {
            counts[0] += 1;
        }
        match outcome["state"].as_str() {
            Some("restarted") => counts[1] += 1,
            Some("deferred_until_idle") => counts[2] += 1,
            Some("still_failing") => counts[3] += 1,
            _ => {}
        }
    }
    if groups.is_empty() {
        return None;
    }
    let rows: Vec<_> = groups
        .into_iter()
        .map(
            |(
                (machine, account, generation),
                [re_verified, restarted, deferred, still_failing],
            )| {
                json!({
                    "machine": machine, "account": account, "generation": generation,
                    "re_verified": re_verified, "restarted": restarted,
                    "deferred_until_idle": deferred, "still_failing": still_failing,
                })
            },
        )
        .collect();
    Some(json!(rows))
}
