//! Token-free local publication evidence. Materialization is never a generation.
use super::*;
use serde_json::Value;
use std::time::Duration;

const KEY: &str = "claude_launch_publication";
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Version {
    pub authority_id: String,
    pub authority_epoch: String,
    pub generation: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Publication {
    pub host_id: String,
    pub account: String,
    pub version: Version,
    pub snapshot_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LaunchPublication {
    runtime_incarnation: String,
    publication: Publication,
}

fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| {
        id.get_variant() == uuid::Variant::RFC4122
            && id.get_version_num() == 4
            && id.to_string() == value
    })
}

fn counter(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|n| n > 0 && n.to_string() == value)
}

fn valid(publication: &Publication) -> bool {
    uuid(&publication.host_id)
        && uuid(&publication.snapshot_id)
        && uuid(&publication.version.authority_id)
        && uuid(&publication.version.authority_epoch)
        && counter(&publication.version.generation)
        && nils_common::provider_runtime::accounts::is_valid_account_nickname(&publication.account)
}

impl Publication {
    pub(crate) fn newer_than(&self, prior: &Self) -> bool {
        valid(self)
            && valid(prior)
            && self.host_id == prior.host_id
            && self.account == prior.account
            && self.version.authority_id == prior.version.authority_id
            && self.version.authority_epoch == prior.version.authority_epoch
            && self.version.generation.parse::<u64>().unwrap()
                > prior.version.generation.parse::<u64>().unwrap()
            && self.snapshot_id != prior.snapshot_id
    }
}

fn refused(reason: &'static str) -> CliError {
    CliError::runtime(
        "claude-publication-unavailable",
        "Claude publication evidence is unavailable",
        Some(json!({"reason":reason})),
    )
}

/// Validate only evidence used for recovery; never retain arbitrary broker fields.
pub(crate) fn decode(value: &Value, account: &str) -> Result<Option<Publication>, CliError> {
    let bad = || refused("invalid_response");
    let state = &value["state"];
    let authority = &state["authority"];
    let p = &state["publication"];
    if value["schema_version"] != "agent-session.credential-state.v1"
        || value["provider"] != "claude"
        || value["account"] != account
        || value["ok"] != true
        || !value["error"].is_null()
        || value["read_at"]
            .as_str()
            .and_then(|s| s.parse::<jiff::Timestamp>().ok())
            .is_none()
    {
        return Err(bad());
    }
    if p["status"] != "published" {
        return Ok(None);
    }
    let receipt = &p["last_published"];
    let publication = Publication {
        host_id: value["host_id"].as_str().ok_or_else(bad)?.into(),
        account: account.into(),
        version: serde_json::from_value(receipt["version"].clone()).map_err(|_| bad())?,
        snapshot_id: receipt["snapshot_id"].as_str().ok_or_else(bad)?.into(),
    };
    let stores = p["stores"].as_array().ok_or_else(bad)?;
    let kinds = [
        "account_credentials",
        "account_config",
        "keychain",
        "directory_preparation",
        "active_credentials",
        "proxy_projection",
    ];
    if !valid(&publication)
        || p["target"] != receipt["version"]
        || authority["version"] != receipt["version"]
        || p["snapshot_id"] != receipt["snapshot_id"]
        || authority["snapshot_id"] != receipt["snapshot_id"]
        || p["policy_revision"] != receipt["policy_revision"]
        || !receipt["policy_revision"].as_str().is_some_and(counter)
        || p["published_at"] != receipt["published_at"]
        || receipt["published_at"]
            .as_str()
            .and_then(|s| s.parse::<jiff::Timestamp>().ok())
            .is_none()
        || !p["failure"].is_null()
        || stores.len() != kinds.len()
        || !kinds
            .iter()
            .all(|kind| stores.iter().filter(|s| s["kind"] == *kind).count() == 1)
        || !stores.iter().all(|s| {
            if s["required"] == true {
                s["status"] == "verified"
                    && s["verified_at"]
                        .as_str()
                        .and_then(|s| s.parse::<jiff::Timestamp>().ok())
                        .is_some()
            } else {
                s["required"] == false
                    && s["status"] == "not_required"
                    && s["verified_at"].is_null()
            }
        })
        || ![
            "account_credentials",
            "account_config",
            "directory_preparation",
        ]
        .iter()
        .all(|kind| {
            stores
                .iter()
                .any(|s| s["kind"] == *kind && s["required"] == true)
        })
        || !matches!(
            authority["auth"]["state"].as_str(),
            Some("unknown" | "valid")
        )
    {
        return Err(bad());
    }
    Ok(Some(publication))
}

pub(crate) fn published(account: &str) -> Result<Option<Publication>, CliError> {
    let list = BROKER.call("list", &[], TIMEOUT)?;
    if list["schema_version"] != BROKER_SCHEMA_VERSION
        || list["provider"] != "claude"
        || !list["capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|c| c == "credential_state_v1"))
    {
        return Err(refused("capability_missing"));
    }
    decode(
        &BROKER.call("credential-state", &["--account", account], TIMEOUT)?,
        account,
    )
}

#[derive(Deserialize)]
struct SwitchBrokerList {
    schema_version: String,
    provider: Option<String>,
    accounts: Vec<AccountSummary>,
    #[serde(default)]
    capabilities: Vec<String>,
}

/// Manual account replacement supports account-broker.v2 without optional
/// generation tracking. Only a validated inventory proves capability absence;
/// publication failures from a broker that advertises it must still refuse.
pub(crate) fn verify_switch(account: &str) -> Result<(), CliError> {
    let list: SwitchBrokerList = BROKER.decode(
        BROKER.call("list", &[], TIMEOUT)?,
        "an invalid account list",
        |r: &SwitchBrokerList| (r.schema_version.as_str(), r.provider.as_deref()),
    )?;
    if !list.accounts.iter().any(|entry| entry.account == account) {
        return Err(BROKER.invalid_response("an account list missing the target account"));
    }
    if list.capabilities.iter().any(|c| c == "credential_state_v1")
        && decode(
            &BROKER.call("credential-state", &["--account", account], TIMEOUT)?,
            account,
        )?
        .is_none()
    {
        return Err(CliError::data(
            "claude-account-credentials-unverified",
            "the next Claude account has no verified credential publication",
            None,
        ));
    }
    Ok(())
}

/// Compatibility launches proceed without capability, but cannot later infer a
/// baseline from timestamps, binding revisions or credential materialization.
pub(crate) fn capture(record: &mut SessionRecord, account: &str) {
    record.extra.remove(KEY);
    if let (Some(runtime), Ok(Some(publication))) = (&record.runtime, published(account)) {
        record.extra.insert(
            KEY.into(),
            json!(LaunchPublication {
                runtime_incarnation: runtime.launch_id.clone(),
                publication
            }),
        );
    }
}

pub(crate) fn baseline(record: &SessionRecord) -> Option<Publication> {
    let checkpoint: LaunchPublication =
        serde_json::from_value(record.extra.get(KEY)?.clone()).ok()?;
    (record.runtime.as_ref()?.launch_id == checkpoint.runtime_incarnation
        && valid(&checkpoint.publication))
    .then_some(checkpoint.publication)
}

#[cfg(test)]
pub(crate) mod tests;
