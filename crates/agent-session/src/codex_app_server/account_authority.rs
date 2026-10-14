use std::time::{Duration, Instant};

use super::{
    CONTROL_SUBMIT_TOTAL_TIMEOUT, CliContext, FreshBootstrap, Message, SessionRecord, Value, json,
};

pub(super) async fn wait_for_initial_thread_start_binding(
    context: &CliContext,
    record: &SessionRecord,
    method: Option<&str>,
    bootstrap: &FreshBootstrap,
) {
    if method != Some("thread/start")
        || !matches!(bootstrap, FreshBootstrap::ThreadStart)
        || !crate::codex_account::proxy_initial_binding_is_pending(record)
    {
        return;
    }
    let deadline = Instant::now() + CONTROL_SUBMIT_TOTAL_TIMEOUT;
    loop {
        let load_context = context.clone();
        let id = record.id.clone();
        let Ok(Ok(current)) =
            tokio::task::spawn_blocking(move || crate::load_session_record(&load_context, &id))
                .await
        else {
            return;
        };
        if crate::ensure_same_session_identity(record, &current).is_err()
            || !crate::codex_account::proxy_initial_binding_is_pending(&current)
            || Instant::now() >= deadline
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Serialize a provider `turn/start` with account mutation. The proxy takes
/// this gate before forwarding and retains it through the matching JSON-RPC
/// response, closing the accepted-but-not-yet-observed turn window. A manual
/// sender marker, when present, is revalidated under the same lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TuiMutationRejection {
    TurnAlreadyPending,
    AccountMutationForbidden,
    AccountNotReady,
    TurnRequestInvalid,
    TurnGateOpenFailed,
    TurnGateBusy,
    ManualMarkerThreadMismatch,
    ManualMarkerReplaced,
    ManualMarkerInvalid,
    ManualAckFailed,
    RuntimeIdentityMissing,
    ManualCancellationBusy {
        operation: &'static str,
        stage: &'static str,
        code: &'static str,
        retryable: bool,
        binding_state: &'static str,
        binding_revision: u64,
        receipt_written: bool,
    },
    RuntimeChanged,
    AccountAuthorityUnavailable {
        operation: &'static str,
        stage: &'static str,
        code: &'static str,
        retryable: bool,
        binding_state: &'static str,
        binding_revision: u64,
        receipt_written: bool,
    },
}

async fn record_cause(
    context: &CliContext,
    record: &SessionRecord,
    operation: &'static str,
    stage: &'static str,
    code: &'static str,
    retryable: bool,
) -> (&'static str, u64, bool) {
    let receipt_context = context.clone();
    let receipt_record = record.clone();
    tokio::task::spawn_blocking(move || {
        let current = crate::load_session_record(&receipt_context, &receipt_record.id)
            .ok()
            .filter(|current| {
                crate::ensure_same_session_identity(&receipt_record, current).is_ok()
            });
        let (binding_state, binding_revision) =
            current.as_ref().map_or(("unknown", 0), |current| {
                crate::codex_account::proxy_binding_diagnostic(current)
            });
        let receipt_written = crate::lifecycle::record_startup_cause_receipt(
            &receipt_context,
            &receipt_record,
            crate::lifecycle::StartupCauseReceipt {
                operation,
                stage,
                code,
                binding_state: binding_state.to_owned(),
                binding_revision,
                retryable,
            },
        )
        .is_ok();
        (binding_state, binding_revision, receipt_written)
    })
    .await
    .unwrap_or(("unknown", 0, false))
}

pub(super) async fn account_authority_rejection(
    context: &CliContext,
    record: &SessionRecord,
    operation: &'static str,
    stage: &'static str,
    code: &'static str,
    retryable: bool,
) -> TuiMutationRejection {
    let retryable = retryable && code == "session-record-lock-timeout";
    let (binding_state, binding_revision, receipt_written) =
        record_cause(context, record, operation, stage, code, retryable).await;
    TuiMutationRejection::AccountAuthorityUnavailable {
        operation,
        stage,
        code,
        retryable: retryable && receipt_written,
        binding_state,
        binding_revision,
        receipt_written,
    }
}

pub(super) async fn manual_cancellation_busy(
    context: &CliContext,
    record: &SessionRecord,
    operation: &'static str,
) -> TuiMutationRejection {
    let (binding_state, binding_revision, receipt_written) = record_cause(
        context,
        record,
        operation,
        "session_record_lock",
        "session-record-lock-busy",
        true,
    )
    .await;
    TuiMutationRejection::ManualCancellationBusy {
        operation,
        stage: "session_record_lock",
        code: "session-record-lock-busy",
        retryable: receipt_written,
        binding_state,
        binding_revision,
        receipt_written,
    }
}

impl TuiMutationRejection {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::TurnAlreadyPending => "turn_already_pending",
            Self::AccountMutationForbidden => "account_mutation_forbidden",
            Self::AccountNotReady => "account_not_ready",
            Self::TurnRequestInvalid => "turn_request_invalid",
            Self::TurnGateOpenFailed => "turn_gate_open_failed",
            Self::TurnGateBusy => "turn_gate_busy",
            Self::ManualMarkerThreadMismatch => "manual_marker_thread_mismatch",
            Self::ManualMarkerReplaced => "manual_marker_replaced",
            Self::ManualMarkerInvalid => "manual_marker_invalid",
            Self::ManualAckFailed => "manual_ack_failed",
            Self::RuntimeIdentityMissing => "runtime_identity_missing",
            Self::ManualCancellationBusy { .. } => "manual_cancellation_busy",
            Self::RuntimeChanged => "runtime_changed",
            Self::AccountAuthorityUnavailable { .. } => "account_authority_unavailable",
        }
    }
}

pub(super) fn tui_busy_response(id: &Value, rejection: TuiMutationRejection) -> Message {
    let data = match rejection {
        TuiMutationRejection::AccountAuthorityUnavailable {
            operation,
            stage,
            code,
            retryable,
            binding_state,
            binding_revision,
            receipt_written,
        } => json!({
            "reason": rejection.code(),
            "retryable": retryable,
            "next_action": if retryable { "retry-request" } else { "inspect-session-state" },
            "cause": {
                "operation": operation,
                "stage": stage,
                "code": code,
                "binding_state": binding_state,
                "binding_revision": binding_revision,
                "receipt_written": receipt_written
            }
        }),
        TuiMutationRejection::ManualCancellationBusy {
            operation,
            stage,
            code,
            retryable,
            binding_state,
            binding_revision,
            receipt_written,
        } => json!({
            "reason": rejection.code(),
            "retryable": retryable,
            "next_action": if retryable { "retry-request" } else { "inspect-session-state" },
            "cause": {
                "operation": operation,
                "stage": stage,
                "code": code,
                "binding_state": binding_state,
                "binding_revision": binding_revision,
                "receipt_written": receipt_written
            }
        }),
        _ => json!({ "reason": rejection.code() }),
    };
    Message::Text(
        json!({
            "id": id,
            "error": {
                "code": -32001,
                "message": "agent-session state is busy; retry the request",
                "data": data
            }
        })
        .to_string()
        .into(),
    )
}
