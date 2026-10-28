//! Token-free context received with a credential snapshot. This is transport
//! provenance, not proof of local publication, provider validity or runtime adoption.
use nils_common::provider_runtime::remote::RemoteSelector;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Generation {
    schema_version: String,
    provider: String,
    account: String,
    version: Version,
    snapshot_id: String,
    authority_committed_at: String,
    #[serde(deserialize_with = "required_nullable")]
    issued_at: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    expires_at: Option<String>,
    auth: Observation,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Version {
    authority_id: String,
    authority_epoch: String,
    generation: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    revision: String,
    state: String,
    reason: String,
    source: String,
    #[serde(deserialize_with = "required_nullable")]
    observed_at: Option<String>,
}

fn required_nullable<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

fn counter(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|n| n > 0 && n.to_string() == value)
}

pub(crate) fn uuid(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 36
        && b[14] == b'4'
        && matches!(b[19], b'8' | b'9' | b'a' | b'b')
        && b.iter().enumerate().all(|(i, c)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                *c == b'-'
            } else {
                c.is_ascii_digit() || matches!(c, b'a'..=b'f')
            }
        })
}

fn time(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 24
        && b[10] == b'T'
        && b[19] == b'.'
        && b[23] == b'Z'
        && b[20..23].iter().all(u8::is_ascii_digit)
        && chrono::DateTime::parse_from_rfc3339(value)
            .is_ok_and(|dt| dt.timestamp_subsec_nanos() < 1_000_000_000)
}

pub(crate) fn context(payload: &Value) -> Result<Option<Generation>, ()> {
    let Some(value) = payload.get("credential_generation") else {
        return if payload.get("publication_attempt_id").is_some() {
            Err(())
        } else {
            Ok(None)
        };
    };
    let generation: Generation = serde_json::from_value(value.clone()).map_err(|_| ())?;
    let g = &generation;
    let a = &g.auth;
    let observation = matches!(
        (a.state.as_str(), a.reason.as_str(), a.source.as_str()),
        (
            "valid",
            "provider_accepted",
            "provider_refresh" | "provider_validation"
        ) | ("expired", "provider_expired", "provider_validation")
            | ("expired", "expiry_elapsed", "expiry_clock")
            | (
                "revoked",
                "provider_revoked" | "provider_rejected",
                "provider_refresh" | "provider_validation"
            )
            | ("unknown", "unvalidated" | "ambiguous_rotation", "unknown")
    );
    if payload
        .get("publication_attempt_id")
        .is_some_and(|v| !v.as_str().is_some_and(uuid))
        || g.schema_version != "nils.auth-generation.v1"
        || g.provider != "claude"
        || super::store::validate_profile_name(&g.account).is_err()
        || payload.get("profile").and_then(Value::as_str) != Some(g.account.as_str())
        || !uuid(&g.version.authority_id)
        || !uuid(&g.version.authority_epoch)
        || !uuid(&g.snapshot_id)
        || !counter(&g.version.generation)
        || !counter(&a.revision)
        || !time(&g.authority_committed_at)
        || !g.issued_at.as_deref().is_none_or(time)
        || !g.expires_at.as_deref().is_none_or(time)
        || !a.observed_at.as_deref().is_none_or(time)
        || !observation
        || (a.observed_at.is_none() && (a.state != "unknown" || a.source != "unknown"))
    {
        return Err(());
    }
    Ok(Some(generation))
}

pub(crate) fn validate(payload: &Value, selector: &RemoteSelector) -> bool {
    match selector {
        RemoteSelector::All => {
            !payload.as_object().is_some_and(|p| {
                p.contains_key("credential_generation") || p.contains_key("publication_attempt_id")
            }) && payload
                .get("profiles")
                .and_then(Value::as_array)
                .is_none_or(|profiles| profiles.iter().all(|profile| context(profile).is_ok()))
        }
        RemoteSelector::Name(name) => {
            context(payload).is_ok_and(|g| g.is_none_or(|g| g.account == *name))
        }
        RemoteSelector::Current => context(payload).is_ok(),
    }
}
