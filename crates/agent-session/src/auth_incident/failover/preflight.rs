//! Auth switching requires an explicit broker policy decision and fresh capacity.
use crate::account_broker::BrokerClient;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Deserialize)]
struct Inventory {
    schema_version: String,
    provider: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    selection_strategies: Vec<String>,
    accounts: Vec<Account>,
}

#[derive(Deserialize)]
struct Account {
    account: String,
}

#[derive(Deserialize)]
struct Response {
    schema_version: String,
    provider: Option<String>,
    account: Option<String>,
    auth_failover: Proof,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Proof {
    pub policy_allowed: bool,
    pub capacity_confirmed: bool,
    pub checked_at_epoch: i64,
    pub expires_at_epoch: i64,
}

impl Proof {
    pub(super) fn fresh(&self, now: i64) -> bool {
        self.policy_allowed
            && self.capacity_confirmed
            && self.checked_at_epoch <= now
            && now.saturating_sub(self.checked_at_epoch) <= 10
            && self.expires_at_epoch > now
            && self.expires_at_epoch <= self.checked_at_epoch.saturating_add(10)
    }
}

pub(super) fn select(
    broker: &BrokerClient,
    record: &crate::SessionRecord,
    operation_id: &str,
    after: &str,
) -> Result<(String, Proof), &'static str> {
    // One bounded inventory and one bounded selection per incident. The new
    // capability opts the broker into the auth-specific policy contract;
    // capacity selection support alone never authorizes an auth switch.
    let inventory: Inventory = broker
        .decode(
            broker
                .call("list", &[], Duration::from_secs(2))
                .map_err(|_| "preflight_unavailable")?,
            "invalid auth failover inventory",
            |r: &Inventory| (r.schema_version.as_str(), r.provider.as_deref()),
        )
        .map_err(|_| "preflight_invalid")?;
    if !inventory
        .capabilities
        .iter()
        .any(|c| c == "auth_failover_v1")
        || !inventory
            .selection_strategies
            .iter()
            .any(|s| s == "next_with_capacity")
    {
        return Err("policy_capability_missing");
    }
    if inventory.accounts.len() > 64 {
        return Err("preflight_invalid");
    }
    let mut names = Vec::new();
    for entry in inventory.accounts {
        broker
            .ensure_broker_nickname(&entry.account, "invalid auth account")
            .map_err(|_| "preflight_invalid")?;
        if names.contains(&entry.account) {
            return Err("preflight_invalid");
        }
        names.push(entry.account);
    }
    let response: Response = broker
        .decode(
            broker
                .call(
                    "select",
                    &[
                        "--strategy",
                        "next_with_capacity",
                        "--after",
                        after,
                        "--exclude",
                        after,
                        "--reason",
                        "authentication",
                        "--session-id",
                        &record.id,
                        "--runtime-incarnation",
                        &record.runtime.as_ref().ok_or("runtime_missing")?.launch_id,
                        "--operation-id",
                        operation_id,
                    ],
                    Duration::from_secs(2),
                )
                .map_err(|_| "preflight_unavailable")?,
            "invalid auth failover selection",
            |r: &Response| (r.schema_version.as_str(), r.provider.as_deref()),
        )
        .map_err(|_| "preflight_invalid")?;
    if !response.auth_failover.policy_allowed {
        return Err("policy_denied");
    }
    let account = response.account.ok_or("no_candidate")?;
    if !response
        .auth_failover
        .fresh(jiff::Timestamp::now().as_second())
    {
        return Err("capacity_unconfirmed_or_stale");
    }
    if account == after || !names.contains(&account) {
        return Err("candidate_invalid");
    }
    Ok((account, response.auth_failover))
}
