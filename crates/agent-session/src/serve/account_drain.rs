//! Idle-boundary drain of queued provider account switches.
//!
//! A control whose applied binding is ready drains a queued next account
//! itself (`ApplyNext`). A control left without a ready binding (failed,
//! malformed, or still pending, for example after a daemon rebuild) refuses
//! that command, so a queued switch for such a session takes the
//! `PUT /sessions/{id}/account` path instead: under the same per-session switch
//! lock, the intent becomes an incarnation- and idle-fenced pending binding
//! that the control then binds.

use tokio::task::JoinSet;

mod claude;

pub(super) async fn run(state: std::sync::Arc<super::ServeState>) {
    tokio::join!(super::codex_control_loop(state.clone()), claude::run(state));
}

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
            let snapshot_context = context.clone();
            let snapshot_id = id.clone();
            let snapshot_launch = launch_id.clone();
            let snapshot = match tokio::task::spawn_blocking(move || {
                crate::auth_incident::recheck::codex_snapshot(
                    &snapshot_context,
                    &snapshot_id,
                    &snapshot_launch,
                )
            })
            .await
            {
                Ok(Ok(snapshot)) => snapshot,
                _ => return,
            };
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
            let bind_result = handle.bind_account(&account, revision).await;
            let failure = if bind_result.is_err() {
                Some("control_apply_unconfirmed")
            } else if let Some(snapshot) = snapshot {
                let settle_context = context.clone();
                let settle_id = id.clone();
                let settle_launch = launch_id.clone();
                let settle_account = account.clone();
                match tokio::task::spawn_blocking(move || {
                    crate::auth_incident::recheck::settle_codex(
                        &settle_context,
                        &settle_id,
                        &settle_launch,
                        &settle_account,
                        revision,
                        snapshot,
                    )
                })
                .await
                {
                    Ok(Ok(())) => None,
                    _ => Some("recheck_settlement_failed"),
                }
            } else {
                None
            };
            if let Some(reason) = failure {
                let _ = tokio::task::spawn_blocking(move || {
                    crate::codex_account::finish_binding(
                        &context,
                        &id,
                        &launch_id,
                        &account,
                        revision,
                        Err(reason),
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

    async fn queued_switch_recheck(succeed: bool, later_failure: bool, store_failure: bool) {
        use std::os::unix::fs::PermissionsExt;
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let broker = tmp.path().join("broker");
        std::fs::write(&broker, r##"#!/bin/sh
case "$1" in
list) printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","accounts":[{"account":"acct1"}]}' ;;
resolve) [ -f "$(dirname "$0")/valid" ] || exit 1
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"fixture-token","chatgpt_account_id":"fixture-account"}' ;;
*) exit 1 ;;
esac
"##).unwrap();
        std::fs::set_permissions(&broker, std::fs::Permissions::from_mode(0o700)).unwrap();
        if succeed {
            std::fs::write(tmp.path().join("valid"), "").unwrap();
        }
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            &serde_json::to_string(&[broker]).unwrap(),
        );
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let mut record = crate::codex_account::record_with_binding_value(json!({
            "schema_version": crate::codex_account::BINDING_SCHEMA_VERSION,
            "selected_account": "acct1",
            "revision": 7,
            "state": "failed",
            "failure_reason": "apply_failed",
            "updated_at": "2030-01-01T00:00:00Z"
        }));
        record.provider_resume = Some(serde_json::from_value(json!({
            "provider":"codex", "session_id":"fixture-thread", "captured_at":"2030-01-01T00:00:00Z",
            "capture_method":"fixture", "resume_args":[]
        })).unwrap());
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
                "provider_session_id": "fixture-thread",
                "kind": kind,
                "confidence": "authoritative"
            }))
            .unwrap();
            crate::activity::ingest_event(&context, &record.id, event).unwrap();
        }
        let observed = jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() - 121)
            .unwrap()
            .to_string();
        crate::auth_incident::observe(
            &context,
            &record,
            "auth-fault",
            crate::auth_incident::AuthSource::CodexExternalRefresh,
            crate::activity::Confidence::Authoritative,
            &observed,
        )
        .unwrap();
        crate::auth_incident::failover::tick(
            &context,
            &record,
            std::path::Path::new("unused"),
            jiff::Timestamp::now().as_second(),
        )
        .unwrap();
        assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
        let incident = crate::auth_incident::view(&context, &record).unwrap();
        assert_eq!(
            serde_json::to_value(incident).unwrap()["failover"]["state"],
            "blocked_no_credential"
        );
        crate::session_account::switch(
            &context,
            &record.id,
            "acct1",
            Some(RUNTIME),
            std::path::Path::new("unused"),
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
            assert_eq!(
                crate::codex_account::resolve_account(&account, false).is_ok(),
                succeed
            );
            if later_failure {
                let current = crate::load_session_record(&control_context, &control_id).unwrap();
                let at = jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() - 121)
                    .unwrap()
                    .to_string();
                crate::auth_incident::observe(
                    &control_context,
                    &current,
                    "later-auth-fault",
                    crate::auth_incident::AuthSource::CodexExternalRefresh,
                    crate::activity::Confidence::Authoritative,
                    &at,
                )
                .unwrap();
                crate::auth_incident::failover::tick(
                    &control_context,
                    &current,
                    std::path::Path::new("unused"),
                    jiff::Timestamp::now().as_second(),
                )
                .unwrap();
            }
            let view = crate::codex_account::finish_binding(
                &control_context,
                &control_id,
                RUNTIME,
                &account,
                revision,
                if succeed {
                    Ok(())
                } else {
                    Err("broker_resolve_rejected")
                },
            );
            if store_failure {
                let path =
                    crate::session_dir(&control_context, &control_id).join("auth-incidents.json");
                std::fs::rename(&path, path.with_extension("backup")).unwrap();
                std::fs::create_dir(&path).unwrap();
            }
            let result = if succeed {
                view.map_err(|err| err.code().to_string())
            } else {
                Err("broker_resolve_rejected".into())
            };
            let _ = response.send(result);
            (account, revision)
        });
        repair_blocked_codex_accounts(
            &context,
            &AccountSwitchRegistry::default(),
            vec![(record.id.clone(), RUNTIME.to_string(), handle)],
        )
        .await;

        assert_eq!(control.await.unwrap(), ("acct1".to_string(), 8));
        if store_failure {
            let path = crate::session_dir(&context, &record.id).join("auth-incidents.json");
            std::fs::remove_dir(&path).unwrap();
            std::fs::rename(path.with_extension("backup"), path).unwrap();
            let failed = crate::load_session_record(&context, &record.id).unwrap();
            let view = crate::codex_account::view_for_record(&failed);
            assert_eq!(view.state, "failed");
            assert_eq!(
                view.failure_reason.as_deref(),
                Some("recheck_settlement_failed")
            );
            assert!(
                crate::auth_incident::recovery::ensure_input_allowed(&context, &failed).is_err()
            );
            crate::session_account::switch(
                &context,
                &record.id,
                "acct1",
                Some(RUNTIME),
                std::path::Path::new("unused"),
            )
            .unwrap();
            let (handle, mut commands) = control_channel();
            let control_context = context.clone();
            let control_id = record.id.clone();
            let retry = tokio::spawn(async move {
                let Some(ControlCommand::BindAccount {
                    account,
                    revision,
                    response,
                }) = commands.recv().await
                else {
                    panic!("retry did not reach control")
                };
                let view = crate::codex_account::finish_binding(
                    &control_context,
                    &control_id,
                    RUNTIME,
                    &account,
                    revision,
                    Ok(()),
                );
                let _ = response.send(view.map_err(|error| error.code().to_string()));
            });
            repair_blocked_codex_accounts(
                &context,
                &AccountSwitchRegistry::default(),
                vec![(record.id.clone(), RUNTIME.to_string(), handle)],
            )
            .await;
            retry.await.unwrap();
        }
        let repaired = crate::load_session_record(&context, &record.id).unwrap();
        let view = crate::codex_account::view_for_record(&repaired);
        if succeed && !later_failure {
            assert_eq!(view.state, "bound");
            assert_eq!(view.applied_runtime_id.as_deref(), Some(RUNTIME));
            assert!(view.next.is_none());
            assert!(crate::codex_account::ensure_input_allowed(&repaired).is_ok());
            assert_eq!(
                crate::auth_incident::recovery::ensure_input_allowed(&context, &repaired).is_ok(),
                !later_failure
            );
        } else {
            assert_eq!(view.state, "failed");
            assert_eq!(
                view.failure_reason.as_deref(),
                Some(if later_failure {
                    "recheck_settlement_failed"
                } else {
                    "broker_resolve_rejected"
                })
            );
            assert!(
                crate::auth_incident::recovery::ensure_input_allowed(&context, &repaired).is_err()
            );
        }
    }

    #[tokio::test]
    async fn queued_same_account_recheck_settles_after_confirmed_bind() {
        queued_switch_recheck(true, false, false).await;
    }

    #[tokio::test]
    async fn queued_same_account_recheck_refusal_keeps_quarantine() {
        queued_switch_recheck(false, false, false).await;
    }
    #[tokio::test]
    async fn queued_same_account_recheck_does_not_settle_a_later_failure() {
        queued_switch_recheck(true, true, false).await;
    }
    #[tokio::test]
    async fn queued_same_account_recheck_retries_after_incident_store_failure() {
        queued_switch_recheck(true, false, true).await;
    }
}
