//! Idle-boundary drain of queued Codex account switches.
//!
//! A control whose applied binding is ready drains a queued next account
//! itself (`ApplyNext`). A control left without a ready binding (failed,
//! malformed, or still pending, for example after a daemon rebuild) refuses
//! that command, so a queued switch for such a session takes the
//! `PUT /sessions/{id}/account` path instead: under the same per-session switch
//! lock, the intent becomes an incarnation- and idle-fenced pending binding
//! that the control then binds.

use tokio::task::JoinSet;

use super::AccountSwitchRegistry;
use crate::CliContext;
use crate::codex_app_server::ControlHandle;

pub(super) async fn apply_pending_codex_accounts(handles: Vec<ControlHandle>) {
    let mut tasks = JoinSet::new();
    for handle in handles {
        tasks.spawn(async move {
            let _ = handle.apply_next().await;
        });
    }
    while tasks.join_next().await.is_some() {}
}

/// Promote and bind each queued switch whose session's binding is not ready.
/// `pending` holds `(session id, runtime launch id, control handle)`. A busy
/// or replaced session keeps its intent queued for a later reconcile tick.
pub(super) async fn repair_blocked_codex_accounts(
    context: &CliContext,
    switches: &AccountSwitchRegistry,
    pending: Vec<(String, String, ControlHandle)>,
) {
    let mut tasks = JoinSet::new();
    for (id, launch_id, handle) in pending {
        let context = context.clone();
        let switches = switches.clone();
        tasks.spawn(async move {
            let _switch_guard = switches.lock(&id).await;
            let begin_context = context.clone();
            let begin_id = id.clone();
            let begin_launch_id = launch_id.clone();
            let Ok(Ok(Some((account, revision)))) = tokio::task::spawn_blocking(move || {
                crate::codex_account::begin_queued_switch_repair(
                    &begin_context,
                    &begin_id,
                    &begin_launch_id,
                )
            })
            .await
            else {
                return;
            };
            if handle.bind_account(&account, revision).await.is_err() {
                let _ = tokio::task::spawn_blocking(move || {
                    crate::codex_account::finish_binding(
                        &context,
                        &id,
                        &launch_id,
                        &account,
                        revision,
                        Err("apply_failed"),
                    )
                })
                .await;
            }
        });
    }
    while tasks.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_app_server::{ControlCommand, control_channel};
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    const RUNTIME: &str = "runtime-binding-fixture";

    #[tokio::test]
    async fn queued_switch_repairs_a_binding_left_failed_by_a_daemon_rebuild() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = crate::codex_account::record_with_binding_value(json!({
            "schema_version": crate::codex_account::BINDING_SCHEMA_VERSION,
            "selected_account": "acct1",
            "revision": 7,
            "state": "failed",
            "failure_reason": "apply_failed",
            "updated_at": "2030-01-01T00:00:00Z"
        }));
        std::fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        crate::activity::activate_runtime(&context, &record).unwrap();
        for (event_id, kind) in [
            ("drain-started", "turn_started"),
            ("drain-done", "turn_completed"),
        ] {
            let event = serde_json::from_value(json!({
                "schema_version": crate::activity::TURN_EVENT_VERSION,
                "event_id": event_id,
                "runtime_id": RUNTIME,
                "provider": "codex",
                "provider_turn_id": "turn-drain",
                "kind": kind,
                "confidence": "authoritative"
            }))
            .unwrap();
            crate::activity::ingest_event(&context, &record.id, event).unwrap();
        }
        crate::codex_account::queue_next_account_with_unbound(
            &context, &record.id, RUNTIME, "acct1",
        )
        .unwrap();

        let (handle, mut commands) = control_channel();
        let control_context = context.clone();
        let control_id = record.id.clone();
        let control = tokio::spawn(async move {
            let Some(ControlCommand::BindAccount {
                account,
                revision,
                response,
            }) = commands.recv().await
            else {
                panic!("the blocked control did not receive the promoted switch");
            };
            let view = crate::codex_account::finish_binding(
                &control_context,
                &control_id,
                RUNTIME,
                &account,
                revision,
                Ok(()),
            );
            let _ = response.send(view.map_err(|err| err.code().to_string()));
            (account, revision)
        });
        repair_blocked_codex_accounts(
            &context,
            &AccountSwitchRegistry::default(),
            vec![(record.id.clone(), RUNTIME.to_string(), handle)],
        )
        .await;

        assert_eq!(control.await.unwrap(), ("acct1".to_string(), 8));
        let repaired = crate::load_session_record(&context, &record.id).unwrap();
        let view = crate::codex_account::view_for_record(&repaired);
        assert_eq!(view.state, "bound");
        assert_eq!(view.applied_runtime_id.as_deref(), Some(RUNTIME));
        assert!(view.next.is_none());
        assert!(crate::codex_account::ensure_input_allowed(&repaired).is_ok());
    }
}
