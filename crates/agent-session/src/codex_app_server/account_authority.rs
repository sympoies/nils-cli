use std::fs;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixDatagram;
use std::time::Duration;

use tokio::time::Instant;

use super::{
    CONTROL_SUBMIT_TOTAL_TIMEOUT, CliContext, FreshBootstrap, MANUAL_INPUT_SECTION_TTL,
    MANUAL_INPUT_SECTION_VERSION, ManualInputGate, Message, SessionRecord, Value, json,
    json_id_key, manual_input_ack_path, manual_input_gate_path,
    manual_input_request_matches_bound_thread, manual_input_section_path,
    marker_has_live_shared_lock, open_manual_input_gate_file, protocol_id_is_valid,
    read_runtime_process_marker_file, valid_runtime_process_marker,
};

pub(super) async fn wait_for_bootstrap_binding(
    context: &CliContext,
    record: &SessionRecord,
    method: Option<&str>,
    bootstrap: &mut FreshBootstrap,
) -> Result<(), TuiMutationRejection> {
    let account_read = method == Some("account/read")
        && crate::codex_account::proxy_binding_diagnostic(record).0 == "pending";
    if !account_read
        && (method != Some("thread/start")
            || !matches!(bootstrap, FreshBootstrap::ThreadStart)
            || !crate::codex_account::proxy_initial_binding_is_pending(record))
    {
        return Ok(());
    }
    let deadline = Instant::now() + CONTROL_SUBMIT_TOTAL_TIMEOUT;
    // The proxy record is a launch snapshot. Fence this request to its first
    // live binding, then reject account/revision changes while it waits.
    let mut expected_binding = None;
    let expected_runtime_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .ok_or(TuiMutationRejection::RuntimeIdentityMissing)?;
    loop {
        if Instant::now() >= deadline {
            return Err(initial_binding_rejection(
                bootstrap,
                context,
                record,
                "initial-binding-unresolved",
            )
            .await);
        }
        let load_context = context.clone();
        let id = record.id.clone();
        let current = match tokio::task::spawn_blocking(move || {
            crate::load_session_record(&load_context, &id)
        })
        .await
        {
            Ok(Ok(current)) => current,
            Ok(Err(_)) => {
                return Err(initial_binding_rejection(
                    bootstrap,
                    context,
                    record,
                    "initial-binding-record-unavailable",
                )
                .await);
            }
            Err(_) => {
                return Err(initial_binding_rejection(
                    bootstrap,
                    context,
                    record,
                    "initial-binding-task-failed",
                )
                .await);
            }
        };
        if crate::ensure_same_session_identity(record, &current).is_err() {
            return Err(initial_binding_rejection(
                bootstrap,
                context,
                record,
                "initial-binding-runtime-changed",
            )
            .await);
        }
        if account_read {
            let binding = (
                crate::codex_account::selected_account(&current),
                crate::codex_account::proxy_binding_diagnostic(&current).1,
            );
            let expected_binding = expected_binding.get_or_insert_with(|| binding.clone());
            if &binding != expected_binding {
                return Err(initial_binding_rejection(
                    bootstrap,
                    context,
                    record,
                    "initial-binding-replaced",
                )
                .await);
            }
            if crate::codex_account::ensure_terminal_input_allowed(&current).is_ok() {
                return Ok(());
            }
        }
        if crate::codex_account::proxy_initial_binding_is_bound(&current, expected_runtime_id) {
            return Ok(());
        }
        if if account_read {
            crate::codex_account::proxy_binding_diagnostic(&current).0 != "pending"
        } else {
            !crate::codex_account::proxy_initial_binding_is_pending(&current)
        } {
            let code = match crate::codex_account::proxy_binding_diagnostic(&current) {
                ("failed", _) => "initial-binding-failed",
                ("unknown", _) => "initial-binding-invalid",
                ("pending", _) | ("bound", _) | ("unbound", _) | ("unsupported", _) => {
                    "initial-binding-replaced"
                }
                _ => "initial-binding-invalid",
            };
            return Err(initial_binding_rejection(bootstrap, context, record, code).await);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn initial_binding_rejection(
    bootstrap: &mut FreshBootstrap,
    context: &CliContext,
    record: &SessionRecord,
    code: &'static str,
) -> TuiMutationRejection {
    bootstrap.close();
    account_authority_rejection(
        context,
        record,
        "thread_start_binding",
        "initial_binding_wait",
        code,
        false,
    )
    .await
}

pub(super) fn acquire_turn_start_gate(
    context: &CliContext,
    record: &SessionRecord,
    value: &Value,
) -> Result<ManualInputGate, TuiMutationRejection> {
    if value.get("method").and_then(Value::as_str) != Some("turn/start")
        || value.get("id").and_then(json_id_key).is_none()
        || !value.pointer("/params/input").is_some_and(Value::is_array)
        || !value
            .pointer("/params/threadId")
            .and_then(Value::as_str)
            .is_some_and(protocol_id_is_valid)
    {
        return Err(TuiMutationRejection::TurnRequestInvalid);
    }
    let gate_file = open_manual_input_gate_file(&manual_input_gate_path(context, record))
        .ok_or(TuiMutationRejection::TurnGateOpenFailed)?;
    // A proxy never waits here: a manual sender may already own the session
    // record lock, and a competing account mutation drops that record lock
    // when its own non-blocking gate attempt loses.
    // SAFETY: `flock` observes the valid descriptor borrowed for this call.
    if unsafe { libc::flock(gate_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(TuiMutationRejection::TurnGateBusy);
    }

    if crate::restart::ensure_not_restarting(context, record).is_err() {
        return Err(TuiMutationRejection::TurnGateBusy);
    }
    let marker_path = manual_input_section_path(context, record);
    let Ok(mut owner_file) = fs::File::open(&marker_path) else {
        return Ok(ManualInputGate {
            gate_file,
            _owner_file: None,
        });
    };
    if !marker_has_live_shared_lock(&owner_file) {
        return Ok(ManualInputGate {
            gate_file,
            _owner_file: None,
        });
    }
    if !manual_input_request_matches_bound_thread(context, record, value) {
        // Keep the rejected TUI's candidate for explicit, provider-verified
        // recovery. This does not change the binding or admit the failed turn.
        if let Some(thread_id) = value.pointer("/params/threadId").and_then(Value::as_str) {
            let _ = crate::conversation::retain_observation(context, record, thread_id);
        }
        return Err(TuiMutationRejection::ManualMarkerThreadMismatch);
    }
    let own = owner_file
        .metadata()
        .map_err(|_| TuiMutationRejection::ManualMarkerReplaced)?;
    let current =
        fs::metadata(&marker_path).map_err(|_| TuiMutationRejection::ManualMarkerReplaced)?;
    if own.dev() != current.dev() || own.ino() != current.ino() {
        return Err(TuiMutationRejection::ManualMarkerReplaced);
    }
    let marker = read_runtime_process_marker_file(&mut owner_file)
        .ok_or(TuiMutationRejection::ManualMarkerInvalid)?;
    if !valid_runtime_process_marker(
        &marker,
        record,
        MANUAL_INPUT_SECTION_VERSION,
        MANUAL_INPUT_SECTION_TTL,
    ) {
        return Err(TuiMutationRejection::ManualMarkerInvalid);
    }
    UnixDatagram::unbound()
        .map_err(|_| TuiMutationRejection::ManualAckFailed)?
        .send_to(
            &[1],
            manual_input_ack_path(record).ok_or(TuiMutationRejection::ManualAckFailed)?,
        )
        .map_err(|_| TuiMutationRejection::ManualAckFailed)?;
    Ok(ManualInputGate {
        gate_file,
        _owner_file: Some(owner_file),
    })
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
    let mut data = match rejection {
        TuiMutationRejection::AccountNotReady => json!({
            "reason": rejection.code(),
            "retryable": true,
            "next_action": "inspect-account-switch",
            "hint": "Inspect account show or list for the pending switch; retry account switch or restart the session if it remains blocked, then resend the prompt."
        }),
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
    let retryable = matches!(rejection, TuiMutationRejection::TurnGateBusy)
        || data["retryable"].as_bool().unwrap_or(false);
    data["retryable"] = json!(retryable);
    if data.get("next_action").is_none() {
        data["next_action"] = json!(if retryable {
            "retry-request-after-state-change"
        } else {
            "inspect-session-state"
        });
    }
    data["recovery"] = json!({ "outcome": "not_forwarded" });
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
