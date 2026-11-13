//! Closed-set diagnostics: never retain provider errors, status strings or paths.
use super::trace::Outcome;
use crate::CliError;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum IdleReason {
    RuntimeConstructionFailed,
    ConnectFailed,
    InitializeFailed,
    ObservationDeadlineExpired,
    LoadedListRpcFailed,
    ThreadReadRpcFailed,
    ThreadNotLoaded,
    ThreadStatusMissing,
    ThreadStatusUnknown,
}

impl IdleReason {
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::RuntimeConstructionFailed => {
                "Observation runtime unavailable: check local process and file-descriptor limits before another guarded send."
            }
            Self::ConnectFailed => {
                "App-server not reachable: restart the session before sending again."
            }
            Self::InitializeFailed => {
                "App-server initialization failed: restart the session before sending again."
            }
            Self::ObservationDeadlineExpired => {
                "Native idle observation timed out: inspect app-server health; restart the session if the timeout persists."
            }
            Self::LoadedListRpcFailed => {
                "Loaded thread identities could not be read: inspect app-server health; restart the session if the failure persists."
            }
            Self::ThreadReadRpcFailed => {
                "Native thread state could not be read: inspect app-server health; restart the session if the failure persists."
            }
            Self::ThreadNotLoaded => {
                "Thread not loaded: restart the session to reload its conversation before sending again."
            }
            Self::ThreadStatusMissing => {
                "Provider thread status is missing: check the configured Codex app-server version before restarting the session."
            }
            Self::ThreadStatusUnknown => {
                "Provider thread status is unrecognized: check the configured Codex app-server version before restarting the session."
            }
        }
    }

    pub(crate) fn from_error(error: &CliError) -> Option<Self> {
        (error.code() == "send-idle-unobservable")
            .then(|| error.details()?.get("native_idle_reason").cloned())
            .flatten()
            .and_then(|value| serde_json::from_value(value).ok())
    }

    pub(crate) fn annotate(self, mut error: CliError) -> CliError {
        let details = error.details_mut().get_or_insert_with(|| json!({}));
        details["native_idle_reason"] = json!(self);
        error.with_hint(self.hint())
    }

    pub(crate) fn notification_detail(self) -> String {
        // Serialization is a closed-set string, never a caller-controlled value.
        format!("send-idle-unobservable:{}", json!(self).as_str().unwrap())
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Error {
    IdentityUnobservable,
    IdentityMismatch,
    Unsupported,
    NotIdle,
    Unobservable(IdleReason),
}

impl Error {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::IdentityUnobservable => "send-identity-unobservable",
            Self::IdentityMismatch => "send-identity-mismatch",
            Self::Unsupported => "send-guard-unsupported",
            Self::NotIdle => "send-session-not-idle",
            Self::Unobservable(_) => "send-idle-unobservable",
        }
    }

    pub(crate) fn annotate(self, error: CliError) -> CliError {
        match self {
            Self::Unobservable(reason) => reason.annotate(error),
            _ => error,
        }
    }
}

pub(super) fn runtime_failure(_: std::io::Error) -> Error {
    Error::Unobservable(IdleReason::RuntimeConstructionFailed)
}

pub(super) fn initialize_failure(outcome: Outcome) -> (Outcome, IdleReason) {
    (
        outcome,
        match outcome {
            Outcome::Timeout => IdleReason::ObservationDeadlineExpired,
            _ => IdleReason::InitializeFailed,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn native_idle_reason_runtime_construction_is_redacted() {
        // Synthetic constructor failure; no process-wide resource exhaustion.
        let failure = runtime_failure(std::io::Error::other("private-runtime-error"));
        let error = failure.annotate(CliError::runtime(failure.code(), "idle unavailable", None));
        assert_eq!(
            error.details().unwrap()["native_idle_reason"],
            "runtime_construction_failed"
        );
        assert_eq!(
            error.clone().into_inner().hint.as_deref(),
            Some(
                "Observation runtime unavailable: check local process and file-descriptor limits before another guarded send."
            )
        );
        assert!(!format!("{error:?}").contains("private-runtime-error"));
    }

    #[test]
    fn native_idle_reason_inner_initialize_timeout_is_deadline_expired() {
        // Exercise the inner receive-timeout mapping independently of the
        // enclosing connection timer; racing real timers cannot pin this path.
        let (_, reason) = initialize_failure(Outcome::Timeout);
        assert_eq!(reason, IdleReason::ObservationDeadlineExpired);
    }
}
