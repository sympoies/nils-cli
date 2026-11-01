//! Why a Claude launch did not attach the native notice module, and the typed
//! refusal a notice reports for it. The record is diagnostic only: attachment is
//! decided at launch and nothing here grants delivery.
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::runtime;
use super::version::{SUPPORTED_MAX, SUPPORTED_MIN, Version};
use crate::{CliContext, CliError, SessionRecord};

const SCHEMA: &str = "agent-session.claude-notice-attach.v1";
const STATE: &str = "not-attached";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Reason {
    WritersDisabled,
    ProviderVersionUnsupported,
    ProviderVersionUnreadable,
}

impl Reason {
    fn as_str(self) -> &'static str {
        match self {
            Self::WritersDisabled => "writers-disabled",
            Self::ProviderVersionUnsupported => "provider-version-unsupported",
            Self::ProviderVersionUnreadable => "provider-version-unreadable",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [
            Self::WritersDisabled,
            Self::ProviderVersionUnsupported,
            Self::ProviderVersionUnreadable,
        ]
        .into_iter()
        .find(|reason| reason.as_str() == text)
    }

    fn version_related(self) -> bool {
        self != Self::WritersDisabled
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Status {
    schema_version: String,
    state: String,
    reason: String,
    provider_version: Option<String>,
    supported_min: String,
    supported_max: String,
}

/// Best effort: a missing diagnostic must never fail a launch.
pub(crate) fn record(
    state: &Path,
    session: &SessionRecord,
    reason: Reason,
    provider_version: Option<&Version>,
) {
    let write = || -> Result<(), CliError> {
        let dir = runtime::module_dir(state, session)?;
        runtime::private_dir(dir.parent().ok_or_else(runtime::unavailable)?)?;
        runtime::private_dir(&dir)?;
        let status = Status {
            schema_version: SCHEMA.into(),
            state: STATE.into(),
            reason: reason.as_str().into(),
            provider_version: provider_version.map(Version::to_string),
            supported_min: SUPPORTED_MIN.to_string(),
            supported_max: SUPPORTED_MAX.to_string(),
        };
        runtime::write_new(
            &dir.join("attach.json"),
            &serde_json::to_vec(&status).map_err(|_| runtime::unavailable())?,
        )
    };
    let _ = write();
}

struct Recorded {
    reason: Reason,
    provider_version: Option<Version>,
    supported_min: Version,
    supported_max: Version,
}

fn read(context: &CliContext, session: &SessionRecord) -> Option<Recorded> {
    let dir = runtime::module_dir(&context.state_dir, session).ok()?;
    if !runtime::existing_private_dir(&dir) {
        return None;
    }
    let status: Status =
        serde_json::from_slice(&runtime::private_read(&dir.join("attach.json"), 4096).ok()?)
            .ok()?;
    let reason = Reason::parse(&status.reason)?;
    let provider_version = match status.provider_version.as_deref() {
        Some(text) => Some(Version::parse(text)?),
        None => None,
    };
    if status.schema_version != SCHEMA || status.state != STATE {
        return None;
    }
    Some(Recorded {
        reason,
        provider_version,
        supported_min: Version::parse(&status.supported_min)?,
        supported_max: Version::parse(&status.supported_max)?,
    })
}

/// The admission refusal for a Claude notice that has no attached native
/// module. It precedes every claim and input and adds no retry or fallback
/// permission; the code separates a version problem from every other cause.
pub(crate) fn refusal(context: &CliContext, session: &SessionRecord) -> CliError {
    let recorded = read(context, session);
    let (code, message) = match &recorded {
        Some(recorded) if recorded.reason.version_related() => (
            "notice-native-version-unsupported",
            "native notice module is not attached: the Claude Code version is outside the tested range",
        ),
        _ => (
            "notice-transport-unsupported",
            "Claude terminal notices cannot be verified exactly; delivery is unsupported",
        ),
    };
    let reason = recorded
        .as_ref()
        .map_or("unrecorded", |recorded| recorded.reason.as_str());
    let (version, min, max) =
        recorded
            .as_ref()
            .map_or((None, SUPPORTED_MIN, SUPPORTED_MAX), |recorded| {
                (
                    recorded.provider_version,
                    recorded.supported_min,
                    recorded.supported_max,
                )
            });
    CliError::runtime(
        code,
        message,
        Some(
            json!({"id":session.id,"guard":"transport","stage":"admission","atomic":false,
            "pasted":false,"submitted":false,
            "native_module":{"attached":false,"reason":reason,
                "provider_version":version.map(|version| version.to_string()),
                "supported_min":min.to_string(),"supported_max":max.to_string()}}),
        ),
    )
}

/// Only known absence of the module permits the mailbox's existing plain route.
/// An installed module with missing/invalid readiness remains uncertain.
pub(crate) fn known_unsupported(context: &CliContext, session: &SessionRecord) -> bool {
    read(context, session).is_some()
        || runtime::module_dir(&context.state_dir, session).is_ok_and(|dir| {
            std::fs::symlink_metadata(dir)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        })
}
