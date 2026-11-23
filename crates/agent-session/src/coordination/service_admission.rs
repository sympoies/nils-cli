//! The current admission document and exact-generation recipient resolution.
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Deserializer};
use serde_json::json;

use super::{Origin, credential, unauthorized, valid_selector};
use crate::CliError;

const VERSION: &str = "agent-session.mailbox-services.v2";
const MAX_BYTES: u64 = 32 * 1024;
const MAX_ACTIVE: usize = 32;
const SCHEMA_WARNING_INTERVAL: Duration = Duration::from_secs(60);
static LAST_SCHEMA_WARNING: Mutex<Option<Instant>> = Mutex::new(None);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    #[serde(rename = "schema_version", deserialize_with = "require_current_schema")]
    _schema_version: (),
    services: Vec<Entry>,
    retired: Vec<Retirement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    service_id: String,
    service_generation: String,
    credential_file: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Retirement {
    service_id: String,
    service_generation: String,
}

fn require_current_schema<'de, D: Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
    if String::deserialize(deserializer)? == VERSION {
        return Ok(());
    }
    // Report the declared version mismatch before any missing v2 fields can mask it.
    let warn = LAST_SCHEMA_WARNING.lock().is_ok_and(|mut last_warning| {
        let now = Instant::now();
        if last_warning.is_some_and(|last| now.duration_since(last) < SCHEMA_WARNING_INTERVAL) {
            return false;
        }
        *last_warning = Some(now);
        true
    });
    if warn {
        let _ = writeln!(
            io::stderr().lock(),
            "warning: mailbox-service-admission-schema-unsupported: \
             unsupported mailbox service admission schema; v2 required. \
             See \"Admit a local mailbox service\" in the daemon runbook."
        );
    }
    Err(serde::de::Error::custom(
        "unsupported mailbox service admission schema",
    ))
}

impl Admission {
    fn read(file: Option<&Path>) -> Result<Option<Self>, CliError> {
        let Some(file) = file else { return Ok(None) };
        let bytes = super::read_private_file(file, MAX_BYTES).map_err(|_| unavailable())?;
        let admission: Self = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        let mut active = HashSet::new();
        let mut retired = HashSet::new();
        if admission.services.len() > MAX_ACTIVE
            || admission.services.iter().any(|entry| {
                !valid_selector(&entry.service_id)
                    || !valid_selector(&entry.service_generation)
                    || !entry.credential_file.is_absolute()
                    || !active.insert(&entry.service_id)
            })
            || admission.retired.iter().any(|marker| {
                !valid_selector(&marker.service_id)
                    || !valid_selector(&marker.service_generation)
                    || !retired.insert((&marker.service_id, &marker.service_generation))
            })
            || admission
                .services
                .iter()
                .any(|entry| retired.contains(&(&entry.service_id, &entry.service_generation)))
        {
            return Err(unavailable());
        }
        Ok(Some(admission))
    }

    fn unique_credential(&self, selected: &Entry) -> Result<String, CliError> {
        let token = credential(&selected.credential_file)?;
        for entry in &self.services {
            if entry.service_id != selected.service_id
                && credential(&entry.credential_file)
                    .is_ok_and(|other| crate::serve::constant_time_eq(&token, &other))
            {
                return Err(unauthorized());
            }
        }
        Ok(token)
    }
}

pub(crate) fn admitted_tokens(file: Option<&Path>) -> Vec<String> {
    Admission::read(file)
        .ok()
        .flatten()
        .into_iter()
        .flat_map(|admission| admission.services)
        .filter_map(|entry| credential(&entry.credential_file).ok())
        .collect()
}

pub(crate) fn authenticate(
    file: Option<&Path>,
    service_id: &str,
    generation: &str,
    token: &str,
    forbidden: &[&str],
) -> Result<(), CliError> {
    authenticate_inbox(file, service_id, generation, token, forbidden).map_err(|_| unauthorized())
}

pub(crate) fn authenticate_inbox(
    file: Option<&Path>,
    service_id: &str,
    generation: &str,
    token: &str,
    forbidden: &[&str],
) -> Result<(), CliError> {
    if !valid_selector(service_id)
        || !valid_selector(generation)
        || !(32..=256).contains(&token.len())
        || token.bytes().any(|byte| !byte.is_ascii_graphic())
        || forbidden
            .iter()
            .any(|other| crate::serve::constant_time_eq(token, other))
    {
        return Err(unauthorized());
    }
    let admission = Admission::read(file)?.ok_or_else(unauthorized)?;
    let entry = admission
        .services
        .iter()
        .find(|entry| entry.service_id == service_id && entry.service_generation == generation)
        .ok_or_else(unauthorized)?;
    let expected = admission.unique_credential(entry)?;
    if !crate::serve::constant_time_eq(token, &expected) {
        return Err(unauthorized());
    }
    Ok(())
}

pub(in crate::coordination) fn require_recipient(
    file: Option<&Path>,
    origin: &Origin,
) -> Result<(), CliError> {
    let admission = Admission::read(file)?;
    // The authenticated, stored local parent establishes the service identity.
    let (admission, entry) = resolve(admission.as_ref(), origin, true)?;
    admission
        .unique_credential(entry)
        .map_err(|_| unadmitted())?;
    Ok(())
}

fn resolve<'a>(
    admission: Option<&'a Admission>,
    origin: &Origin,
    verified_parent: bool,
) -> Result<(&'a Admission, &'a Entry), CliError> {
    if let Some(admission) = admission {
        if admission.retired.iter().any(|marker| {
            marker.service_id == origin.service_id
                && marker.service_generation == origin.service_generation
        }) {
            return Err(refusal(
                "mailbox-service-recipient-retired",
                "service generation is retired",
            ));
        }
        if let Some(entry) = admission
            .services
            .iter()
            .find(|entry| entry.service_id == origin.service_id)
        {
            if entry.service_generation != origin.service_generation {
                return Err(refusal(
                    "mailbox-service-recipient-generation-conflict",
                    "service recipient has a different active generation",
                ));
            }
            return Ok((admission, entry));
        }
        if admission
            .retired
            .iter()
            .any(|marker| marker.service_id == origin.service_id)
        {
            return Err(unadmitted());
        }
    }
    if verified_parent {
        Err(unadmitted())
    } else {
        Err(refusal(
            "mailbox-service-recipient-unknown",
            "service recipient identity is unknown",
        ))
    }
}

fn unavailable() -> CliError {
    CliError::unavailable(
        "mailbox-service-unavailable",
        "mailbox service admission is unavailable",
        None,
    )
}

fn unadmitted() -> CliError {
    refusal(
        "mailbox-service-recipient-unadmitted",
        "service recipient is not admitted",
    )
}

pub(in crate::coordination) fn refusal(code: &'static str, message: &'static str) -> CliError {
    CliError::data(code, message, Some(json!({"retryable": false})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_distinguishes_unknown_from_the_verified_origin_of_a_revoked_service() {
        let origin = Origin {
            machine: "fixture-machine".into(),
            service_id: "reporter".into(),
            service_generation: "generation-1".into(),
        };
        let code = |verified_parent| {
            resolve(None, &origin, verified_parent)
                .err()
                .unwrap()
                .code()
                .to_string()
        };
        pretty_assertions::assert_eq!(code(false), "mailbox-service-recipient-unknown");
        pretty_assertions::assert_eq!(code(true), "mailbox-service-recipient-unadmitted");
    }
}
