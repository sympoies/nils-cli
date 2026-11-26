//! Project native results to the frozen refusal envelope without diagnostics.
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::cli::resume_move::destination::Outcome;

use super::wire::ALLOWLIST;

pub(crate) struct Failure {
    status: StatusCode,
    code: &'static str,
    action: &'static str,
    state: Option<Outcome>,
    no_create: bool,
}

impl Failure {
    pub(crate) fn read(status: StatusCode, code: &'static str) -> Self {
        Self {
            status,
            code,
            action: allowlisted(code)
                .expect("destination refusal must be allowlisted")
                .1,
            state: None,
            no_create: false,
        }
    }

    pub(crate) fn before_start(status: StatusCode, code: &'static str) -> Self {
        Self {
            no_create: true,
            ..Self::read(status, code)
        }
    }

    pub(crate) fn unknown(state: Option<Outcome>) -> Self {
        Self {
            state,
            ..Self::read(StatusCode::BAD_GATEWAY, "resume-move-outcome-unknown")
        }
    }

    pub(crate) fn after_start(code: &str, state: Option<Outcome>) -> Self {
        if let Some(state @ (Outcome::Prepared | Outcome::Staged | Outcome::AbortedNoWriter)) =
            state
            && let Some(status) = pre_admission_status(code)
            && let Some((code, action)) = allowlisted(code)
        {
            return Self {
                status,
                code,
                action,
                state: Some(state),
                no_create: true,
            };
        }
        Self::unknown(state)
    }

    pub(crate) fn envelope(self, schema: &str, move_id: Option<&str>) -> (StatusCode, Value) {
        let mut details = json!({"safe_to_retry":false,"next_action":self.action});
        if let Some(move_id) = move_id {
            details["move_id"] = json!(move_id);
        }
        if let Some(state) = self.state {
            details["state"] = json!(state);
        }
        if self.no_create {
            details["session_created"] = json!(false);
        }
        (
            self.status,
            json!({"schema_version":schema,"ok":false,"error":{"code":self.code,"details":details}}),
        )
    }
}

fn allowlisted(code: &str) -> Option<(&'static str, &'static str)> {
    ALLOWLIST
        .iter()
        .copied()
        .find(|(candidate, _)| *candidate == code)
}

fn pre_admission_status(code: &str) -> Option<StatusCode> {
    use StatusCode as Http;

    Some(match code {
        "resume-unsupported" => Http::BAD_REQUEST,
        "resume-move-binding-mismatch" => Http::FORBIDDEN,
        "resume-move-proof-missing" => Http::NOT_FOUND,
        "session-exists" | "resume-move-destination-writer-present" | "resume-move-aborted" => {
            Http::CONFLICT
        }
        "resume-account-mismatch"
        | "resume-history-missing"
        | "resume-source-live"
        | "resume-move-session-mismatch"
        | "resume-move-proof-invalid"
        | "resume-move-proof-expired"
        | "resume-move-target-mismatch"
        | "resume-move-path-mismatch"
        | "resume-move-transcript-cwd-outside-checkout"
        | "resume-move-history-stale"
        | "resume-move-history-unsupported" => Http::UNPROCESSABLE_ENTITY,
        "resume-move-ssh-keygen-unavailable"
        | "resume-move-trust-unavailable"
        | "resume-move-signing-unavailable"
        | "resume-move-state-unavailable"
        | "resume-move-unavailable" => Http::SERVICE_UNAVAILABLE,
        _ => return None,
    })
}

#[cfg(test)]
#[test]
fn console_resume_move_refusal_requires_observed_pre_admission_state() {
    use crate::cli::resume_move::Refusal;

    for (native, action) in [
        (Refusal::ProofInvalid, "inspect_destination"),
        (Refusal::ProofExpired, "inspect_destination"),
        (Refusal::MoveMismatch, "inspect_destination"),
        (Refusal::SessionMismatch, "inspect_source"),
        (Refusal::TargetMismatch, "inspect_destination"),
        (Refusal::AccountMismatch, "inspect_destination"),
        (Refusal::SourceLive, "inspect_source"),
        (Refusal::SourceBusy, "inspect_source"),
        (Refusal::SourceAlreadyStopped, "inspect_source"),
        (Refusal::PathMismatch, "inspect_destination"),
        (Refusal::TranscriptCwdOutsideCheckout, "inspect_destination"),
        (Refusal::HistoryStale, "inspect_destination"),
        (Refusal::HistoryUnsupported, "inspect_destination"),
        (Refusal::DestinationWriterPresent, "reconcile_destination"),
        (Refusal::SshKeygenUnavailable, "inspect_destination"),
        (Refusal::TrustUnavailable, "inspect_destination"),
        (Refusal::SigningUnavailable, "inspect_destination"),
        (Refusal::SourceFenced, "inspect_source"),
        (Refusal::StateUnavailable, "inspect_destination"),
    ] {
        assert_eq!(allowlisted(native.code()), Some((native.code(), action)));
    }
    let verified = Failure::after_start(Refusal::PathMismatch.code(), Some(Outcome::Staged));
    assert!(verified.no_create);
    assert_eq!(verified.status, StatusCode::UNPROCESSABLE_ENTITY);
    for (code, state) in [
        (Refusal::PathMismatch.code(), Some(Outcome::WriterAdmitted)),
        (Refusal::PathMismatch.code(), Some(Outcome::Created)),
        (Refusal::PathMismatch.code(), None),
        ("unlisted", Some(Outcome::Staged)),
    ] {
        let unresolved = Failure::after_start(code, state);
        assert!(!unresolved.no_create);
        assert_eq!(unresolved.code, "resume-move-outcome-unknown");
        assert_eq!(unresolved.action, "reconcile_destination");
    }
}
