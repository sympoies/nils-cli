use super::*;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::{assert_eq, assert_ne};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

fn capability_probe_test_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn external_auth_login_request_matches_codex_0_144_1_contract() {
    assert_eq!(
        external_auth_login_request(7, "fixture-access-token", "workspace-fixture", Some("pro"),),
        json!({
            "id": 7,
            "method": "account/login/start",
            "params": {
                "type": "chatgptAuthTokens",
                "accessToken": "fixture-access-token",
                "chatgptAccountId": "workspace-fixture",
                "chatgptPlanType": "pro"
            }
        })
    );
}

#[test]
fn external_auth_refresh_response_preserves_json_rpc_id() {
    assert_eq!(
        external_auth_refresh_response(
            json!("refresh-request"),
            "refreshed-fixture-token",
            "workspace-refreshed",
            None,
        ),
        json!({
            "id": "refresh-request",
            "result": {
                "accessToken": "refreshed-fixture-token",
                "chatgptAccountId": "workspace-refreshed",
                "chatgptPlanType": null
            }
        })
    );
}

#[test]
fn tui_busy_response_preserves_stable_contract_for_every_rejection() {
    for (rejection, reason) in [
        (
            TuiMutationRejection::TurnAlreadyPending,
            "turn_already_pending",
        ),
        (
            TuiMutationRejection::AccountMutationForbidden,
            "account_mutation_forbidden",
        ),
        (TuiMutationRejection::AccountNotReady, "account_not_ready"),
        (
            TuiMutationRejection::TurnRequestInvalid,
            "turn_request_invalid",
        ),
        (
            TuiMutationRejection::TurnGateOpenFailed,
            "turn_gate_open_failed",
        ),
        (TuiMutationRejection::TurnGateBusy, "turn_gate_busy"),
        (
            TuiMutationRejection::ManualMarkerThreadMismatch,
            "manual_marker_thread_mismatch",
        ),
        (
            TuiMutationRejection::ManualMarkerReplaced,
            "manual_marker_replaced",
        ),
        (
            TuiMutationRejection::ManualMarkerInvalid,
            "manual_marker_invalid",
        ),
        (TuiMutationRejection::ManualAckFailed, "manual_ack_failed"),
        (
            TuiMutationRejection::RuntimeIdentityMissing,
            "runtime_identity_missing",
        ),
        (
            TuiMutationRejection::ManualCancellationBusy,
            "manual_cancellation_busy",
        ),
        (TuiMutationRejection::RuntimeChanged, "runtime_changed"),
        (
            TuiMutationRejection::AccountAuthorityUnavailable,
            "account_authority_unavailable",
        ),
    ] {
        let Message::Text(text) = tui_busy_response(&json!(7), rejection) else {
            panic!("busy response must be text");
        };
        let response: Value = serde_json::from_str(text.as_str()).unwrap();
        assert_eq!(response["id"], 7);
        assert_eq!(response["error"]["code"], -32001);
        assert_eq!(
            response["error"]["message"],
            "agent-session state is busy; retry the request"
        );
        assert_eq!(response["error"]["data"]["reason"], reason);
    }
}

#[test]
fn steering_request_fences_the_exact_active_turn() {
    assert_eq!(
        steering_request(9, "thread-fixture", "turn-fixture", "mailbox checkpoint"),
        json!({
            "id": 9,
            "method": "turn/steer",
            "params": {
                "threadId": "thread-fixture",
                "expectedTurnId": "turn-fixture",
                "input": [{
                    "type": "text",
                    "text": "mailbox checkpoint",
                    "text_elements": []
                }]
            }
        })
    );
}

#[test]
fn latest_turn_recovery_requests_metadata_only_and_accepts_only_in_progress() {
    assert_eq!(
        latest_turn_request(10, "thread-fixture"),
        json!({
            "id": 10,
            "method": "thread/turns/list",
            "params": {
                "threadId": "thread-fixture",
                "limit": 1,
                "sortDirection": "desc",
                "itemsView": "notLoaded"
            }
        })
    );
    assert_eq!(
        latest_in_progress_turn_id(&json!({
            "data": [{"id": "raw-active-turn", "status": "inProgress", "items": []}],
            "nextCursor": null
        })),
        Some("raw-active-turn")
    );
    assert_eq!(
        latest_in_progress_turn_id(&json!({
            "data": [{"id": "raw-completed-turn", "status": "completed", "items": []}],
            "nextCursor": null
        })),
        None
    );
    assert_eq!(
        latest_turn_state(&json!({
            "data": [{"id": "raw-active-turn", "status": "inProgress", "items": []}],
            "nextCursor": null
        })),
        Some(LatestTurnState::InProgress("raw-active-turn"))
    );
    assert_eq!(
        latest_turn_state(&json!({
            "data": [{"id": "raw-completed-turn", "status": "completed", "items": []}],
            "nextCursor": null
        })),
        Some(LatestTurnState::Idle(Some("raw-completed-turn")))
    );
    assert_eq!(
        latest_turn_state(&json!({"data": []})),
        Some(LatestTurnState::Idle(None))
    );
    assert_eq!(
        latest_turn_state(&json!({
            "data": [{"id": "raw-future-turn", "status": "future", "items": []}]
        })),
        None
    );
    assert_eq!(
        latest_turn_state(&json!({
            "data": [{"id": "", "status": "completed", "items": []}]
        })),
        None
    );
}

struct PendingMessageSink;

impl futures_util::Sink<Message> for PendingMessageSink {
    type Error = tokio_tungstenite::tungstenite::Error;

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Pending
    }

    fn start_send(self: std::pin::Pin<&mut Self>, _item: Message) -> Result<(), Self::Error> {
        unreachable!("pending sink never becomes ready")
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Pending
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl futures_util::Stream for PendingMessageSink {
    type Item = Result<Message, tokio_tungstenite::tungstenite::Error>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Pending
    }
}

#[derive(Default)]
struct RecordingMessageSink {
    messages: Vec<Message>,
}

impl futures_util::Sink<Message> for RecordingMessageSink {
    type Error = tokio_tungstenite::tungstenite::Error;

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn start_send(mut self: std::pin::Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        self.messages.push(item);
        Ok(())
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
}

struct FailingMessageSink;

impl futures_util::Sink<Message> for FailingMessageSink {
    type Error = tokio_tungstenite::tungstenite::Error;

    fn poll_ready(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn start_send(self: std::pin::Pin<&mut Self>, _item: Message) -> Result<(), Self::Error> {
        Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
}

/// Drains a projection's background work before its fixture is removed.
///
/// `Drop for ProxyProjection` only calls `abort()`, which schedules
/// cancellation instead of waiting, and it does not touch the fail-close
/// task at all. Either one can still be inside a write when the test body
/// returns; because those writes run `create_dir_all` first, they re-create
/// the `TempDir` that teardown just removed — one leaked directory per run,
/// invisible because the test itself passes.
///
/// This mirrors the shutdown a live proxy performs, so tests observe the
/// same ordering production does.
async fn settle_projection(projection: &mut ProxyProjection) {
    projection.sender = None;
    if let Some(task) = projection.task.take() {
        let _ = task.await;
    }
    if projection.has_fail_close_task() {
        projection.finish_fail_close().await;
    }
}

fn record_with_runtime(id: &str, socket: &Path) -> SessionRecord {
    SessionRecord {
        schema_version: crate::SESSION_DOCUMENT_VERSION.to_string(),
        id: id.to_string(),
        agent: "codex".to_string(),
        mode: "interactive".to_string(),
        coordination_mode: crate::cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        title_revision: 0,
        cwd: "/repo".to_string(),
        tmux_session: format!("hs-{id}"),
        prompt_file: None,
        log_file: None,
        created_at: "2030-01-01T00:00:00Z".to_string(),
        updated_at: "2030-01-01T00:00:00Z".to_string(),
        provider_resume: None,
        runtime: Some(crate::RuntimeInfo {
            kind: RUNTIME_KIND.to_string(),
            tmux_session: format!("hs-{id}"),
            generation: 1,
            started_at: "2030-01-01T00:00:00Z".to_string(),
            launch_id: format!("runtime-{id}"),
            extra: BTreeMap::from([
                (
                    ATTENTION_AUTHORITY_KEY.to_string(),
                    json!(ATTENTION_AUTHORITY_PROTOCOL),
                ),
                (PROTOCOL_KEY.to_string(), json!(PROTOCOL_VERSION)),
                (SOCKET_KEY.to_string(), json!(display_path(socket))),
                (
                    PROXY_KEY.to_string(),
                    json!(display_path(&socket.with_extension("proxy"))),
                ),
                (
                    THREAD_HANDOFF_KEY.to_string(),
                    json!(display_path(&socket.with_extension("thread"))),
                ),
                (
                    THREAD_ATTACHED_KEY.to_string(),
                    json!(display_path(&socket.with_extension("attached"))),
                ),
            ]),
        }),
        public_metadata: None,
        agent_args: Vec::new(),
        agent_bin: None,
        extra: BTreeMap::new(),
        lineage: None,
        work: None,
        lineage_adoption: None,
        role: None,
        resume_sidecar_extra: BTreeMap::new(),
    }
}

async fn wait_for_activity(
    context: &CliContext,
    id: &str,
    predicate: impl Fn(&crate::activity::TurnState) -> bool,
) -> crate::activity::TurnState {
    for _ in 0..100 {
        let state = crate::activity::activity_status(context, id)
            .unwrap()
            .turn_state;
        if predicate(&state) {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("activity did not converge before the bounded deadline")
}

fn write_create_bootstrap_marker(record: &SessionRecord) {
    let runtime = record.runtime.as_ref().unwrap();
    write_private_file(
        thread_handoff_path(record).unwrap(),
        runtime.launch_id.as_bytes(),
    )
    .unwrap();
}

fn write_manual_input_marker(
    context: &CliContext,
    record: &SessionRecord,
    launch_id: &str,
    expires_at_epoch_ms: u64,
) -> ManualInputMarker {
    let path = manual_input_section_path(context, record);
    let token = uuid::Uuid::new_v4().to_string();
    write_private_file(
        &path,
        &serde_json::to_vec(&RuntimeProcessMarker {
            schema_version: MANUAL_INPUT_SECTION_VERSION.to_string(),
            launch_id: launch_id.to_string(),
            token: token.clone(),
            owner_pid: std::process::id(),
            expires_at_epoch_ms,
            conversation_capability: None,
        })
        .unwrap(),
    )
    .unwrap();
    let file = fs::File::open(&path).unwrap();
    // SAFETY: test owns this valid descriptor.
    assert_eq!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH) }, 0);
    open_manual_input_gate_file(&manual_input_gate_path(context, record)).unwrap();
    let ack_path = manual_input_ack_path(record).unwrap();
    let _ = fs::remove_file(&ack_path);
    let ack_socket = UnixDatagram::bind(&ack_path).unwrap();
    ack_socket
        .set_read_timeout(Some(MANUAL_INPUT_ACK_TIMEOUT))
        .unwrap();
    ManualInputMarker {
        path,
        token,
        _owner_file: file,
        gate_path: Some(manual_input_gate_path(context, record)),
        ack_path: Some(ack_path),
        ack_socket: Some(ack_socket),
        cleanup_on_drop: true,
    }
}

#[test]
fn create_bootstrap_guard_owns_the_marker_lifetime() {
    let tmp = tempfile::TempDir::new().unwrap();
    let record = record_with_runtime("guard", &tmp.path().join("guard.sock"));
    let marker = thread_handoff_path(&record).unwrap().to_path_buf();
    let guard = begin_create_bootstrap(&record).unwrap().unwrap();
    assert!(create_bootstrap_is_live(&record));
    drop(guard);
    assert!(!marker.exists());
    assert!(!create_bootstrap_is_live(&record));
}

#[test]
fn forced_runtime_still_requires_the_installed_capability() {
    let _probe_guard = capability_probe_test_guard();
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let runtime_dir = tmp.path().join("run");
    fs::create_dir(&runtime_dir).unwrap();
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let _runtime_dir = EnvGuard::set(&lock, "XDG_RUNTIME_DIR", runtime_dir.to_str().unwrap());
    let _preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "app-server");
    let agent = tmp.path().join("codex");
    fs::write(&agent, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("forced-probe", &runtime_dir.join("placeholder"));
    record.runtime.as_mut().unwrap().kind = "tmux".to_string();
    record.runtime.as_mut().unwrap().extra.clear();

    let err = configure_runtime(&context, &agent, &mut record, true).unwrap_err();
    assert_eq!(err.code(), "codex-app-server-capability-unavailable");
    assert_eq!(record.runtime.unwrap().kind, "tmux");
}

#[test]
fn selected_account_requires_capability_even_with_auto_preference() {
    let _probe_guard = capability_probe_test_guard();
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let runtime_dir = tmp.path().join("run");
    fs::create_dir(&runtime_dir).unwrap();
    let _runtime_dir = EnvGuard::set(&lock, "XDG_RUNTIME_DIR", runtime_dir.to_str().unwrap());
    let _preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "auto");
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let agent = tmp.path().join("codex");
    fs::write(&agent, "#!/bin/sh\nprintf '%s\\n' 'codex-cli 0.145.0'\n").unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("selected-auto", &runtime_dir.join("placeholder"));
    record.runtime.as_mut().unwrap().kind = "tmux".to_string();
    record.runtime.as_mut().unwrap().extra.clear();
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();

    let err = configure_runtime(&context, &agent, &mut record, true).unwrap_err();
    assert_eq!(err.code(), "codex-app-server-capability-unavailable");
    assert_eq!(record.runtime.unwrap().kind, "tmux");
}

#[test]
fn capability_probe_requires_0_145_or_newer_with_unix_transport() {
    let _probe_guard = capability_probe_test_guard();
    let tmp = tempfile::TempDir::new().unwrap();
    for (name, version, help, expected) in [
        (
            "supported",
            "codex-cli 0.144.1",
            "  --listen <URL>  Supported values: stdio://, unix://PATH",
            false,
        ),
        (
            "supported-0.144.3",
            "codex-cli 0.144.3",
            "  --listen <URL>  Supported values: stdio://, unix://PATH",
            false,
        ),
        (
            "old",
            "codex-cli 0.143.9",
            "  --listen <URL>  Supported values: stdio://, unix://PATH",
            false,
        ),
        (
            "newer-patch",
            "codex-cli 0.144.5",
            "  --listen <URL>  Supported values: stdio://, unix://PATH",
            false,
        ),
        (
            "newer-minor",
            "codex-cli 0.145.0",
            "  --listen <URL>  Supported values: stdio://, unix://PATH",
            true,
        ),
        (
            "extra-component",
            "codex-cli 0.144.1.1",
            "  --listen <URL>  Supported values: stdio://, unix://PATH",
            false,
        ),
        (
            "prerelease",
            "codex-cli 0.144.1-beta",
            "  --listen <URL>  Supported values: stdio://, unix://PATH",
            false,
        ),
        (
            "build-metadata",
            "codex-cli 0.144.1+custom",
            "  --listen <URL>  Supported values: stdio://, unix://PATH",
            false,
        ),
        (
            "unrelated-token",
            "wrapper release 0.144.1",
            "  --listen <URL>  Supported values: stdio://, unix://PATH",
            false,
        ),
        (
            "no-unix",
            "codex-cli 0.144.1",
            "  --listen <URL>  Supported values: stdio://",
            false,
        ),
    ] {
        let path = tmp.path().join(name);
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' '{version}'; exit 0; fi\nif [ \"$1\" = app-server ] && [ \"$2\" = --help ]; then printf '%s\\n' '{help}'; exit 0; fi\nexit 1\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(app_server_capabilities(&path).transport, expected, "{name}");
    }
}

#[test]
fn account_binding_readiness_reports_bounded_safe_reasons() {
    let _probe_guard = capability_probe_test_guard();
    let tmp = tempfile::TempDir::new().unwrap();
    let old = tmp.path().join("old");
    fs::write(&old, "#!/bin/sh\nprintf '%s\\n' 'codex-cli 0.143.9'\n").unwrap();
    fs::set_permissions(&old, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        account_binding_readiness(&old, tmp.path()),
        CodexAccountReadiness {
            schema_version: CODEX_ACCOUNT_READINESS_SCHEMA_VERSION,
            supported: false,
            provider_version: Some("0.143.9".to_string()),
            reason_code: Some("codex-version-too-old"),
        }
    );

    let missing = tmp.path().join("missing");
    assert_eq!(
        account_binding_readiness(&missing, tmp.path()),
        CodexAccountReadiness {
            schema_version: CODEX_ACCOUNT_READINESS_SCHEMA_VERSION,
            supported: false,
            provider_version: None,
            reason_code: Some("codex-unavailable"),
        }
    );
}

#[test]
fn configured_0_145_app_server_runtime_keeps_hook_attention_authority() {
    let _probe_guard = capability_probe_test_guard();
    let lock = GlobalStateLock::new();
    let tmp = tempfile::Builder::new()
        .prefix("agent-session-authority-")
        .tempdir_in("/tmp")
        .unwrap();
    let runtime_dir = tmp.path().join("run");
    let home = tmp.path().join("home");
    fs::create_dir(&runtime_dir).unwrap();
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let _runtime_dir = EnvGuard::set(&lock, "XDG_RUNTIME_DIR", runtime_dir.to_str().unwrap());
    let _home = EnvGuard::set(&lock, "HOME", home.to_str().unwrap());
    let _preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "app-server");
    fs::write(
        home.join(".codex/hooks.json"),
        serde_json::to_vec_pretty(&json!({
            "hooks": {
                "PermissionRequest": [{
                    "hooks": [{
                        "type": "command",
                        "command": "sh -c 'if [ \"${AGENT_SESSION_ATTENTION_AUTHORITY:-hook}\" = protocol ]; then exit 0; fi; exec agent-session activity hook --agent codex'",
                        "timeout": 5
                    }]
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let agent = tmp.path().join("codex");
    fs::write(
        &agent,
        "#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'codex-cli 0.145.0'; exit 0; fi\nif [ \"$1\" = app-server ] && [ \"$2\" = --help ]; then printf '%s\\n' '  --listen <URL>  Supported values: stdio://, unix://PATH'; exit 0; fi\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("authority", &runtime_dir.join("placeholder"));
    record.runtime.as_mut().unwrap().kind = "tmux".to_string();
    record.runtime.as_mut().unwrap().extra = BTreeMap::from([(
        ATTENTION_AUTHORITY_KEY.to_string(),
        json!(ATTENTION_AUTHORITY_HOOK),
    )]);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();

    configure_runtime(&context, &agent, &mut record, true).unwrap();
    assert_eq!(record.runtime.as_ref().unwrap().kind, RUNTIME_KIND);
    assert_eq!(attention_authority(&record), ATTENTION_AUTHORITY_HOOK);
    assert!(managed_account_handoff_supported(&record));
    assert_eq!(
        record.runtime.as_ref().unwrap().extra[MANAGED_ACCOUNT_HANDOFF_CAPABILITY_KEY],
        MANAGED_ACCOUNT_HANDOFF_CAPABILITY
    );
    assert_eq!(
        record
            .runtime
            .as_ref()
            .unwrap()
            .extra
            .get(ATTENTION_AUTHORITY_KEY),
        Some(&json!(ATTENTION_AUTHORITY_HOOK))
    );
}

#[test]
fn configured_0_145_runtime_stays_hook_authority_with_unguarded_permission_hook() {
    let _probe_guard = capability_probe_test_guard();
    let lock = GlobalStateLock::new();
    let tmp = tempfile::Builder::new()
        .prefix("agent-session-unguarded-")
        .tempdir_in("/tmp")
        .unwrap();
    let runtime_dir = tmp.path().join("run");
    let home = tmp.path().join("home");
    fs::create_dir(&runtime_dir).unwrap();
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let _runtime_dir = EnvGuard::set(&lock, "XDG_RUNTIME_DIR", runtime_dir.to_str().unwrap());
    let _home = EnvGuard::set(&lock, "HOME", home.to_str().unwrap());
    let _preference = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "app-server");
    fs::write(
        home.join(".codex/hooks.json"),
        serde_json::to_vec_pretty(&json!({
            "hooks": {
                "PermissionRequest": [{
                    "hooks": [
                        {
                            "type": "command",
                            "command": "sh -c 'if [ \"${AGENT_SESSION_ATTENTION_AUTHORITY:-hook}\" = protocol ]; then exit 0; fi; exec agent-session activity hook --agent codex'",
                            "timeout": 5
                        },
                        {
                            "type": "command",
                            "command": "agent-session activity hook --agent codex",
                            "timeout": 5
                        }
                    ]
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let agent = tmp.path().join("codex");
    fs::write(
        &agent,
        "#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'codex-cli 0.145.0'; exit 0; fi\nif [ \"$1\" = app-server ] && [ \"$2\" = --help ]; then printf '%s\\n' '  --listen <URL>  Supported values: stdio://, unix://PATH'; exit 0; fi\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o700)).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("unguarded-hook", &runtime_dir.join("placeholder"));
    record.runtime.as_mut().unwrap().kind = "tmux".to_string();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();

    configure_runtime(&context, &agent, &mut record, true).unwrap();
    assert_eq!(record.runtime.as_ref().unwrap().kind, RUNTIME_KIND);
    assert_eq!(attention_authority(&record), ATTENTION_AUTHORITY_HOOK);
}

#[test]
fn transport_only_app_server_runtime_keeps_hook_attention_authority() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::Builder::new()
        .prefix("agent-session-transport-")
        .tempdir_in("/tmp")
        .unwrap();
    let runtime_dir = tmp.path().join("run");
    fs::create_dir(&runtime_dir).unwrap();
    fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let _runtime_dir = EnvGuard::set(&lock, "XDG_RUNTIME_DIR", runtime_dir.to_str().unwrap());
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("transport-only", &runtime_dir.join("placeholder"));
    record.runtime.as_mut().unwrap().kind = "tmux".to_string();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();

    configure_runtime_with_capabilities(
        &context,
        &mut record,
        true,
        true,
        AppServerCapabilities {
            transport: true,
            exact_attention: false,
            source_guard: true,
        },
    )
    .unwrap();

    assert_eq!(record.runtime.as_ref().unwrap().kind, RUNTIME_KIND);
    assert_eq!(attention_authority(&record), ATTENTION_AUTHORITY_HOOK);
}

#[test]
fn capability_probe_tolerates_cold_start_latency() {
    let _probe_guard = capability_probe_test_guard();
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("cold-codex");
    fs::write(
        &path,
        r#"#!/bin/sh
sleep 0.35
if [ "$1" = --version ]; then
  printf '%s\n' 'codex-cli 0.145.0'
  exit 0
fi
if [ "$1" = app-server ] && [ "$2" = --help ]; then
  printf '%s\n' '  --listen <URL>  Supported values: stdio://, unix://PATH'
  exit 0
fi
exit 1
"#,
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();

    assert!(app_server_capabilities(&path).transport);
}

#[test]
fn capability_probe_timeout_kills_and_reaps_the_process_group() {
    let _probe_guard = capability_probe_test_guard();
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("hung-codex");
    let pid_file = tmp.path().join("descendant.pid");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nsleep 60 &\nprintf '%s' \"$!\" > {}\nwait\n",
            shell_words::quote(&pid_file.to_string_lossy())
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();

    let started = Instant::now();
    assert!(
        bounded_command_output_with_timeout(&path, &["--version"], Duration::from_millis(500))
            .is_none()
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "injected probe timeout must remain bounded"
    );

    let pid = fs::read_to_string(&pid_file)
        .unwrap()
        .parse::<libc::pid_t>()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[test]
fn capability_probe_timeout_kills_descendant_after_leader_exits() {
    let _probe_guard = capability_probe_test_guard();
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("exited-codex");
    let pid_file = tmp.path().join("descendant.pid");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nsleep 60 &\nprintf '%s' \"$!\" > {}\nexit 0\n",
            shell_words::quote(&pid_file.to_string_lossy())
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();

    let started = Instant::now();
    assert!(
        bounded_command_output_with_timeout(&path, &["--version"], Duration::from_millis(500))
            .is_none()
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "output collection must remain under the injected probe timeout"
    );

    let pid = fs::read_to_string(&pid_file)
        .unwrap()
        .parse::<libc::pid_t>()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[test]
fn launch_applies_session_config_to_server_without_changing_tui_arguments() {
    let tmp = tempfile::TempDir::new().unwrap();
    let helper = tmp.path().join("fake-provider");
    fs::write(
        &helper,
        r#"#!/bin/sh
if [ "$1" = app-server ]; then
  printf '%s\n' "$@" > "$FAKE_SERVER_ARGS"
  exec sleep 60
fi
case " $* " in
  *" codex-app-server-proxy "*)
    : > "$FAKE_PROXY_READY"
    exec sleep 60
    ;;
esac
printf '%s\n' "$@" > "$FAKE_TUI_ARGS"
"#,
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let overrides = [
        "-c",
        "model_auto_compact_token_limit=40000",
        "--config=model_context_window=100000",
        "-cmodel_auto_compact_token_limit=45000",
        "--config",
        "model_auto_compact_token_limit=40000",
        "--enable",
        "example_feature",
        "--disable=other_feature",
        "-c",
        "model_providers.example.name=\"a'b; $(touch injected); `id`\"",
    ];
    // A fresh default session must not inherit its predecessor's overrides;
    // recreating a configured runtime must apply them again.
    for (index, config) in [overrides.as_slice(), &[], overrides.as_slice()]
        .into_iter()
        .enumerate()
    {
        let state = tmp.path().join(format!("state-{index}"));
        let id = "configured-session";
        fs::create_dir_all(state.join("sessions").join(id)).unwrap();
        let socket = tmp.path().join(format!("server-{index}.sock"));
        let proxy = socket.with_extension("proxy");
        let server_args = state.join("server.args");
        let tui_args = state.join("tui.args");
        let proxy_ready = state.join("proxy.ready");
        let stop = Arc::new(AtomicBool::new(false));
        let bind = |marker: PathBuf, path: PathBuf| {
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !marker.is_file()
                    && !stop.load(Ordering::Relaxed)
                    && Instant::now() < deadline
                {
                    thread::sleep(Duration::from_millis(5));
                }
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let _listener = std::os::unix::net::UnixListener::bind(path).unwrap();
                while !stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(5));
                }
            })
        };
        let server = bind(server_args.clone(), socket.clone());
        let proxy_server = bind(proxy_ready.clone(), proxy.clone());
        let args = config
            .iter()
            .copied()
            .chain([
                "--model",
                "example-model",
                "--",
                "--config=prompt-is-literal",
            ])
            .map(str::to_string)
            .collect::<Vec<_>>();
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(launch_script(&args))
            .arg("launch-config-test")
            .arg(&socket)
            .arg(&proxy)
            .arg(socket.with_extension("thread"))
            .arg(socket.with_extension("attached"))
            .arg(&helper)
            .arg(&state)
            .arg(id)
            .arg(&helper)
            .arg(tmp.path())
            .args(&args)
            .env("FAKE_SERVER_ARGS", &server_args)
            .env("FAKE_TUI_ARGS", &tui_args)
            .env("FAKE_PROXY_READY", &proxy_ready)
            .current_dir(tmp.path());
        let output = crate::run_output_with_timeout(command, Duration::from_secs(10));
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap();
        proxy_server.join().unwrap();
        assert!(output.unwrap().status.success());
        let captured = fs::read_to_string(server_args).unwrap();
        let actual = captured.lines().collect::<Vec<_>>();
        assert_eq!(
            &actual[3..],
            config,
            "server must receive literal ordered overrides"
        );
        let mut expected_tui = vec![
            "-c".to_string(),
            "check_for_update_on_startup=false".to_string(),
            "--remote".to_string(),
            format!("unix://{}", proxy.display()),
        ];
        expected_tui.extend(args);
        assert_eq!(
            fs::read_to_string(tui_args)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            expected_tui
        );
        assert!(!tmp.path().join("injected").exists());
    }
}

#[test]
fn launch_routes_the_visible_tui_through_the_private_proxy() {
    let script = launch_script(&[]);
    assert!(script.contains("codex-app-server-proxy"));
    assert!(script.contains("--remote \"unix://$proxy\""));
    assert!(!script.contains("--remote \"unix://$socket\""));
    assert!(!script.contains("thread/shellCommand"));
    assert!(script.contains(".startup-stage"));
    assert!(script.contains(".startup-failure"));
    assert!(script.contains(".startup-diagnostic.log"));
    assert!(script.contains(".runtime-exit-status"));
    assert!(script.contains("write_startup_marker \"$runtime_exit_status\" \"$status\""));
    let hold = script
        .find("exec 9>\"$startup_diagnostic_pipe\"")
        .expect("diagnostic pipe hold");
    let marker = script
        .find("write_startup_marker \"$runtime_exit_status\" \"$status\"")
        .expect("runtime exit marker");
    let provider_child_close = script
        .find("\"$@\" 9>&- 2>\"$provider_stderr_pipe\"")
        .expect("provider child closes diagnostic hold descriptor");
    let release = script[marker..]
        .find("exec 9>&-")
        .map(|offset| marker + offset)
        .expect("diagnostic pipe release");
    assert!(hold < provider_child_close && provider_child_close < marker && marker < release);
    assert!(script.contains("runtime-helper-unavailable"));
    assert!(script.contains("provider-client-exited"));
    assert!(script.contains("!= initial_connection"));
    assert!(script.contains("collect_startup_diagnostic"));
    assert!(script.contains("tail -c 16384"));
    assert!(script.contains("mkfifo \"$startup_diagnostic_pipe\""));
    assert!(script.contains("tee \"$startup_diagnostic_pipe\""));
    assert!(script.contains(
        "(umask 077; exec \"$proxy_bin\" --state-dir \"$state_dir\" codex-app-server-proxy"
    ));
    assert!(script.contains("kill -9 \"$provider_stderr_pid\""));
    let final_tee_kill = script
        .rfind("kill -9 \"$provider_stderr_pid\"")
        .expect("final tee escalation");
    let claim_before_wait = &script[final_tee_kill..];
    let clear = claim_before_wait
        .find("provider_stderr_pid=")
        .expect("tee pid ownership clear");
    let wait = claim_before_wait
        .find("wait \"$owned_pid\"")
        .expect("tee wait through claimed pid");
    assert!(clear < wait);
    assert!(!script.contains(">>\"$startup_diagnostic\""));
    let cleanup_lines = script
        .lines()
        .filter(|line| line.trim_start().starts_with("rm -f --") && line.contains("$socket"))
        .collect::<Vec<_>>();
    assert!(!cleanup_lines[0].contains("$handoff"));
    assert!(cleanup_lines[1].contains("$handoff"));
}

#[test]
fn launch_script_retains_failed_provider_diagnostics_without_leaking_the_hold_descriptor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let helper = tmp.path().join("fake-codex-runtime");
    fs::write(
        &helper,
        r#"#!/bin/sh
if [ "$1" = app-server ]; then
  : > "$FAKE_APP_SERVER_REQUEST"
  exec sleep 60
fi
case " $* " in
  *" codex-app-server-proxy "*)
    : > "$FAKE_PROXY_REQUEST"
    exec sleep 60
    ;;
esac
i=0
while [ "$(cat "$FAKE_PROVIDER_STAGE" 2>/dev/null)" != initial_connection ]; do
  i=$((i + 1))
  [ "$i" -lt 200 ] || exit 99
  sleep 0.01
done
printf '%s' "$FAKE_PROVIDER_STDERR" >&2
if [ -n "$FAKE_LAUNCHER_PID_FILE" ]; then
  printf '%s' "$PPID" > "$FAKE_LAUNCHER_PID_FILE"
fi
exit "$FAKE_PROVIDER_EXIT"
"#,
    )
    .unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    let tee = tmp.path().join("tee");
    fs::write(&tee, "#!/bin/sh\ntrap '' TERM\nexec /usr/bin/tee \"$@\"\n").unwrap();
    fs::set_permissions(&tee, fs::Permissions::from_mode(0o700)).unwrap();
    let mut test_paths = vec![tmp.path().to_path_buf()];
    test_paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let test_path = env::join_paths(test_paths).unwrap();
    let state_dir = tmp.path().join("state");
    let cwd = tmp.path().join("cwd");
    fs::create_dir(&cwd).unwrap();

    let run_case = |id: &str, status: i32, stderr: &str, descendant: bool, interrupt: bool| {
        let session_dir = state_dir.join("sessions").join(id);
        fs::create_dir_all(&session_dir).unwrap();
        let socket = tmp.path().join(format!("{id}-app.sock"));
        let proxy = tmp.path().join(format!("{id}-proxy.sock"));
        let handoff = tmp.path().join(format!("{id}-thread"));
        let attached = tmp.path().join(format!("{id}-attached"));
        let stage = session_dir.join(".startup-stage");
        let runtime_exit_status = session_dir.join(".runtime-exit-status");
        let provider_stderr_pipe = session_dir.join(".provider-stderr.pipe");
        let launcher_pid_file = session_dir.join("launcher.pid");
        let app_server_request = session_dir.join("app-server.request");
        let proxy_request = session_dir.join("proxy.request");
        let stop = Arc::new(AtomicBool::new(false));
        let descriptor_holder_ready = descendant.then(|| Arc::new(AtomicBool::new(false)));
        let bind_socket = |request: PathBuf,
                           socket: PathBuf,
                           stage: Option<PathBuf>,
                           stage_barrier: Option<Arc<AtomicBool>>,
                           stop: Arc<AtomicBool>| {
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(3);
                while !request.exists()
                    && !stop.load(Ordering::Relaxed)
                    && Instant::now() < deadline
                {
                    thread::sleep(Duration::from_millis(5));
                }
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
                if let Some(stage) = stage {
                    while fs::read_to_string(&stage)
                        .ok()
                        .is_none_or(|value| value.trim() != "provider_client")
                        && !stop.load(Ordering::Relaxed)
                        && Instant::now() < deadline
                    {
                        thread::sleep(Duration::from_millis(5));
                    }
                    if !stop.load(Ordering::Relaxed) {
                        if let Some(stage_barrier) = stage_barrier {
                            while !stage_barrier.load(Ordering::Acquire)
                                && !stop.load(Ordering::Relaxed)
                                && Instant::now() < deadline
                            {
                                thread::sleep(Duration::from_millis(5));
                            }
                            assert!(
                                stage_barrier.load(Ordering::Acquire),
                                "provider stderr holder must be ready before stage advance"
                            );
                        }
                        fs::write(stage, "initial_connection\n").unwrap();
                    }
                }
                while !stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(5));
                }
            })
        };
        let app_server_thread = bind_socket(
            app_server_request.clone(),
            socket.clone(),
            None,
            None,
            Arc::clone(&stop),
        );
        let proxy_thread = bind_socket(
            proxy_request.clone(),
            proxy.clone(),
            Some(stage.clone()),
            descriptor_holder_ready.as_ref().map(Arc::clone),
            Arc::clone(&stop),
        );
        let descriptor_holder_thread = descriptor_holder_ready.map(|ready| {
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(10);
                let stderr = loop {
                    match OpenOptions::new()
                        .write(true)
                        .custom_flags(libc::O_NONBLOCK)
                        .open(&provider_stderr_pipe)
                    {
                        Ok(stderr) => break stderr,
                        Err(err)
                            if Instant::now() < deadline
                                && matches!(
                                    err.raw_os_error(),
                                    Some(libc::ENOENT) | Some(libc::ENXIO)
                                ) =>
                        {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(err) => panic!("provider stderr holder must open the pipe: {err}"),
                    }
                };
                let child = Command::new("sleep")
                    .arg("300")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::from(stderr))
                    .spawn()
                    .expect("provider stderr holder must start");
                ready.store(true, Ordering::Release);
                child
            })
        });
        let interrupt_thread = interrupt.then(|| {
            let runtime_exit_status = runtime_exit_status.clone();
            let launcher_pid_file = launcher_pid_file.clone();
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(10);
                while (!runtime_exit_status.exists() || !launcher_pid_file.exists())
                    && Instant::now() < deadline
                {
                    thread::sleep(Duration::from_millis(1));
                }
                let pid = fs::read_to_string(&launcher_pid_file)
                    .expect("provider must persist the launcher pid before exiting")
                    .parse::<libc::pid_t>()
                    .expect("launcher pid must be valid");
                assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
            })
        });
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(launch_script(&[]))
            .arg("agent-session-launch-test")
            .arg(&socket)
            .arg(&proxy)
            .arg(&handoff)
            .arg(&attached)
            .arg(&helper)
            .arg(&state_dir)
            .arg(id)
            .arg(&helper)
            .arg(&cwd)
            .env("FAKE_PROVIDER_STAGE", &stage)
            .env("FAKE_APP_SERVER_REQUEST", &app_server_request)
            .env("FAKE_PROXY_REQUEST", &proxy_request)
            .env("FAKE_PROVIDER_EXIT", status.to_string())
            .env("FAKE_PROVIDER_STDERR", stderr)
            .env(
                "FAKE_LAUNCHER_PID_FILE",
                if interrupt {
                    launcher_pid_file.to_string_lossy().into_owned()
                } else {
                    String::new()
                },
            )
            .env("PATH", &test_path);
        let output = crate::run_output_with_timeout(command, Duration::from_secs(10));
        stop.store(true, Ordering::Relaxed);
        app_server_thread.join().unwrap();
        proxy_thread.join().unwrap();
        if let Some(thread) = interrupt_thread {
            thread.join().expect("launcher interrupt must not panic");
        }
        let mut descriptor_holder = descriptor_holder_thread.map(|thread| {
            thread
                .join()
                .expect("provider stderr holder setup must not panic")
        });
        let holder_was_alive = descriptor_holder
            .as_mut()
            .map(|child| {
                let alive = child
                    .try_wait()
                    .expect("provider stderr holder status must be readable")
                    .is_none();
                let _ = child.kill();
                let _ = child.wait();
                alive
            })
            .unwrap_or(true);
        let output =
            output.expect("generated launcher must terminate without waiting for stderr holders");
        if descendant {
            assert!(
                holder_was_alive,
                "generated launcher must return while the provider stderr holder is alive"
            );
        }
        assert_eq!(
            output.status.code(),
            Some(if interrupt { 143 } else { status })
        );
        session_dir
    };

    let failed = run_case("failed-stderr", 17, "post-ready-failure\n", true, false);
    assert_eq!(
        fs::read(failed.join(".startup-diagnostic.log")).unwrap(),
        b"post-ready-failure\n"
    );
    assert_eq!(
        fs::read_to_string(failed.join(".runtime-exit-status")).unwrap(),
        "17\n"
    );

    let interrupted = run_case("interrupted-drain", 29, "interrupted\n", true, true);
    assert_eq!(
        fs::read(interrupted.join(".startup-diagnostic.log")).unwrap(),
        b"interrupted\n"
    );
    assert_eq!(
        fs::read_to_string(interrupted.join(".runtime-exit-status")).unwrap(),
        "29\n"
    );

    let status_only = run_case("failed-status-only", 23, "", false, false);
    assert!(!status_only.join(".startup-diagnostic.log").exists());
    assert_eq!(
        fs::read_to_string(status_only.join(".startup-failure")).unwrap(),
        "provider-client-exited\n",
        "an initial proxy connection without a bound thread must not hide failed bootstrap"
    );
    assert_eq!(
        fs::read_to_string(status_only.join(".runtime-exit-status")).unwrap(),
        "23\n"
    );

    let clean = run_case("clean", 0, "ordinary ready stderr\n", false, false);
    assert!(!clean.join(".startup-diagnostic.log").exists());
    assert!(!clean.join(".runtime-exit-status").exists());
}

#[test]
fn managed_codex_client_launch_disables_startup_update_check_without_owning_base_arguments() {
    let script = launch_script(&[]);
    assert!(script.contains(
        "\"$agent\" -c check_for_update_on_startup=false --remote \"unix://$proxy\" \"$@\" 9>&-"
    ));
    assert!(!script.contains("--cd \"$cwd\""));
    assert!(!script.contains("--no-alt-screen \"$@\""));
}

#[test]
fn startup_diagnostic_collector_caps_private_failure_output_discards_clean_ready_output_and_retains_abnormal_ready_output()
 {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let tmp = tempfile::TempDir::new().unwrap();
    let stage = tmp.path().join("stage");
    let diagnostic = tmp.path().join("diagnostic.log");
    let exit_status = tmp.path().join("runtime-exit-status");
    fs::write(&stage, "provider_client\n").unwrap();
    let buffer = tmp.path().join("diagnostic.buffer");
    let command = format!(
        "startup_stage={}; startup_diagnostic={}; startup_diagnostic_buffer={}; runtime_exit_status={}; {}; collect_startup_diagnostic",
        shell_words::quote(&stage.to_string_lossy()),
        shell_words::quote(&diagnostic.to_string_lossy()),
        shell_words::quote(&buffer.to_string_lossy()),
        shell_words::quote(&exit_status.to_string_lossy()),
        STARTUP_DIAGNOSTIC_COLLECTOR_SCRIPT,
    );
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = vec![b'x'; 2 * 1024 * 1024];
    input.extend_from_slice(b"failure-sentinel");
    child.stdin.take().unwrap().write_all(&input).unwrap();
    assert!(child.wait().unwrap().success());

    let retained = fs::read(&diagnostic).unwrap();
    assert!(retained.len() <= 16 * 1024);
    assert!(retained.ends_with(b"failure-sentinel"));
    assert_eq!(
        fs::metadata(&diagnostic).unwrap().permissions().mode() & 0o777,
        0o600
    );

    fs::remove_file(&diagnostic).unwrap();
    fs::write(&stage, "initial_connection\n").unwrap();
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&vec![b'y'; 2 * 1024 * 1024])
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert!(!diagnostic.exists());
    assert!(!buffer.exists());

    fs::write(&exit_status, "1\n").unwrap();
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"post-ready-failure\n")
        .unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(fs::read(&diagnostic).unwrap(), b"post-ready-failure\n");

    fs::remove_file(&diagnostic).unwrap();
    let mut split_utf8 = "🙂".repeat(4096).into_bytes();
    split_utf8.push(b'x');
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&split_utf8).unwrap();
    assert!(child.wait().unwrap().success());
    let retained = fs::read(&diagnostic).unwrap();
    assert_eq!(retained.len(), 16 * 1024);
    assert!(
        std::str::from_utf8(&retained).is_err(),
        "the byte cap fixture must split a multibyte code point"
    );
}

#[test]
fn explicit_cleanup_removes_only_the_derived_runtime_files() {
    let lock = GlobalStateLock::new();
    let runtime_dir = tempfile::Builder::new()
        .prefix("cx-")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let _runtime_dir = EnvGuard::set(
        &lock,
        "XDG_RUNTIME_DIR",
        runtime_dir.path().to_str().unwrap(),
    );
    let context = CliContext {
        state_dir: runtime_dir.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("cleanup-runtime", Path::new("/placeholder"));
    let socket = allocate_socket_path(&context, &record).unwrap();
    record = record_with_runtime("cleanup-runtime", &socket);
    for path in [
        socket.clone(),
        socket.with_extension("proxy"),
        socket.with_extension("thread"),
        socket.with_extension("attached"),
    ] {
        fs::write(path, b"stale").unwrap();
    }
    let unrelated = socket.with_extension("unrelated");
    fs::write(&unrelated, b"keep").unwrap();

    let replacement_runtime = tempfile::Builder::new()
        .prefix("cx-")
        .tempdir_in("/tmp")
        .unwrap();
    let _replacement_runtime = EnvGuard::set(
        &lock,
        "XDG_RUNTIME_DIR",
        replacement_runtime.path().to_str().unwrap(),
    );

    cleanup_runtime_files(&context, &record).unwrap();

    assert!(!socket.exists());
    assert!(!socket.with_extension("proxy").exists());
    assert!(!socket.with_extension("thread").exists());
    assert!(!socket.with_extension("attached").exists());
    assert!(unrelated.exists());
}

#[test]
fn runtime_rejects_a_world_accessible_runtime_root() {
    let lock = GlobalStateLock::new();
    let runtime_dir = tempfile::Builder::new()
        .prefix("cx-")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let _runtime_dir = EnvGuard::set(
        &lock,
        "XDG_RUNTIME_DIR",
        runtime_dir.path().to_str().unwrap(),
    );

    let context = CliContext {
        state_dir: runtime_dir.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("unsafe-runtime", Path::new("/placeholder"));
    let err = allocate_socket_path(&context, &record).unwrap_err();
    assert_eq!(err.code(), "codex-app-server-runtime-dir-unsafe");
}

#[test]
fn runtime_paths_are_isolated_by_state_and_launch_identity() {
    let lock = GlobalStateLock::new();
    let runtime_dir = tempfile::Builder::new()
        .prefix("cx-")
        .tempdir_in("/tmp")
        .unwrap();
    fs::set_permissions(runtime_dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let _runtime_dir = EnvGuard::set(
        &lock,
        "XDG_RUNTIME_DIR",
        runtime_dir.path().to_str().unwrap(),
    );
    let record = record_with_runtime("shared-id", Path::new("/placeholder"));
    let context_a = CliContext {
        state_dir: runtime_dir.path().join("state-a"),
        host: None,
    };
    let context_b = CliContext {
        state_dir: runtime_dir.path().join("state-b"),
        host: None,
    };
    let first = allocate_socket_path(&context_a, &record).unwrap();
    let second = allocate_socket_path(&context_b, &record).unwrap();
    let mut next_launch = record.clone();
    next_launch.runtime.as_mut().unwrap().launch_id = "next-launch".to_string();
    let third = allocate_socket_path(&context_a, &next_launch).unwrap();

    assert_ne!(first, second);
    assert_ne!(first, third);
}

#[test]
fn runtime_rejects_a_symlinked_runtime_root() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&root, &link).unwrap();
    let _runtime_dir = EnvGuard::set(&lock, "XDG_RUNTIME_DIR", link.to_str().unwrap());
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("symlink-runtime", Path::new("/placeholder"));

    let err = allocate_socket_path(&context, &record).unwrap_err();
    assert_eq!(err.code(), "codex-app-server-runtime-dir-unsafe");
}

async fn receive_json<S>(socket: &mut S) -> Value
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let message = socket.next().await.unwrap().unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

#[tokio::test(start_paused = true)]
async fn control_readiness_is_bounded_and_fails_when_startup_ends() {
    let (handle, _commands, ready) = starting_control_channel();
    assert_eq!(
        handle.wait_ready(Duration::from_secs(1)).await.unwrap_err(),
        "codex control startup timed out"
    );
    drop(ready);
    assert_eq!(
        handle.wait_ready(Duration::from_secs(1)).await.unwrap_err(),
        "codex control startup ended"
    );
}

#[tokio::test]
async fn response_wait_is_bounded_for_reconnect() {
    let mut stream = PendingMessageSink;
    let err = receive_response_with_timeout(&mut stream, 1, None, None, Duration::from_millis(5))
        .await
        .unwrap_err();
    assert_eq!(err, "Codex app-server request timed out");
}

#[test]
fn auth_loss_codex_structured_failures_and_negative_controls() {
    for info in [
        json!("unauthorized"),
        json!({"httpConnectionFailed":{"httpStatusCode":401}}),
    ] {
        let mut reducer = FailureReducer::new("thread-a");
        let raw = json!({"method":"turn/completed", "params":{"threadId":"thread-a", "turn":{"id":"turn-a", "status":"failed", "error":{"codexErrorInfo":info, "message":"private-canary"}}}});
        let projected = match server_observation(&raw) {
            ServerProjection::Unique(value) => value,
            _ => panic!("expected structured projection"),
        };
        let failure = reducer
            .ingest(&projected)
            .expect("401 must classify model authentication");
        assert_eq!(failure.kind.activity_reason(), "authentication");
        assert!(!projected.to_string().contains("private-canary"));
    }
    for raw in [
        json!({"method":"turn/completed", "params":{"threadId":"thread-a", "turn":{"id":"turn-a", "status":"failed", "error":{"codexErrorInfo":{"httpConnectionFailed":{"httpStatusCode":403}}, "message":"Unauthorized 401"}}}}),
        json!({"method":"mcpServer/elicitation/request", "id":1, "params":{"threadId":"thread-a", "mode":"url", "message":"Unauthorized 401"}}),
        json!({"method":"item/agentMessage/delta", "params":{"threadId":"thread-a", "delta":"401 Unauthorized"}}),
    ] {
        assert!(FailureReducer::new("thread-a").ingest(&raw).is_none());
    }
}

#[tokio::test]
async fn auth_loss_codex_retry_policy_does_not_delay_incident_and_can_recover() {
    for will_retry in [None, Some(true), Some(false)] {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = record_with_runtime("auth-retry", &tmp.path().join("server.sock"));
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        crate::activity::activate_runtime(&context, &record).unwrap();
        let mut raw = json!({"method":"error", "params":{"threadId":"thread-a",
            "turnId":"turn-a", "error":{"codexErrorInfo":"unauthorized"}}});
        if let Some(value) = will_retry {
            raw["params"]["willRetry"] = json!(value);
        }
        let ServerProjection::Unique(projected) = server_observation(&raw) else {
            panic!("structured unauthorized observation");
        };
        let mut reducer = FailureReducer::new("thread-a");
        for _ in 0..3 {
            process_live_message(&context, &record, &mut reducer, None, &projected)
                .await
                .unwrap();
        }
        let incident = crate::auth_incident::view(&context, &record).unwrap();
        assert_eq!(
            incident.source,
            crate::auth_incident::AuthSource::CodexAppServer
        );
        assert_eq!(incident.status, "auth_failed");
        let stored: Value = serde_json::from_slice(
            &fs::read(crate::session_dir(&context, &record.id).join("auth-incidents.json"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(stored["incidents"].as_array().unwrap().len(), 1);
        process_live_message(
            &context,
            &record,
            &mut reducer,
            None,
            &json!({"method":"turn/completed", "params":{"threadId":"thread-a",
                "turn":{"id":"turn-a", "status":"completed"}}}),
        )
        .await
        .unwrap();
        let recovered = crate::auth_incident::view(&context, &record).unwrap();
        assert_eq!(recovered.status, "recovered");
        assert_eq!(recovered.recovery_result.as_deref(), Some("healthy"));
    }
}

#[tokio::test]
async fn auth_loss_codex_fake_protocol_surfaces_durable_incident_and_board() {
    for (name, info, expected) in [
        ("unauthorized", json!("unauthorized"), true),
        (
            "http401",
            json!({"responseStreamConnectionFailed":{"httpStatusCode":401}}),
            true,
        ),
        (
            "http403",
            json!({"httpConnectionFailed":{"httpStatusCode":403}}),
            false,
        ),
        ("text401", json!("other"), false),
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = record_with_runtime(name, &tmp.path().join("server.sock"));
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        crate::activity::activate_runtime(&context, &record).unwrap();
        let mut reducer = FailureReducer::new("thread-a");
        let message = json!({"method":"turn/completed", "params":{"threadId":"thread-a", "turn":{"id":"turn-a", "status":"failed", "error":{"codexErrorInfo":info,"message":"ordinary text mentioning 401 Unauthorized private-canary"}}}});
        let ServerProjection::Unique(projected) = server_observation(&message) else {
            panic!("structured projection");
        };
        process_live_message(&context, &record, &mut reducer, None, &projected)
            .await
            .unwrap();
        let incident = crate::auth_incident::view(&context, &record);
        assert_eq!(incident.is_some(), expected, "{name}");
        let view = crate::session_view(
            &context,
            &record,
            Some("running".into()),
            Some(Path::new("/nonexistent/fixture-tmux")),
        );
        let list = serde_json::to_value(view).unwrap();
        let board = crate::board::project_record(&list, "fixture-machine", None, false).unwrap();
        if expected {
            assert_eq!(
                list["turn_state"]["last_turn"]["provider_failure_kind"],
                "authentication"
            );
            assert_eq!(
                board["turn_state"]["last_turn"]["provider_failure_kind"],
                "authentication"
            );
            assert_eq!(board["auth_incident"]["provider"], "codex");
            assert_eq!(
                board["auth_incident"]["runtime_incarnation"],
                record.runtime.as_ref().unwrap().launch_id
            );
            assert!(!board.to_string().contains("private-canary"));
        } else {
            assert!(board.get("auth_incident").is_none());
        }
        // MCP login and ordinary assistant text never become provider failure.
        for raw in [
            json!({"method":"mcpServer/elicitation/request","id":1,"params":{"threadId":"thread-a","mode":"url"}}),
            json!({"method":"item/agentMessage/delta","params":{"threadId":"thread-a","delta":"401 Unauthorized"}}),
        ] {
            process_live_message(&context, &record, &mut reducer, None, &raw)
                .await
                .unwrap();
        }
        assert_eq!(
            crate::auth_incident::view(&context, &record).is_some(),
            expected
        );
    }
}

#[tokio::test]
async fn external_auth_refresh_rebinds_durable_state_without_serializing_token() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    let calls = tmp.path().join("calls");
    fs::write(
        &broker,
        r#"#!/bin/sh
calls=$1
shift
printf '%s\n' "$*" >> "$calls"
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"refreshed-fixture-token","chatgpt_account_id":"workspace-refreshed","plan":"team"}'
"#,
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let argv = serde_json::to_string(&vec![
        broker.to_string_lossy().into_owned(),
        calls.to_string_lossy().into_owned(),
    ])
    .unwrap();
    let _broker = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &argv);
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("refresh-success", &tmp.path().join("server.sock"));
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-refresh-success",
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();

    let mut sink = RecordingMessageSink::default();
    assert!(
        respond_to_external_auth_refresh(
            &mut sink,
            &json!({
                "id": "refresh-1",
                "method": "account/chatgptAuthTokens/refresh",
                "params": {
                    "reason": "unauthorized",
                    "previousAccountId": "workspace-old"
                }
            }),
            Some((&context, &record, "acct1")),
        )
        .await
        .unwrap()
    );
    let incident = crate::auth_incident::view(&context, &record).unwrap();
    assert_eq!(
        incident.source,
        crate::auth_incident::AuthSource::CodexExternalRefresh
    );
    assert_eq!(
        incident.recovery_result.as_deref(),
        Some("credentials_refreshed")
    );
    assert_eq!(incident.status, "recovered");
    let response: Value = match &sink.messages[0] {
        Message::Text(text) => serde_json::from_str(text).unwrap(),
        message => panic!("unexpected refresh response: {message:?}"),
    };
    assert_eq!(response["id"], "refresh-1");
    assert_eq!(response["result"]["accessToken"], "refreshed-fixture-token");
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.state, "bound");
    assert_eq!(
        view.applied_runtime_id.as_deref(),
        Some("runtime-refresh-success")
    );
    let session_json =
        fs::read_to_string(crate::session_dir(&context, &record.id).join("session.json")).unwrap();
    assert!(!session_json.contains("refreshed-fixture-token"));
    assert_eq!(
        fs::read_to_string(calls).unwrap().trim(),
        "resolve --account acct1 --force-refresh --format json"
    );
    let previous_incident = incident.incident_id;
    assert!(respond_to_external_auth_refresh(&mut sink, &json!({"id":"refresh-1", "method":"account/chatgptAuthTokens/refresh", "params":{"reason":"unauthorized"}}), Some((&context, &record, "acct1"))).await.unwrap());
    assert_ne!(
        crate::auth_incident::view(&context, &record)
            .unwrap()
            .incident_id,
        previous_incident
    );
    // A broken reporting store must not prevent the credential response.
    fs::write(
        crate::session_dir(&context, &record.id).join("auth-incidents.json"),
        b"invalid",
    )
    .unwrap();
    assert!(respond_to_external_auth_refresh(&mut sink, &json!({"id":"refresh-2", "method":"account/chatgptAuthTokens/refresh", "params":{"reason":"unauthorized"}}), Some((&context, &record, "acct1"))).await.unwrap());
    assert_eq!(sink.messages.len(), 3);
    assert_eq!(
        crate::auth_incident::detection_health(&context, &record).as_deref(),
        Some("degraded_store_unavailable")
    );
}

#[tokio::test]
async fn external_auth_refresh_failure_restores_bound_binding_for_retry() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(&broker, "#!/bin/sh\nexit 9\n").unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let argv = serde_json::to_string(&vec![broker.to_string_lossy().into_owned()]).unwrap();
    let _broker = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &argv);
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("refresh-failure", &tmp.path().join("server.sock"));
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-refresh-failure",
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();

    let error = respond_to_external_auth_refresh(
        &mut RecordingMessageSink::default(),
        &json!({
            "id": 19,
            "method": "account/chatgptAuthTokens/refresh",
            "params": { "reason": "unauthorized" }
        }),
        Some((&context, &record, "acct1")),
    )
    .await
    .unwrap_err();
    assert!(error.starts_with("Codex account refresh failed:"));
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.state, "bound");
    assert_eq!(view.failure_reason, None);
    assert_eq!(
        view.applied_runtime_id.as_deref(),
        Some("runtime-refresh-failure")
    );
    assert_eq!(
        crate::auth_incident::view(&context, &record)
            .unwrap()
            .recovery_result
            .as_deref(),
        Some("refresh_failed")
    );
    assert!(crate::codex_account::ensure_input_allowed(&persisted).is_ok());

    fs::write(
        &broker,
        r#"#!/bin/sh
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"retry-fixture-token","chatgpt_account_id":"workspace-retry"}'
"#,
    )
    .unwrap();
    let mut retry_sink = RecordingMessageSink::default();
    assert!(
        respond_to_external_auth_refresh(
            &mut retry_sink,
            &json!({
                "id": 20,
                "method": "account/chatgptAuthTokens/refresh",
                "params": { "reason": "unauthorized" }
            }),
            Some((&context, &record, "acct1")),
        )
        .await
        .unwrap()
    );
    let retried = crate::load_session_record(&context, &record.id).unwrap();
    let retry_view = crate::codex_account::view_for_record(&retried);
    assert_eq!(retry_view.state, "bound");
    assert_eq!(retry_view.revision, 3);
    assert_eq!(retry_sink.messages.len(), 1);
}

#[tokio::test]
async fn external_auth_refresh_send_failure_restores_bound_binding_for_retry() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(
        &broker,
        r#"#!/bin/sh
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"send-failure-fixture-token","chatgpt_account_id":"workspace-send-failure"}'
"#,
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let argv = serde_json::to_string(&vec![broker.to_string_lossy().into_owned()]).unwrap();
    let _broker = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &argv);
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("refresh-send-failure", &tmp.path().join("server.sock"));
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-refresh-send-failure",
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();

    let error = respond_to_external_auth_refresh(
        &mut FailingMessageSink,
        &json!({
            "id": 21,
            "method": "account/chatgptAuthTokens/refresh",
            "params": { "reason": "unauthorized" }
        }),
        Some((&context, &record, "acct1")),
    )
    .await
    .unwrap_err();

    assert!(error.starts_with("Codex app-server write failed:"));
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.state, "bound");
    assert_eq!(view.revision, 2);
    assert_eq!(view.selected_account.as_deref(), Some("acct1"));
    assert_eq!(
        view.applied_runtime_id.as_deref(),
        Some("runtime-refresh-send-failure")
    );
    assert!(crate::codex_account::ensure_input_allowed(&persisted).is_ok());
}

#[tokio::test(start_paused = true)]
async fn command_enqueue_is_included_in_the_control_timeout() {
    let (handle, _commands) = control_channel();
    let mut tasks = Vec::new();
    for _ in 0..5 {
        let handle = handle.clone();
        tasks.push(tokio::spawn(async move { handle.usage().await }));
    }
    tokio::task::yield_now().await;
    tokio::time::advance(CONTROL_RESPONSE_TIMEOUT + Duration::from_millis(1)).await;
    for task in tasks {
        assert_eq!(
            task.await.unwrap().unwrap_err(),
            "codex rate-limit request timed out"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn queued_steer_waits_for_handler_so_its_durable_fence_cannot_expire_early() {
    let (handle, mut commands) = control_channel();
    let task = tokio::spawn(async move {
        handle
            .steer_prompt("mailbox checkpoint", "projected-active-turn")
            .await
    });
    let command = commands.recv().await.expect("queued steer");
    let ControlCommand::Steer { response, .. } = command else {
        panic!("expected steer command");
    };

    tokio::time::advance(CONTROL_SUBMIT_TOTAL_TIMEOUT + Duration::from_secs(1)).await;
    assert!(
        !task.is_finished(),
        "an enqueued steer must keep its caller-side fence until the handler answers"
    );
    response
        .send(Ok("projected-active-turn".to_string()))
        .expect("complete steer");
    assert_eq!(task.await.unwrap().unwrap(), "projected-active-turn");
}

#[tokio::test(start_paused = true)]
async fn submit_timeout_covers_resume_plus_turn_acknowledgement_budget() {
    let (handle, mut commands) = control_channel();
    let responder = tokio::spawn(async move {
        let Some(ControlCommand::Continue { response, .. }) = commands.recv().await else {
            panic!("continuation command was not delivered");
        };
        tokio::time::sleep(Duration::from_secs(20)).await;
        let _ = response.send(Ok("acknowledged-turn".to_string()));
    });

    assert_eq!(
        handle.submit("fixed continuation").await.unwrap(),
        "acknowledged-turn"
    );
    responder.await.unwrap();
}

async fn respond<S>(socket: &mut S, request: &Value, result: Value)
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    socket
        .send(Message::Text(
            json!({ "id": request["id"], "result": result })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
}

#[test]
fn reducer_requires_exact_error_and_matching_failed_completion() {
    let mut reducer = FailureReducer::new("thread-a");
    let error = json!({
        "method": "error",
        "params": {
            "threadId": "thread-a",
            "turnId": "turn-a",
            "willRetry": false,
            "error": { "message": "ignored", "codexErrorInfo": "usageLimitExceeded" }
        }
    });
    assert_eq!(reducer.ingest(&error), None);
    assert_eq!(
        reducer.ingest(&json!({
            "method": "turn/completed",
            "params": { "threadId": "thread-a", "turn": { "id": "turn-a", "status": "failed" } }
        })),
        Some(StructuredFailure {
            thread_id: "thread-a".into(),
            turn_id: "turn-a".into(),
            kind: StructuredFailureKind::UsageExhausted,
        })
    );
    assert_eq!(
        reducer.ingest(&error),
        None,
        "a completed turn cannot be re-armed"
    );
}

#[test]
fn reducer_keeps_raw_active_turn_transient_and_matches_only_its_projection() {
    let mut reducer = FailureReducer::new("thread-a");
    reducer.ingest(&json!({
        "method": "turn/started",
        "params": {
            "threadId": "thread-a",
            "turn": { "id": "raw-turn-a", "status": "inProgress" }
        }
    }));
    let projected = crate::activity::projected_codex_turn_identifier("runtime-a", "raw-turn-a")
        .expect("project raw active turn");
    assert_eq!(
        reducer
            .raw_turn_for_projection("runtime-a", &projected)
            .expect("match exact projection"),
        "raw-turn-a"
    );
    assert!(
        reducer
            .raw_turn_for_projection("runtime-a", "local:v1:stale")
            .is_err()
    );

    reducer.ingest(&json!({
        "method": "turn/completed",
        "params": {
            "threadId": "thread-a",
            "turn": { "id": "raw-turn-a", "status": "completed" }
        }
    }));
    assert!(
        reducer
            .raw_turn_for_projection("runtime-a", &projected)
            .is_err(),
        "completed turns must no longer accept steering"
    );
}

#[test]
fn reducer_admits_exact_server_overload_as_a_structured_capacity_failure() {
    let mut reducer = FailureReducer::new("thread-a");
    assert_eq!(
        reducer.ingest(&json!({
            "method": "error",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-capacity",
                "willRetry": false,
                "error": {
                    "message": "Selected model is at capacity",
                    "codexErrorInfo": "serverOverloaded"
                }
            }
        })),
        None
    );
    assert_eq!(
        reducer.ingest(&json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": { "id": "turn-capacity", "status": "failed" }
            }
        })),
        Some(StructuredFailure {
            thread_id: "thread-a".into(),
            turn_id: "turn-capacity".into(),
            kind: StructuredFailureKind::ProviderCapacity,
        }),
        "the exact structured serverOverloaded enum must survive the matched failed completion",
    );
}

#[test]
fn structured_provider_capacity_arms_managed_capacity_recovery() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("provider-capacity", &tmp.path().join("unused.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();

    let result = crate::activity::ingest_codex_app_server_failure_with_kind(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        "thread-capacity",
        "turn-capacity",
        StructuredFailureKind::ProviderCapacity,
    )
    .unwrap();

    assert_eq!(
        result
            .turn_state
            .last_turn
            .as_ref()
            .and_then(crate::activity::LastTurn::provider_failure_kind),
        Some("provider_capacity")
    );
    assert_eq!(
        crate::auto_resume::view_for_record(&context, &record).state,
        "scheduled",
        "authoritative provider capacity should arm the managed recovery chain"
    );
}

#[test]
fn reducer_fails_closed_for_conflicting_failure_kinds_on_one_turn() {
    for kinds in [
        ["usageLimitExceeded", "serverOverloaded"],
        ["serverOverloaded", "usageLimitExceeded"],
    ] {
        let mut reducer = FailureReducer::new("thread-a");
        for kind in kinds {
            assert_eq!(
                reducer.ingest(&json!({
                    "method": "error",
                    "params": {
                        "threadId": "thread-a",
                        "turnId": "turn-conflict",
                        "willRetry": false,
                        "error": { "codexErrorInfo": kind }
                    }
                })),
                None
            );
        }
        assert_eq!(
            reducer.ingest(&json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-a",
                    "turn": { "id": "turn-conflict", "status": "failed" }
                }
            })),
            None,
            "conflicting structured evidence must never select the retry-authorizing usage branch"
        );
    }

    let mut embedded = FailureReducer::new("thread-a");
    for kind in ["usageLimitExceeded", "serverOverloaded"] {
        assert_eq!(
            embedded.ingest(&json!({
                "method": "error",
                "params": {
                    "threadId": "thread-a",
                    "turnId": "turn-embedded",
                    "willRetry": false,
                    "error": { "codexErrorInfo": kind }
                }
            })),
            None
        );
    }
    assert_eq!(
        embedded.ingest(&json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": {
                    "id": "turn-embedded",
                    "status": "failed",
                    "error": { "codexErrorInfo": "serverOverloaded" }
                }
            }
        })),
        Some(StructuredFailure {
            thread_id: "thread-a".into(),
            turn_id: "turn-embedded".into(),
            kind: StructuredFailureKind::ProviderCapacity,
        }),
        "the terminal completion's embedded structured kind resolves earlier conflicting frames"
    );
}

#[test]
fn reducer_fails_closed_for_wrong_thread_status_reason_retry_and_order() {
    for mutation in [
        json!({"threadId":"other","turnId":"turn-a","willRetry":false,"error":{"codexErrorInfo":"usageLimitExceeded"}}),
        json!({"threadId":"thread-a","turnId":"turn-a","willRetry":true,"error":{"codexErrorInfo":"usageLimitExceeded"}}),
        json!({"threadId":"thread-a","turnId":"turn-a","willRetry":false,"error":{"codexErrorInfo":"other"}}),
    ] {
        let mut reducer = FailureReducer::new("thread-a");
        assert_eq!(
            reducer.ingest(&json!({"method":"error","params":mutation})),
            None
        );
        assert_eq!(reducer.ingest(&json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"failed"}}})), None);
    }
    let mut reordered = FailureReducer::new("thread-a");
    assert_eq!(reordered.ingest(&json!({"method":"turn/completed","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"failed"}}})), None);
    assert_eq!(reordered.ingest(&json!({"method":"error","params":{"threadId":"thread-a","turnId":"turn-a","willRetry":false,"error":{"codexErrorInfo":"usageLimitExceeded"}}})), None);

    let mut embedded = FailureReducer::new("thread-a");
    assert_eq!(
        embedded.ingest(&json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": {
                    "id": "turn-b",
                    "status": "failed",
                    "error": { "codexErrorInfo": "usageLimitExceeded" }
                }
            }
        })),
        Some(StructuredFailure {
            thread_id: "thread-a".into(),
            turn_id: "turn-b".into(),
            kind: StructuredFailureKind::UsageExhausted,
        })
    );
}

#[test]
fn reducer_bounds_provider_controlled_turn_identifiers() {
    let mut reducer = FailureReducer::new("thread-a");
    for index in 0..(MAX_REDUCER_PENDING_TURNS * 2) {
        assert_eq!(
            reducer.ingest(&json!({
                "method": "error",
                "params": {
                    "threadId": "thread-a",
                    "turnId": format!("turn-{index}"),
                    "willRetry": false,
                    "error": { "codexErrorInfo": "usageLimitExceeded" }
                }
            })),
            None
        );
        assert_eq!(
            reducer.ingest(&json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-a",
                    "turn": { "id": format!("failed-{index}"), "status": "failed" }
                }
            })),
            None
        );
    }
    assert_eq!(reducer.pending_turns.len(), MAX_REDUCER_PENDING_TURNS);
    assert_eq!(reducer.completed_turns.len(), MAX_REDUCER_PENDING_TURNS);
    let oversized = "x".repeat(MAX_PROTOCOL_ID_BYTES + 1);
    assert_eq!(
        reducer.ingest(&json!({
            "method": "error",
            "params": {
                "threadId": "thread-a",
                "turnId": oversized,
                "willRetry": false,
                "error": { "codexErrorInfo": "usageLimitExceeded" }
            }
        })),
        None
    );
    assert_eq!(reducer.pending_turns.len(), MAX_REDUCER_PENDING_TURNS);
}

#[test]
fn reducer_detects_a_real_quota_failure_after_the_bounded_horizon() {
    let mut reducer = FailureReducer::new("thread-a");
    for index in 0..(MAX_REDUCER_PENDING_TURNS + 1) {
        assert_eq!(
            reducer.ingest(&json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-a",
                    "turn": { "id": format!("ordinary-failure-{index}"), "status": "failed" }
                }
            })),
            None
        );
    }
    assert_eq!(
        reducer.ingest(&json!({
            "method": "error",
            "params": {
                "threadId": "thread-a",
                "turnId": "quota-after-horizon",
                "willRetry": false,
                "error": { "codexErrorInfo": "usageLimitExceeded" }
            }
        })),
        None
    );
    assert!(
        reducer
            .ingest(&json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-a",
                    "turn": { "id": "quota-after-horizon", "status": "failed" }
                }
            }))
            .is_some()
    );
}

#[test]
fn proxy_request_tracking_bounds_id_size_and_cardinality() {
    let record = record_with_runtime("proxy-bounds", Path::new("/tmp/proxy-bounds.sock"));
    let mut observer = ProxyObserver::new();
    assert!(json_id_key(&Value::String("x".repeat(MAX_PROTOCOL_ID_BYTES + 1))).is_none());
    for index in 0..(MAX_REDUCER_PENDING_TURNS * 2) {
        observer
            .observe_client(
                &record,
                &json!({ "id": index, "method": "thread/start", "params": {} }),
            )
            .unwrap();
    }
    assert!(observer.pending_thread_starts.len() <= MAX_REDUCER_PENDING_TURNS);
    assert!(
        client_observation(&json!({
            "method": "turn/start",
            "params": { "threadId": "x".repeat(MAX_PROTOCOL_ID_BYTES + 1) }
        }))
        .is_none()
    );
    assert!(matches!(
        server_observation(&json!({
            "method": "account/rateLimits/updated",
            "params": {
                "rateLimits": {
                    "primary": {
                        "usedPercent": 100,
                        "oversized": "x".repeat(MAX_PROXY_OBSERVATION_BYTES)
                    }
                }
            }
        })),
        ServerProjection::Irrelevant
    ));
}

#[test]
fn exact_attention_projection_preserves_typed_ids_and_discards_content() {
    let cases = [
        (
            json!({
                "id": 1,
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "thread-a",
                    "turnId": "turn-a",
                    "command": "must-not-leave-the-adapter",
                    "reason": "must-not-leave-the-adapter"
                }
            }),
            json!(1),
            "approval",
            json!("turn-a"),
        ),
        (
            json!({
                "id": "1",
                "method": "item/tool/requestUserInput",
                "params": {
                    "threadId": "thread-a",
                    "turnId": "turn-a",
                    "questions": [{"question": "must-not-leave-the-adapter"}]
                }
            }),
            json!("1"),
            "clarification",
            json!("turn-a"),
        ),
        (
            json!({
                "id": 2,
                "method": "item/fileChange/requestApproval",
                "params": {"threadId": "thread-a", "turnId": "turn-a", "changes": "discarded"}
            }),
            json!(2),
            "approval",
            json!("turn-a"),
        ),
        (
            json!({
                "id": 3,
                "method": "item/permissions/requestApproval",
                "params": {"threadId": "thread-a", "turnId": "turn-a", "permissions": "discarded"}
            }),
            json!(3),
            "approval",
            json!("turn-a"),
        ),
        (
            json!({
                "id": 4,
                "method": "mcpServer/elicitation/request",
                "params": {"threadId": "thread-a", "turnId": null, "mode": "form", "serverName": "discarded"}
            }),
            json!(4),
            "clarification",
            Value::Null,
        ),
    ];

    let mut projected = Vec::new();
    for (raw, request_id, kind, turn_id) in cases {
        let ServerProjection::Unique(value) = server_observation(&raw) else {
            panic!("recognized blocking request must be a unique projection");
        };
        assert_eq!(
            value,
            json!({
                "method": "agent-session/attention/requested",
                "params": {
                    "requestId": request_id,
                    "threadId": "thread-a",
                    "turnId": turn_id,
                    "kind": kind
                }
            })
        );
        let wire = serde_json::to_string(&value).unwrap();
        assert!(!wire.contains("must-not-leave-the-adapter"));
        assert!(!wire.contains("discarded"));
        projected.push(value);
    }
    assert_ne!(
        projected[0].pointer("/params/requestId"),
        projected[1].pointer("/params/requestId"),
        "JSON integer 1 and string \"1\" must remain distinct"
    );

    let ServerProjection::Unique(resolved) = server_observation(&json!({
        "method": "serverRequest/resolved",
        "params": {"threadId": "thread-a", "requestId": 1}
    })) else {
        panic!("typed resolution must be a unique projection");
    };
    assert_eq!(
        resolved,
        json!({
            "method": "agent-session/attention/resolved",
            "params": {"threadId": "thread-a", "requestId": 1}
        })
    );
}

#[test]
fn mcp_elicitation_mode_maps_exactly_and_rejects_unknown_shapes() {
    for (mode, expected) in [
        ("form", "clarification"),
        ("openai/form", "clarification"),
        ("url", "authentication"),
    ] {
        let ServerProjection::Unique(projected) = server_observation(&json!({
            "id": format!("request-{mode}"),
            "method": "mcpServer/elicitation/request",
            "params": {"threadId": "thread-a", "turnId": null, "mode": mode}
        })) else {
            panic!("audited MCP elicitation mode must project");
        };
        assert_eq!(projected["params"]["kind"], expected);
    }
    for mode in [Value::Null, json!("future-mode"), json!({"bad": true})] {
        assert!(matches!(
            server_observation(&json!({
                "id": "request-invalid",
                "method": "mcpServer/elicitation/request",
                "params": {"threadId": "thread-a", "turnId": null, "mode": mode}
            })),
            ServerProjection::RejectedUnique
        ));
    }
}

#[test]
fn exact_attention_projection_rejects_invalid_scope_and_request_ids() {
    for value in [
        json!({
            "id": 1.5,
            "method": "item/fileChange/requestApproval",
            "params": {"threadId": "thread-a", "turnId": "turn-a"}
        }),
        json!({
            "id": 1,
            "method": "item/permissions/requestApproval",
            "params": {"threadId": "", "turnId": "turn-a"}
        }),
        json!({
            "method": "serverRequest/resolved",
            "params": {"threadId": "thread-a", "requestId": {"bad": true}}
        }),
    ] {
        assert!(matches!(
            server_observation(&value),
            ServerProjection::RejectedUnique
        ));
    }
}

#[tokio::test]
async fn exact_attention_requests_clear_independently_before_turn_completion() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("exact-attention", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let mut projection = ProxyProjection::new(context.clone(), record.clone());
    projection.observe_client(&json!({
        "method": "turn/start",
        "params": {"threadId": "thread-a"}
    }));
    projection.observe_server(&json!({
        "id": 1,
        "method": "item/commandExecution/requestApproval",
        "params": {"threadId": "thread-a", "turnId": "turn-a", "command": "secret"}
    }));
    projection.observe_server(&json!({
        "id": "1",
        "method": "item/commandExecution/requestApproval",
        "params": {"threadId": "thread-a", "turnId": "turn-a", "command": "secret"}
    }));
    let state = wait_for_activity(&context, &record.id, |state| {
        state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.attention.as_ref())
            .is_some_and(|attention| attention.pending_count == 2)
    })
    .await;
    assert_eq!(state.phase, crate::activity::TurnPhase::NeedsInput);
    assert_eq!(
        state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.attention.as_ref())
            .map(|attention| attention.pending_count),
        Some(2)
    );
    assert!(!serde_json::to_string(&state).unwrap().contains("secret"));

    projection.observe_server(&json!({
        "method": "serverRequest/resolved",
        "params": {"threadId": "thread-a", "requestId": 1}
    }));
    let one = wait_for_activity(&context, &record.id, |state| {
        state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.attention.as_ref())
            .is_some_and(|attention| attention.pending_count == 1)
    })
    .await;
    assert_eq!(one.phase, crate::activity::TurnPhase::NeedsInput);
    assert_eq!(
        one.current_turn
            .as_ref()
            .and_then(|turn| turn.attention.as_ref())
            .map(|attention| attention.pending_count),
        Some(1)
    );

    projection.observe_server(&json!({
        "method": "serverRequest/resolved",
        "params": {"threadId": "thread-a", "requestId": "1"}
    }));
    let cleared = wait_for_activity(&context, &record.id, |state| {
        state.phase == crate::activity::TurnPhase::Working
            && state
                .current_turn
                .as_ref()
                .and_then(|turn| turn.attention.as_ref())
                .is_none()
    })
    .await;
    assert_eq!(cleared.phase, crate::activity::TurnPhase::Working);
    assert!(
        cleared
            .current_turn
            .as_ref()
            .and_then(|turn| turn.attention.as_ref())
            .is_none()
    );

    let revision = cleared.revision;
    for request_id in [json!("1"), json!("unmatched")] {
        projection.observe_server(&json!({
            "method": "serverRequest/resolved",
            "params": {"threadId": "thread-a", "requestId": request_id}
        }));
    }
    projection.finish().await;
    assert_eq!(
        crate::activity::activity_status(&context, &record.id)
            .unwrap()
            .turn_state
            .revision,
        revision,
        "repeated and unmatched resolutions must be idempotent no-ops"
    );
}

#[tokio::test]
async fn exact_attention_allows_sequential_id_reuse_and_keeps_raw_id_private() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("attention-reuse", &tmp.path().join("server.sock"));
    let dir = crate::session_dir(&context, &record.id);
    fs::create_dir_all(&dir).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let mut projection = ProxyProjection::new(context.clone(), record.clone());
    projection.observe_client(&json!({
        "method": "turn/start",
        "params": {"threadId": "thread-a"}
    }));
    let raw_id = format!("private-{}", "x".repeat(MAX_PROTOCOL_ID_BYTES - 8));
    assert_eq!(raw_id.len(), MAX_PROTOCOL_ID_BYTES);

    for occurrence in 1..=2 {
        projection.observe_server(&json!({
            "id": raw_id.clone(),
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thread-a",
                "turnId": "turn-a",
                "command": "private-command-must-not-persist"
            }
        }));
        let requested = wait_for_activity(&context, &record.id, |state| {
            state.phase == crate::activity::TurnPhase::NeedsInput
        })
        .await;
        assert!(requested.revision >= occurrence * 2 - 1);

        let activity_files = [
            "activity.json",
            "activity.journal.jsonl",
            "activity.replay.bin",
        ];
        for _ in 0..100 {
            if activity_files.iter().all(|file| dir.join(file).is_file()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(activity_files.iter().all(|file| dir.join(file).is_file()));
        for file in activity_files {
            let bytes = fs::read(dir.join(file)).unwrap();
            assert!(
                !bytes
                    .windows(raw_id.len())
                    .any(|window| window == raw_id.as_bytes())
            );
            assert!(!String::from_utf8_lossy(&bytes).contains("private-command-must-not-persist"));
        }
        let public = serde_json::to_string(
            &crate::activity::activity_status(&context, &record.id)
                .unwrap()
                .turn_state,
        )
        .unwrap();
        assert!(!public.contains(&raw_id));
        assert!(!public.contains("private-command-must-not-persist"));

        projection.observe_server(&json!({
            "method": "serverRequest/resolved",
            "params": {"threadId": "thread-a", "requestId": raw_id.clone()}
        }));
        wait_for_activity(&context, &record.id, |state| {
            state.phase == crate::activity::TurnPhase::Working
                && state
                    .current_turn
                    .as_ref()
                    .and_then(|turn| turn.attention.as_ref())
                    .is_none()
        })
        .await;
    }
    projection.finish().await;

    assert!(matches!(
        server_observation(&json!({
            "id": "x".repeat(MAX_PROTOCOL_ID_BYTES + 1),
            "method": "item/commandExecution/requestApproval",
            "params": {"threadId": "thread-a", "turnId": "turn-a"}
        })),
        ServerProjection::RejectedUnique
    ));
}

#[tokio::test]
async fn exact_attention_rejects_wrong_turn_and_allows_nullable_mcp_turn() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("attention-turn-scope", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let mut projection = ProxyProjection::new(context.clone(), record.clone());
    projection.observe_client(&json!({
        "method": "turn/start",
        "params": {"threadId": "thread-a"}
    }));

    projection.observe_server(&json!({
        "id": "matching-request",
        "method": "item/commandExecution/requestApproval",
        "params": {"threadId": "thread-a", "turnId": "turn-a"}
    }));
    wait_for_activity(&context, &record.id, |state| {
        state.phase == crate::activity::TurnPhase::NeedsInput
    })
    .await;
    projection.observe_server(&json!({
        "method": "serverRequest/resolved",
        "params": {"threadId": "thread-a", "requestId": "matching-request"}
    }));
    wait_for_activity(&context, &record.id, |state| {
        state.phase == crate::activity::TurnPhase::Working
    })
    .await;

    projection.observe_server(&json!({
        "id": "mcp-request",
        "method": "mcpServer/elicitation/request",
        "params": {"threadId": "thread-a", "turnId": null, "mode": "form"}
    }));
    wait_for_activity(&context, &record.id, |state| {
        state.phase == crate::activity::TurnPhase::NeedsInput
    })
    .await;
    projection.observe_server(&json!({
        "method": "serverRequest/resolved",
        "params": {"threadId": "thread-a", "requestId": "mcp-request"}
    }));
    wait_for_activity(&context, &record.id, |state| {
        state.phase == crate::activity::TurnPhase::Working
    })
    .await;

    projection.observe_server(&json!({
        "id": "wrong-turn-request",
        "method": "item/commandExecution/requestApproval",
        "params": {"threadId": "thread-a", "turnId": "turn-b"}
    }));
    let unknown = wait_for_activity(&context, &record.id, |state| {
        state.phase == crate::activity::TurnPhase::Unknown
    })
    .await;
    assert_eq!(unknown.phase, crate::activity::TurnPhase::Unknown);
    projection.finish().await;
}

#[tokio::test]
async fn persisted_thread_observation_skips_duplicate_marker_io() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("persisted-binding", &tmp.path().join("server.sock"));
    let mut observer = ProxyObserver::new();
    let primary_start = client_observation(&json!({
        "id": 1,
        "method": "thread/start",
        "params": { "ephemeral": false, "threadSource": "user" }
    }))
    .unwrap();
    observer.observe_client(&record, &primary_start).unwrap();

    observer
        .observe_server(
            &context,
            &record,
            &json!({ "id": 1, "result": { "thread": { "id": "fresh-thread" } } }),
            Some("fresh-thread"),
        )
        .await
        .expect("the projection worker should trust the pre-forward persisted binding");

    assert_eq!(
        observer
            .reducer
            .as_ref()
            .map(|reducer| reducer.thread_id.as_str()),
        Some("fresh-thread")
    );
    assert!(
        !thread_attached_path(&record).unwrap().exists(),
        "the projection worker must not repeat marker persistence or validation"
    );
}

#[tokio::test]
async fn session_model_fresh_thread_and_optional_write_failure_preserve_binding() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("fresh-model-proxy", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let mut observer = ProxyObserver::new();
    observer
        .observe_client(
            &record,
            &json!({"id": 1, "method": "thread/start", "params": {}}),
        )
        .unwrap();
    observer.observe_server(&context, &record, &json!({"id": 1, "result": {"thread": {"id": "primary-thread"}, "model": "resolved-model", "reasoningEffort": "medium"}}), Some("primary-thread")).await.unwrap();
    observer.finish_model_settings().await;
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    assert!(persisted.provider_resume.is_none());
    let settings = crate::session_model::ModelSettings::for_record(&persisted);
    assert_eq!(settings.model.as_deref(), Some("resolved-model"));
    assert_eq!(settings.reasoning_effort.as_deref(), Some("medium"));
    // Remove only the fixture record to inject failure of optional metadata I/O.
    fs::remove_file(crate::session_dir(&context, &record.id).join("session.json")).unwrap();
    observer
        .observe_client(
            &record,
            &json!({"id": 2, "method": "thread/start", "params": {}}),
        )
        .unwrap();
    observer.observe_server(&context, &record, &json!({"id": 2, "result": {"thread": {"id": "primary-thread"}, "model": "other-model"}}), Some("primary-thread")).await.unwrap();
    observer.finish_model_settings().await;
    assert_eq!(
        observer.reducer.as_ref().unwrap().thread_id,
        "primary-thread"
    );
}

#[tokio::test]
async fn session_model_confirmed_update_survives_busy_record_without_delaying_response() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("busy-model-proxy", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let mut observer = ProxyObserver::new();
    observer
        .observe_client(
            &record,
            &json!({"id": 1, "method": "turn/start", "params": {
                "threadId": "primary-thread", "model": "confirmed-model", "reasoning_effort": "high"
            }}),
        )
        .unwrap();
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    tokio::time::timeout(
        Duration::from_millis(200),
        observer.observe_server(
            &context,
            &record,
            &json!({"id": 1, "result": {"turn": {"id": "confirmed"}}}),
            None,
        ),
    )
    .await
    .expect("optional metadata must not delay the protocol response")
    .unwrap();
    drop(lock);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let settings = crate::session_model::ModelSettings::for_record(
            &crate::load_session_record(&context, &record.id).unwrap(),
        );
        if settings.model.as_deref() == Some("confirmed-model") {
            assert_eq!(settings.reasoning_effort.as_deref(), Some("high"));
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "confirmed settings were dropped while the record was busy"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn session_model_updates_only_after_confirmed_primary_turn_requests() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("model-settings-proxy", &tmp.path().join("server.sock"));
    record.provider_resume = Some(serde_json::from_value(json!({
        "provider": "codex", "session_id": "primary-thread", "captured_at": "2030-01-01T00:00:00Z",
        "capture_method": "fixture", "resume_args": []
    })).unwrap());
    record.agent_args = vec!["--model=initial-model".into()];
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let mut observer = ProxyObserver::new();
    let request = client_observation(&json!({"id": 2, "method": "turn/start", "params": {
        "threadId": "primary-thread", "model": "updated-model", "effort": "high", "input": "private prompt"
    }})).unwrap();
    assert!(!request.to_string().contains("private prompt"));
    observer.observe_client(&record, &request).unwrap();
    assert_eq!(
        crate::session_model::ModelSettings::for_record(
            &crate::load_session_record(&context, &record.id).unwrap()
        )
        .model
        .as_deref(),
        Some("initial-model")
    );
    observer
        .observe_server(
            &context,
            &record,
            &json!({"id": 999, "result": {"turn": {"id": "unrelated"}}}),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        crate::session_model::ModelSettings::for_record(
            &crate::load_session_record(&context, &record.id).unwrap()
        )
        .model
        .as_deref(),
        Some("initial-model")
    );
    observer
        .observe_server(
            &context,
            &record,
            &json!({"id": 2, "result": {"turn": {"id": "confirmed"}}}),
            None,
        )
        .await
        .unwrap();
    observer.finish_model_settings().await;
    let settings = crate::session_model::ModelSettings::for_record(
        &crate::load_session_record(&context, &record.id).unwrap(),
    );
    assert_eq!(settings.model.as_deref(), Some("updated-model"));
    assert_eq!(settings.reasoning_effort.as_deref(), Some("high"));
    let ServerProjection::Unique(response) = server_observation(&json!({"id": 1, "result": {
        "thread": {"id": "primary-thread"}, "model": "resolved-model", "reasoningEffort": "medium", "config": {"secret": "must-not-project"}
    }})) else {
        panic!("thread response");
    };
    assert_eq!(response["result"]["model"], "resolved-model");
    assert_eq!(response["result"]["reasoning_effort"], "medium");
    assert!(!response.to_string().contains("must-not-project"));
}

#[tokio::test]
async fn proxy_observer_keeps_primary_binding_when_tui_starts_an_auxiliary_thread() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("auxiliary-thread", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let mut observer = ProxyObserver::new();
    observer
        .observe_client(
            &record,
            &json!({ "id": 1, "method": "thread/start", "params": {} }),
        )
        .unwrap();
    observer
        .observe_server(
            &context,
            &record,
            &json!({ "id": 1, "result": { "thread": { "id": "primary-thread" } } }),
            Some("primary-thread"),
        )
        .await
        .unwrap();

    let auxiliary_thread = concat!(
        "local:v1:",
        "aaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaa",
        "aaaaaaaaaaaaaaaa"
    );

    let auxiliary_start = client_observation(&json!({
        "id": 2,
        "method": "thread/start",
        "params": {
            "ephemeral": true,
            "threadSource": "thread_title",
            "model": "must-not-leave-the-adapter"
        }
    }))
    .unwrap();
    assert_eq!(
        auxiliary_start,
        json!({
            "id": 2,
            "method": "thread/start",
            "params": { "systemEphemeral": true }
        })
    );
    assert_eq!(
        client_observation(&json!({
            "id": 6,
            "method": "thread/start",
            "params": { "ephemeral": true, "threadSource": "user" }
        }))
        .unwrap()
        .pointer("/params/systemEphemeral"),
        Some(&json!(false)),
        "user-created ephemeral threads must still enforce the primary binding"
    );
    observer.observe_client(&record, &auxiliary_start).unwrap();
    observer
        .observe_server(
            &context,
            &record,
            &json!({ "id": 2, "result": { "thread": { "id": auxiliary_thread } } }),
            None,
        )
        .await
        .expect("an auxiliary thread/start must not replace or disable the primary projection");
    let registry_bytes = fs::read(system_ephemeral_thread_registry_path(&context, &record))
        .expect("system-ephemeral registry");
    let registry_json: Value =
        serde_json::from_slice(&registry_bytes).expect("valid registry json");
    assert!(registry_json.get("identity_digests").is_some());
    assert!(registry_json.get("provider_session_ids").is_none());
    let registry: SystemEphemeralThreadRegistry =
        serde_json::from_slice(&registry_bytes).expect("valid system-ephemeral registry");
    let projected_auxiliary = crate::activity::projected_codex_session_identifier(
        &record.runtime.as_ref().unwrap().launch_id,
        auxiliary_thread,
    )
    .expect("unconditionally project the raw auxiliary identity");
    assert_ne!(projected_auxiliary, auxiliary_thread);
    assert_eq!(registry.identity_digests, vec![projected_auxiliary.clone()]);
    assert!(
        system_ephemeral_raw_session_is_registered(&context, &record, auxiliary_thread)
            .expect("look up a raw auxiliary identity")
    );
    assert!(
        system_ephemeral_normalized_session_is_registered(&context, &record, &projected_auxiliary,)
            .expect("look up a projected auxiliary identity"),
        "the private registry must retain only projected provider identities"
    );

    assert_eq!(
        observer
            .reducer
            .as_ref()
            .map(|reducer| reducer.thread_id.as_str()),
        Some("primary-thread")
    );
    observer
        .observe_client(
            &record,
            &json!({
                "id": 3,
                "method": "turn/start",
                "params": { "threadId": auxiliary_thread }
            }),
        )
        .expect("a turn on the confirmed system-ephemeral thread must be ignored");
    assert_eq!(
        observer
            .observe_client(
                &record,
                &json!({
                    "id": 4,
                    "method": "turn/start",
                    "params": { "threadId": "unknown-thread" }
                }),
            )
            .unwrap_err(),
        "Codex TUI proxy switched to a different thread"
    );

    let user_start = client_observation(&json!({
        "id": 5,
        "method": "thread/start",
        "params": { "ephemeral": false, "threadSource": "user" }
    }))
    .unwrap();
    observer.observe_client(&record, &user_start).unwrap();
    observer
        .observe_server(
            &context,
            &record,
            &json!({ "id": 5, "result": { "thread": { "id": "other-user-thread" } } }),
            None,
        )
        .await
        .expect("native clear may switch the primary conversation");
    assert_eq!(
        observer.reducer.as_ref().unwrap().thread_id,
        "other-user-thread"
    );
}

#[tokio::test]
async fn proxy_acknowledges_system_ephemeral_thread_before_forwarding_its_response() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("auxiliary-barrier", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let mut projection = ProxyProjection::new(context.clone(), record.clone());

    projection.observe_client(&json!({
        "id": 1,
        "method": "thread/start",
        "params": { "ephemeral": false, "threadSource": "user" }
    }));
    projection
        .observe_server_before_forward(&json!({
            "id": 1,
            "result": { "thread": { "id": "primary-thread" } }
        }))
        .await
        .unwrap();

    projection.observe_client(&json!({
        "id": 2,
        "method": "thread/start",
        "params": { "ephemeral": true, "threadSource": "system" }
    }));
    projection
        .observe_server_before_forward(&json!({
            "id": 2,
            "result": { "thread": { "id": "auxiliary-thread" } }
        }))
        .await
        .expect("the auxiliary identity must be accepted before its response is forwarded");
    projection.observe_client(&json!({
        "id": 3,
        "method": "turn/start",
        "params": { "threadId": "auxiliary-thread" }
    }));
    projection.finish().await;

    assert!(
        !crate::session_dir(&context, &record.id)
            .join("activity.unhealthy.json")
            .exists(),
        "the confirmed system-ephemeral turn must not fail the primary projection"
    );
    assert_eq!(
        fs::read_to_string(thread_attached_path(&record).unwrap()).unwrap(),
        projected_thread_binding("primary-thread")
    );
}

#[tokio::test]
async fn fresh_thread_binding_precedes_first_turn_authorization() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("binding-barrier", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let mut projection = ProxyProjection::new(context, record.clone());

    projection.observe_client(&json!({
        "id": 1,
        "method": "thread/start",
        "params": { "cwd": "/repo" }
    }));
    projection
        .observe_server_before_forward(&json!({
            "id": 1,
            "result": { "thread": { "id": "fresh-thread" } }
        }))
        .await
        .unwrap();

    let attached = thread_attached_path(&record).unwrap();
    assert_eq!(
        fs::read_to_string(attached).unwrap(),
        projected_thread_binding("fresh-thread"),
        "the proxy must publish the bound thread before forwarding the response that enables the first turn"
    );

    let _capability = begin_proxy_capability(&projection.context, &record).unwrap();
    let lifecycle_lock =
        crate::acquire_session_record_lock(&projection.context, &record.id).unwrap();
    let marker = begin_manual_input_section(&projection.context, &record)
        .unwrap()
        .unwrap();
    let first_turn = json!({
        "id": 2,
        "method": "turn/start",
        "params": { "threadId": "fresh-thread", "input": [] }
    });
    let gate = acquire_manual_input_gate(&projection.context, &record, &first_turn)
        .expect("the serialized first turn must recognize the synchronously bound thread");
    drop(gate);
    marker.finish(|| drop(lifecycle_lock));
    projection.finish().await;
}

#[tokio::test]
async fn saturated_projection_preserves_fresh_thread_binding_barrier() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("binding-overflow", &tmp.path().join("server.sock"));
    let mut projection = ProxyProjection::new(context, record.clone());

    projection.task.take().unwrap().abort();
    let (sender, mut blocked_receiver) = mpsc::channel(MAX_PROXY_OBSERVATIONS);
    projection.sender = Some(sender);
    projection.observe_client(&json!({ "id": 1, "method": "thread/start" }));
    for _ in 1..MAX_PROXY_OBSERVATIONS {
        projection.observe_server(&json!({
            "method": "account/rateLimits/updated",
            "params": {
                "rateLimits": {
                    "primary": { "usedPercent": 42.0, "resetsAt": 1_900_000_000_i64 }
                }
            }
        }));
    }

    let (release_worker, wait_for_release) = oneshot::channel();
    let worker = tokio::spawn(async move {
        wait_for_release.await.unwrap();
        while let Some(observation) = blocked_receiver.recv().await {
            if let ProxyObservation::Server {
                persisted_thread: Some(thread_id),
                binding_ack: Some(binding_ack),
                ..
            } = observation
            {
                assert_eq!(thread_id, "fresh-thread");
                binding_ack.send(Ok(())).unwrap();
                return;
            }
        }
        panic!("the saturated queue never admitted the critical binding observation");
    });
    let response_value = json!({
        "id": 1,
        "result": { "thread": { "id": "fresh-thread" } }
    });
    let response = projection.observe_server_before_forward(&response_value);
    tokio::pin!(response);
    assert!(
        futures_util::poll!(&mut response).is_pending(),
        "the critical binding send must wait while every queue slot remains occupied"
    );
    release_worker.send(()).unwrap();
    response.await.unwrap();
    worker.await.unwrap();

    assert_eq!(
        fs::read_to_string(thread_attached_path(&record).unwrap()).unwrap(),
        projected_thread_binding("fresh-thread"),
        "a saturated projection queue must not bypass the pre-forward binding barrier"
    );
}

#[tokio::test]
async fn disabled_projection_never_persists_thread_binding() {
    let tmp = tempfile::TempDir::new().unwrap();
    for disable_before_request in [true, false] {
        let context = CliContext {
            state_dir: tmp.path().join(format!("state-{disable_before_request}")),
            host: None,
        };
        let record = record_with_runtime(
            &format!("disabled-{disable_before_request}"),
            &tmp.path()
                .join(format!("server-{disable_before_request}.sock")),
        );
        let mut projection = ProxyProjection::new(context, record.clone());
        if disable_before_request {
            projection.disable();
        }
        projection.observe_client(&json!({ "id": 1, "method": "thread/start" }));
        if !disable_before_request {
            projection.disable();
        }
        let result = projection
            .observe_server_before_forward(&json!({
                "id": 1,
                "result": { "thread": { "id": "fresh-thread" } }
            }))
            .await;
        assert!(result.is_err());
        assert!(!thread_attached_path(&record).unwrap().exists());
    }

    let context = CliContext {
        state_dir: tmp.path().join("state-worker-close"),
        host: None,
    };
    let record = record_with_runtime("worker-close", &tmp.path().join("worker-close.sock"));
    let mut projection = ProxyProjection::new(context, record.clone());
    projection.observe_client(&json!({ "id": 1, "method": "thread/start" }));
    projection.task.take().unwrap().abort();
    let result = projection
        .observe_server_before_forward(&json!({
            "id": 1,
            "result": { "thread": { "id": "fresh-thread" } }
        }))
        .await;
    assert!(result.is_err());
    assert!(
        !thread_attached_path(&record).unwrap().exists(),
        "a worker closing after the active check must prevent marker publication"
    );
}

#[tokio::test]
async fn rejected_fresh_thread_binding_is_critical() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("rejected-binding", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();

    let mut projection = ProxyProjection::new(context, record);
    projection.observe_client(&json!({ "id": 1, "method": "thread/start" }));
    let result = projection
        .observe_server_before_forward(&json!({
            "id": 1,
            "result": { "thread": { "id": "" } }
        }))
        .await;

    assert!(
        result.is_err(),
        "an invalid response for the required fresh binding must stop forwarding"
    );
    assert!(
        projection.has_fail_close_task(),
        "the critical rejection must retain durable fail-close"
    );
    projection.finish_fail_close().await;
}

#[test]
fn critical_binding_failure_completes_fail_close_before_runtime_shutdown() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("binding-fail-close", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();

    let runtime_context = context.clone();
    let runtime_record = record.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let mut projection = ProxyProjection::new(runtime_context, runtime_record.clone());
            projection.observe_client(&json!({ "id": 1, "method": "thread/start" }));
            let (closed_sender, closed_receiver) = mpsc::channel(1);
            drop(closed_receiver);
            projection.sender = Some(closed_sender);

            let result = projection
                .observe_server_before_forward(&json!({
                    "id": 1,
                    "result": { "thread": { "id": "fresh-thread" } }
                }))
                .await;
            assert!(result.is_err());
            projection.finish_fail_close().await;
        });
    })
    .join()
    .unwrap();

    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(!view.enabled);
    assert_eq!(view.state, "terminal_failure");
    assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
}

#[test]
fn critical_binding_failure_returns_before_fail_close_lock_retry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("binding-retry", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();

    let runtime_context = context.clone();
    let runtime_record = record.clone();
    std::thread::spawn(move || {
        let record_lock =
            crate::acquire_session_record_lock(&runtime_context, &runtime_record.id).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let mut projection = ProxyProjection::new(runtime_context, runtime_record.clone());
            projection.observe_client(&json!({ "id": 1, "method": "thread/start" }));
            let (closed_sender, closed_receiver) = mpsc::channel(1);
            drop(closed_receiver);
            projection.sender = Some(closed_sender);

            let observed = tokio::time::timeout(
                Duration::from_millis(100),
                projection.observe_server_before_forward(&json!({
                    "id": 1,
                    "result": { "thread": { "id": "fresh-thread" } }
                })),
            )
            .await;
            assert!(
                observed.is_ok(),
                "critical binding failure must return promptly"
            );
            assert!(observed.unwrap().is_err());
            let fail_close = projection.finish_fail_close();
            tokio::pin!(fail_close);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut fail_close)
                    .await
                    .is_err(),
                "durable fail-close should remain pending while the record lock is held"
            );
            drop(record_lock);
            fail_close.await;
        });
    })
    .join()
    .unwrap();

    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(!view.enabled);
    assert_eq!(view.state, "terminal_failure");
    assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
}

#[test]
fn disabled_fresh_projection_finish_waits_for_durable_fail_close() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("disabled-finish", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();

    let runtime_context = context.clone();
    let runtime_record = record.clone();
    std::thread::spawn(move || {
        let record_lock =
            crate::acquire_session_record_lock(&runtime_context, &runtime_record.id).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let mut projection = ProxyProjection::new(runtime_context, runtime_record);
            projection.disable();
            let finish = projection.finish();
            tokio::pin!(finish);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut finish)
                    .await
                    .is_err(),
                "fresh disabled projection finalization must retain the fail-close retry"
            );
            drop(record_lock);
            finish.await;
        });
    })
    .join()
    .unwrap();

    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(!view.enabled);
    assert_eq!(view.state, "terminal_failure");
    assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
}

#[test]
fn disabled_bound_projection_finish_waits_for_durable_fail_close() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("disabled-bound-finish", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    bind_thread(&record, "bound-thread").unwrap();

    let runtime_context = context.clone();
    let runtime_record = record.clone();
    std::thread::spawn(move || {
        let record_lock =
            crate::acquire_session_record_lock(&runtime_context, &runtime_record.id).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let mut projection = ProxyProjection::new(runtime_context, runtime_record);
            projection.disable();
            let finish = projection.finish();
            tokio::pin!(finish);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut finish)
                    .await
                    .is_err(),
                "bound disabled projection finalization must retain the fail-close retry"
            );
            drop(record_lock);
            finish.await;
        });
    })
    .join()
    .unwrap();

    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(!view.enabled);
    assert_eq!(view.state, "terminal_failure");
    assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
}

#[tokio::test]
async fn worker_failure_finish_waits_for_durable_fail_close() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("worker-fail-close", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let record_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();

    let mut projection = ProxyProjection::new(context.clone(), record.clone());
    projection.observe_client(&json!({
        "method": "turn/start",
        "params": { "threadId": "thread-a" }
    }));
    projection.observe_client(&json!({
        "method": "turn/start",
        "params": { "threadId": "thread-b" }
    }));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let release_lock = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        drop(record_lock);
    });

    projection.finish().await;
    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(!view.enabled);
    assert_eq!(view.state, "terminal_failure");
    assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
    release_lock.await.unwrap();
}

#[tokio::test]
async fn critical_binding_failure_closes_listener_before_fail_close_retry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("critical-listener.sock");
    let upstream_listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("critical-listener", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    write_create_bootstrap_marker(&record);

    let listen = upstream.with_extension("proxy");
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: listen.clone(),
    };
    let proxy_context = context.clone();
    let mut proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server_record = record.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = upstream_listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["method"], "thread/start");
        bind_thread(&server_record, "conflicting-thread").unwrap();
        respond(
            &mut socket,
            &request,
            json!({ "thread": { "id": "fresh-thread" } }),
        )
        .await;
        let closed = tokio::time::timeout(Duration::from_secs(1), socket.next())
            .await
            .expect("the upstream connection must close promptly");
        assert!(
            closed.is_none() || closed.is_some_and(|message| message.is_err()),
            "critical binding failure must close the upstream connection"
        );
    });
    let proxy_stream = connect_socket(&listen).await.unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    let record_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "thread/start",
            "params": { "cwd": "/repo" }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(1), tui.next())
        .await
        .expect("the TUI connection must close promptly");
    assert!(
        closed.is_none() || closed.is_some_and(|message| message.is_err()),
        "critical binding failure must not forward the successful response"
    );

    let reconnect = tokio::time::timeout(
        Duration::from_millis(100),
        tokio::net::UnixStream::connect(&listen),
    )
    .await
    .expect("a retrying client must fail promptly");
    assert!(
        reconnect.is_err(),
        "the proxy listener must stop accepting while durable fail-close retries"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut proxy)
            .await
            .is_err(),
        "durable fail-close must remain pending while the record lock is held"
    );

    drop(record_lock);
    assert!(
        tokio::time::timeout(Duration::from_secs(3), &mut proxy)
            .await
            .expect("the proxy must finish after fail-close acquires the record lock")
            .unwrap()
            .is_err()
    );
    server.await.unwrap();
    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(!view.enabled);
    assert_eq!(view.state, "terminal_failure");
    assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
}

#[tokio::test]
async fn oversized_server_message_projects_only_bounded_fields() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("oversized-projection", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    let mut projection = ProxyProjection::new(context.clone(), record.clone());
    let value = json!({
        "padding": "x".repeat(MAX_PROXY_OBSERVATION_BYTES * 5),
        "method": "turn/completed",
        "params": {
            "threadId": "thread-a",
            "turn": { "id": "turn-a", "status": "completed" }
        }
    });

    projection.observe_server(&value);
    tokio::time::sleep(Duration::from_millis(100)).await;
    projection.finish().await;

    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(view.enabled);
    assert_eq!(view.state, "enabled");
    assert_eq!(view.failure_reason, None);
}

#[tokio::test]
async fn oversized_selected_unique_observation_fails_closed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("oversized-unique", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    let mut projection = ProxyProjection::new(context.clone(), record.clone());

    projection.observe_server(&json!({
        "method": "turn/completed",
        "params": {
            "threadId": "thread-a",
            "turn": {
                "id": "turn-a",
                "status": "failed",
                "error": {
                    "codexErrorInfo": "x".repeat(MAX_PROXY_OBSERVATION_BYTES)
                }
            }
        }
    }));
    let mut failed_closed = false;
    for _ in 0..20 {
        let view = crate::auto_resume::view_for_record(&context, &record);
        if view.state == "terminal_failure" {
            assert!(!view.enabled);
            assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
            failed_closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        failed_closed,
        "oversized selected unique observation did not fail closed"
    );
    settle_projection(&mut projection).await;
}

#[tokio::test]
async fn saturated_projection_coalesces_repeatable_usage_updates() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("usage-coalesce", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    let mut projection = ProxyProjection::new(context.clone(), record.clone());

    for _ in 0..(MAX_PROXY_OBSERVATIONS + 4) {
        projection.observe_server(&json!({
            "method": "account/rateLimits/updated",
            "params": {
                "rateLimits": {
                    "primary": { "usedPercent": 42.0, "resetsAt": 1_900_000_000_i64 }
                }
            }
        }));
    }
    projection.finish().await;

    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(view.enabled);
    assert_eq!(view.state, "enabled");
    assert_eq!(view.failure_reason, None);
}

#[tokio::test]
async fn saturated_projection_still_fails_closed_for_unique_events() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("unique-overflow", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    let mut projection = ProxyProjection::new(context.clone(), record.clone());

    for index in 0..(MAX_PROXY_OBSERVATIONS + 4) {
        projection.observe_server(&json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": { "id": format!("turn-{index}"), "status": "completed" }
            }
        }));
    }
    let mut failed_closed = false;
    for _ in 0..20 {
        let view = crate::auto_resume::view_for_record(&context, &record);
        if view.state == "terminal_failure" {
            assert!(!view.enabled);
            assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
            failed_closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        failed_closed,
        "unique projection overflow did not fail closed"
    );
    settle_projection(&mut projection).await;
}

#[tokio::test]
async fn projection_fail_close_retries_after_timed_lock_contention() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("projection-retry", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let task_context = context.clone();
    let task_record = record.clone();
    let fail = tokio::spawn(async move {
        fail_closed_projection(&task_context, &task_record).await;
    });

    tokio::time::sleep(Duration::from_millis(1_100)).await;
    drop(lock);
    tokio::time::timeout(Duration::from_secs(3), fail)
        .await
        .expect("fail-close retry did not finish")
        .unwrap();
    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(!view.enabled);
    assert_eq!(view.state, "terminal_failure");
    assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
}

#[tokio::test]
async fn projection_fail_close_marks_unknown_while_activity_lock_is_held() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("projection-activity-lock", &tmp.path().join("server.sock"));
    let dir = crate::session_dir(&context, &record.id);
    fs::create_dir_all(&dir).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join(".activity.lock"))
        .unwrap();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);

    tokio::time::timeout(
        Duration::from_secs(2),
        fail_closed_projection(&context, &record),
    )
    .await
    .expect("activity-lock fail-close must not hang");
    assert_eq!(
        crate::activity::activity_status(&context, &record.id)
            .unwrap()
            .turn_state
            .phase,
        crate::activity::TurnPhase::Unknown
    );
    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(!view.enabled);
    assert_eq!(view.state, "terminal_failure");
    assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));

    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) }, 0);
}

#[tokio::test]
async fn open_usage_lock_contention_is_retryable_without_disabling_projection() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("usage-wake-busy", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();

    wake_from_open_usage(
        &context,
        &record,
        &UsageSnapshot {
            authoritative: true,
            has_exhausted_windows: false,
            exhausted_reset_epochs: Vec::new(),
            soonest_reset_epoch: None,
        },
    )
    .await
    .expect("advisory open-usage wake should defer while the lifecycle lock is busy");
    drop(lock);

    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(view.enabled);
    assert_eq!(view.state, "enabled");
    assert_eq!(view.failure_reason, None);
}

#[tokio::test]
async fn open_usage_burst_during_lifecycle_lock_keeps_projection_enabled() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("usage-wake-burst", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    let mut projection = ProxyProjection::new(context.clone(), record.clone());
    projection.observe_client(&json!({
        "method": "turn/start",
        "params": { "threadId": "thread-a" }
    }));
    tokio::time::sleep(Duration::from_millis(25)).await;
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    projection.observe_server(&json!({
        "method": "account/rateLimits/updated",
        "params": {
            "rateLimits": {
                "primary": {
                    "usedPercent": 42.0,
                    "resetsAt": 1_900_000_000_i64
                }
            }
        }
    }));
    tokio::time::sleep(Duration::from_millis(25)).await;

    for index in 0..(MAX_PROXY_OBSERVATIONS + 4) {
        projection.observe_server(&json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-a",
                "turn": {
                    "id": format!("turn-{index}"),
                    "status": "completed"
                }
            }
        }));
        tokio::task::yield_now().await;
    }
    drop(lock);
    tokio::time::sleep(Duration::from_millis(100)).await;
    projection.finish().await;

    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(view.enabled);
    assert_eq!(view.state, "enabled");
    assert_eq!(view.failure_reason, None);
}

#[tokio::test]
async fn projection_fail_close_stops_after_permanent_state_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("projection-permanent", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    fs::remove_dir_all(crate::session_dir(&context, &record.id)).unwrap();

    tokio::time::timeout(
        Duration::from_millis(250),
        fail_closed_projection(&context, &record),
    )
    .await
    .expect("permanent projection state error must terminate the retry task");
}

#[test]
fn attached_thread_binding_rejects_a_different_reconnect_thread() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("codex.sock");
    let record = record_with_runtime("thread-binding", &socket);

    bind_thread(&record, "raw-thread-a").unwrap();
    let binding = fs::read_to_string(socket.with_extension("attached")).unwrap();
    assert_eq!(binding, projected_thread_binding("raw-thread-a"));
    assert!(!binding.contains("raw-thread-a"));
    let err = bind_thread(&record, "raw-thread-b").unwrap_err();
    assert_eq!(
        err,
        "Codex loaded thread did not match the attached runtime"
    );
}

#[tokio::test]
async fn app_server_thread_binding_persists_provider_resume_identity() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let socket = tmp.path().join("codex.sock");
    let record = record_with_runtime("thread-resume-identity", &socket);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();

    bind_thread_and_persist_resume(&context, &record, "raw-recoverable-thread")
        .await
        .unwrap();

    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let resume = persisted
        .provider_resume
        .as_ref()
        .expect("app-server binding must persist provider resume metadata");
    assert_eq!(resume.provider, "codex");
    assert_eq!(resume.session_id, "raw-recoverable-thread");
    assert_eq!(resume.capture_method, "codex-app-server-thread-binding");
    assert_eq!(
        resume.resume_args,
        crate::canonical_provider_resume_args(
            crate::AgentKind::Codex,
            &record.cwd,
            "raw-recoverable-thread",
        )
        .unwrap()
    );

    let sidecar = fs::read_to_string(
        crate::session_dir(&context, &record.id).join(crate::SESSION_RESUME_FILE),
    )
    .unwrap();
    assert!(sidecar.contains("raw-recoverable-thread"));
    let attached = fs::read_to_string(socket.with_extension("attached")).unwrap();
    assert_eq!(attached, projected_thread_binding("raw-recoverable-thread"));
    assert!(!attached.contains("raw-recoverable-thread"));
}

#[test]
fn app_server_thread_binding_preserves_matching_resume_provenance() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let socket = tmp.path().join("codex.sock");
    let expected = record_with_runtime("thread-resume-preserved", &socket);
    let mut current = expected.clone();
    current.provider_resume = Some(ProviderResume {
        provider: "codex".to_string(),
        session_id: "raw-existing-thread".to_string(),
        captured_at: "2030-01-01T00:00:01Z".to_string(),
        capture_method: "codex-user-prompt-submit-hook".to_string(),
        resume_args: canonical_provider_resume_args(
            AgentKind::Codex,
            &current.cwd,
            "raw-existing-thread",
        )
        .unwrap(),
        extra: BTreeMap::from([("retained".to_string(), json!(true))]),
    });
    fs::create_dir_all(crate::session_dir(&context, &current.id)).unwrap();
    crate::write_session_record(&context, &current).unwrap();

    persist_bound_thread_resume(&context, &expected, "raw-existing-thread").unwrap();

    let persisted = crate::load_session_record(&context, &expected.id).unwrap();
    let resume = persisted.provider_resume.expect("provider resume");
    assert_eq!(resume.capture_method, "codex-user-prompt-submit-hook");
    assert_eq!(resume.extra.get("retained"), Some(&json!(true)));
    assert!(socket.with_extension("attached").is_file());
}

#[test]
fn app_server_thread_binding_rejects_conflicting_resume_without_marker() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let socket = tmp.path().join("codex.sock");
    let expected = record_with_runtime("thread-resume-conflict", &socket);
    let mut current = expected.clone();
    current.provider_resume = Some(ProviderResume {
        provider: "codex".to_string(),
        session_id: "raw-existing-thread".to_string(),
        captured_at: "2030-01-01T00:00:01Z".to_string(),
        capture_method: "codex-user-prompt-submit-hook".to_string(),
        resume_args: canonical_provider_resume_args(
            AgentKind::Codex,
            &current.cwd,
            "raw-existing-thread",
        )
        .unwrap(),
        extra: BTreeMap::new(),
    });
    fs::create_dir_all(crate::session_dir(&context, &current.id)).unwrap();
    crate::write_session_record(&context, &current).unwrap();

    let error =
        persist_bound_thread_resume(&context, &expected, "raw-different-thread").unwrap_err();

    assert_eq!(error.code(), "codex-app-server-resume-identity-conflict");
    assert!(!socket.with_extension("attached").exists());
    let persisted = crate::load_session_record(&context, &expected.id).unwrap();
    assert_eq!(
        persisted
            .provider_resume
            .as_ref()
            .map(|resume| resume.session_id.as_str()),
        Some("raw-existing-thread")
    );
}

#[test]
fn stale_app_server_runtime_cannot_bind_thread_identity() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let socket = tmp.path().join("codex.sock");
    let expected = record_with_runtime("thread-resume-stale", &socket);
    let mut replacement = expected.clone();
    replacement.runtime.as_mut().expect("runtime").launch_id = "replacement-runtime".to_string();
    fs::create_dir_all(crate::session_dir(&context, &replacement.id)).unwrap();
    crate::write_session_record(&context, &replacement).unwrap();

    let error = persist_bound_thread_resume(&context, &expected, "raw-stale-thread").unwrap_err();

    assert_eq!(error.code(), "session-runtime-changed");
    assert!(!socket.with_extension("attached").exists());
    let persisted = crate::load_session_record(&context, &expected.id).unwrap();
    assert!(persisted.provider_resume.is_none());
}

#[test]
fn usage_projection_is_authoritative_only_for_well_formed_response() {
    assert!(!usage_snapshot(&json!({})).authoritative);
    assert!(!usage_snapshot(&json!({ "rateLimits": {} })).authoritative);
    for malformed in [
        json!({ "usedPercent": "100" }),
        json!({ "resetsAt": 1_900_000_100 }),
        json!([]),
    ] {
        let snapshot = usage_snapshot(&json!({
            "rateLimits": {
                "primary": { "usedPercent": 42.0, "resetsAt": 1_900_000_000 },
                "secondary": malformed
            }
        }));
        assert!(!snapshot.authoritative, "snapshot={snapshot:?}");
    }
    let snapshot = usage_snapshot(&json!({
        "rateLimits": {
            "primary": { "usedPercent": 100.0, "resetsAt": 1_900_000_000 },
            "secondary": { "usedPercent": 42.0, "resetsAt": 1_900_000_100 }
        }
    }));
    assert!(snapshot.authoritative);
    assert!(snapshot.has_exhausted_windows);
    assert_eq!(snapshot.exhausted_reset_epochs, vec![1_900_000_000]);

    let snapshot = usage_snapshot(&json!({
        "rateLimits": {
            "primary": { "usedPercent": 42.0, "resetsAt": 1_900_000_000 }
        },
        "rateLimitsByLimitId": {
            "codex": {
                "primary": { "usedPercent": 100.0, "resetsAt": 1_900_000_200 },
                "secondary": { "usedPercent": 100.0, "resetsAt": 1_900_000_300 }
            }
        }
    }));
    assert!(snapshot.authoritative);
    assert!(snapshot.has_exhausted_windows);
    assert_eq!(
        snapshot.exhausted_reset_epochs,
        vec![1_900_000_200, 1_900_000_300]
    );

    assert!(
        !usage_snapshot(&json!({
            "rateLimits": { "primary": { "usedPercent": 42.0 } },
            "rateLimitsByLimitId": { "codex": { "primary": { "usedPercent": "100" } } }
        }))
        .authoritative
    );
}

#[test]
fn exact_runtime_failure_never_arms_a_sibling_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let target = record_with_runtime("codex-target", &tmp.path().join("target.sock"));
    let sibling = record_with_runtime("codex-sibling", &tmp.path().join("sibling.sock"));
    for record in [&target, &sibling] {
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, record).unwrap();
        crate::activity::activate_runtime(&context, record).unwrap();
        crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z")
            .unwrap();
    }

    let mut reducer = FailureReducer::new("target-thread");
    assert_eq!(
        reducer.ingest(&json!({
            "method": "error",
            "params": {
                "threadId": "target-thread",
                "turnId": "target-turn",
                "willRetry": false,
                "error": { "codexErrorInfo": "usageLimitExceeded" }
            }
        })),
        None
    );
    let failure = reducer
        .ingest(&json!({
            "method": "turn/completed",
            "params": {
                "threadId": "target-thread",
                "turn": { "id": "target-turn", "status": "failed" }
            }
        }))
        .unwrap();
    crate::activity::ingest_codex_app_server_failure_with_kind(
        &context,
        &target.id,
        &target.runtime.as_ref().unwrap().launch_id,
        &failure.thread_id,
        &failure.turn_id,
        failure.kind,
    )
    .unwrap();

    assert_eq!(
        crate::auto_resume::pending_sessions(&context, 1_893_456_000)
            .unwrap()
            .usage_ids,
        vec![target.id]
    );
    let sibling_view = crate::auto_resume::view_for_record(&context, &sibling);
    assert!(sibling_view.enabled);
    assert_eq!(sibling_view.state, "enabled");
}

#[tokio::test]
async fn resumed_proxy_binds_exact_identity_before_first_tui_request() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("resume-bind.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("resume-bind", &path);
    record.provider_resume = Some(ProviderResume {
        provider: "codex".into(),
        session_id: "resumed-thread".into(),
        captured_at: "2030-01-01T00:00:00Z".into(),
        capture_method: "test".into(),
        resume_args: canonical_provider_resume_args(
            AgentKind::Codex,
            &record.cwd,
            "resumed-thread",
        )
        .unwrap(),
        extra: BTreeMap::new(),
    });
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let server_record = record.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["method"], "thread/resume");
        assert_eq!(
            fs::read_to_string(thread_attached_path(&server_record).unwrap()).unwrap(),
            projected_thread_binding("resumed-thread")
        );
        respond(
            &mut socket,
            &request,
            json!({"thread":{"id":"resumed-thread"}}),
        )
        .await;
        socket.close(None).await.unwrap();
    });
    let proxy_context = context.clone();
    let proxy_id = record.id.clone();
    let proxy_path = path.with_extension("proxy");
    let args = crate::cli::CodexAppServerProxyArgs {
        id: proxy_id,
        upstream: path,
        listen: proxy_path.clone(),
    };
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, args).await });
    let stream = connect_socket(&proxy_path).await.unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", stream)
        .await
        .unwrap();
    send_json(
        &mut tui,
        json!({"id":1,"method":"thread/resume","params":{"threadId":"resumed-thread"}}),
    )
    .await
    .unwrap();
    assert_eq!(
        receive_json(&mut tui).await["result"]["thread"]["id"],
        "resumed-thread"
    );
    assert!(manual_input_request_matches_bound_thread(
        &context,
        &record,
        &json!({"id":2,"method":"turn/start","params":{"threadId":"resumed-thread","input":[]}})
    ));
    server.await.unwrap();
    let _ = proxy.await.unwrap();
}

#[tokio::test]
async fn native_new_rebinds_marker_resume_and_projection_before_response_forwarding() {
    for pending_bootstrap in [false, true] {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let mut record = record_with_runtime("native-new", &tmp.path().join("server.sock"));
        record.provider_resume = Some(ProviderResume {
            provider: "codex".into(),
            session_id: "old-thread".into(),
            captured_at: "2030-01-01T00:00:00Z".into(),
            capture_method: "test".into(),
            resume_args: canonical_provider_resume_args(
                AgentKind::Codex,
                &record.cwd,
                "old-thread",
            )
            .unwrap(),
            extra: BTreeMap::new(),
        });
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        crate::activity::activate_runtime(&context, &record).unwrap();
        bind_thread(&record, "old-thread").unwrap();
        let mut projection = ProxyProjection::new(context.clone(), record.clone());
        projection.requires_thread_binding = pending_bootstrap;
        projection.observe_client(
            &json!({"id":1,"method":"thread/start","params":{"threadSource":"user"}}),
        );
        projection
            .observe_server_before_forward(&json!({"id":1,"result":{"thread":{"id":"new-thread"}}}))
            .await
            .unwrap();
        let current = crate::load_session_record(&context, &record.id).unwrap();
        assert_eq!(
            current.provider_resume.as_ref().unwrap().session_id,
            "new-thread"
        );
        assert_eq!(
            fs::read_to_string(thread_attached_path(&record).unwrap()).unwrap(),
            projected_thread_binding("new-thread")
        );
        assert_eq!(
            crate::activity::state_for_view(&context, &current)
                .unwrap()
                .phase,
            crate::activity::TurnPhase::Waiting
        );
        projection.observe_client(
            &json!({"id":2,"method":"turn/start","params":{"threadId":"new-thread"}}),
        );
        projection
            .observe_server_before_forward(&json!({"id":2,"result":{"turn":{"id":"new-turn"}}}))
            .await
            .unwrap();
        projection.finish().await;
        assert!(!crate::activity::runtime_is_unhealthy(&context, &current));
    }
}

#[test]
fn legacy_proxy_capability_cannot_authorize_conversation_mutation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("compat-proxy", &tmp.path().join("socket"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    let guard = begin_proxy_capability(&context, &record).unwrap();
    assert!(live_conversation_capability(&context, &record));
    let path = proxy_capability_path(&context, &record);
    let mut marker: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    marker
        .as_object_mut()
        .unwrap()
        .remove("conversation_capability");
    fs::write(&path, serde_json::to_vec(&marker).unwrap()).unwrap();
    assert!(live_proxy_capability(&context, &record));
    assert!(!live_conversation_capability(&context, &record));
    drop(guard);
    assert!(!live_conversation_capability(&context, &record));
}

#[tokio::test]
async fn rejected_live_thread_and_failed_codex_commit_recover_through_rebind() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("recovery.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("mismatch-recovery", &path);
    record.provider_resume = Some(ProviderResume {
        provider: "codex".into(),
        session_id: "old-thread".into(),
        captured_at: "2030-01-01T00:00:00Z".into(),
        capture_method: "test".into(),
        resume_args: canonical_provider_resume_args(AgentKind::Codex, &record.cwd, "old-thread")
            .unwrap(),
        extra: BTreeMap::new(),
    });
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    bind_thread(&record, "old-thread").unwrap();
    let _capability = begin_proxy_capability(&context, &record).unwrap();
    let marker = begin_manual_input_section(&context, &record)
        .unwrap()
        .unwrap();
    let request =
        json!({"id":1,"method":"turn/start","params":{"threadId":"live-thread","input":[]}});
    assert!(matches!(
        acquire_turn_start_gate(&context, &record, &request),
        Err(TuiMutationRejection::ManualMarkerThreadMismatch)
    ));
    drop(marker);
    let attached = fs::read(thread_attached_path(&record).unwrap()).unwrap();
    let snapshot_path = crate::session_dir(&context, &record.id).join("activity.json");
    let snapshot = fs::read(&snapshot_path).unwrap();
    crate::fail_session_record_write_on_nth_call(1);
    assert!(crate::conversation::observe_native(&context, &record, "live-thread").is_err());
    assert_eq!(
        fs::read(thread_attached_path(&record).unwrap()).unwrap(),
        attached
    );
    assert_eq!(fs::read(&snapshot_path).unwrap(), snapshot);
    assert_eq!(
        crate::load_session_record(&context, &record.id)
            .unwrap()
            .provider_resume
            .unwrap()
            .session_id,
        "old-thread"
    );
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let initialize = receive_json(&mut socket).await;
            respond(&mut socket, &initialize, json!({})).await;
            assert_eq!(receive_json(&mut socket).await["method"], "initialized");
            let loaded = receive_json(&mut socket).await;
            respond(
                &mut socket,
                &loaded,
                json!({"data":["old-thread","live-thread"],"nextCursor":null}),
            )
            .await;
            let read = receive_json(&mut socket).await;
            assert_eq!(read["params"]["threadId"], "live-thread");
            respond(
                &mut socket,
                &read,
                json!({"thread":{"id":"live-thread","status":{"type":"idle"}}}),
            )
            .await;
        }
    });
    let tmux = tmp.path().join("tmux");
    fs::write(&tmux, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let command_context = context.clone();
    let command_id = record.id.clone();
    let exit = tokio::task::spawn_blocking(move || {
        crate::conversation::run(
            &command_context,
            crate::cli::ConversationArgs {
                id: command_id,
                expect_idle: true,
                timeout: 1,
                tmux_bin: Some(tmux),
                format: crate::OutputFormat::Json,
            },
            false,
        )
    })
    .await
    .unwrap();
    assert_eq!(exit, 0);
    server.await.unwrap();
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(current.provider_resume.unwrap().session_id, "live-thread");
    assert_eq!(
        fs::read_to_string(thread_attached_path(&record).unwrap()).unwrap(),
        projected_thread_binding("live-thread")
    );
}

#[tokio::test]
async fn conversation_recovery_probes_only_unambiguous_idle_provider_threads() {
    for (ids, returned_id, status, succeeds) in [
        (vec!["live-thread"], "live-thread", "idle", true),
        (vec!["live-thread"], "other-thread", "idle", false),
        (vec!["live-thread"], "live-thread", "active", false),
        (
            vec!["old-thread", "live-thread"],
            "live-thread",
            "idle",
            false,
        ),
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("probe.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = record_with_runtime("conversation-probe", &path);
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        let count = ids.len();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let initialize = receive_json(&mut socket).await;
            respond(&mut socket, &initialize, json!({})).await;
            assert_eq!(receive_json(&mut socket).await["method"], "initialized");
            let loaded = receive_json(&mut socket).await;
            assert_eq!(loaded["method"], "thread/loaded/list");
            respond(&mut socket, &loaded, json!({"data":ids,"nextCursor":null})).await;
            if count == 1 {
                let read = receive_json(&mut socket).await;
                assert_eq!(read["method"], "thread/read");
                assert_eq!(read["params"]["includeTurns"], false);
                respond(
                    &mut socket,
                    &read,
                    json!({"thread":{"id":returned_id,"status":{"type":status}}}),
                )
                .await;
            }
        });
        let outcome =
            tokio::task::spawn_blocking(move || probe_idle_conversation(&context, &record, None))
                .await
                .unwrap();
        assert_eq!(outcome.is_ok(), succeeds);
        if let Ok(id) = outcome {
            assert_eq!(id, "live-thread");
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn conversation_recovery_refuses_paginated_thread_list() {
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("probe-pagination.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("conversation-probe-pagination", &path);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({"data":["live-thread"],"nextCursor":"next-page"}),
        )
        .await;
        assert!(
            matches!(
                socket.next().await,
                None | Some(Ok(Message::Close(_))) | Some(Err(_))
            ),
            "recovery must not read a candidate from an incomplete thread list"
        );
    });

    let outcome =
        tokio::task::spawn_blocking(move || probe_idle_conversation(&context, &record, None))
            .await
            .unwrap();

    assert_eq!(
        outcome.unwrap_err().code(),
        "conversation-live-identity-unverified"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn control_reconnect_resumes_the_bound_loaded_thread() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("reconnect.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("reconnect", &socket);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "raw-thread-a").unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let loaded = receive_json(&mut socket).await;
        respond(
            &mut socket,
            &loaded,
            json!({
                "data": ["raw-thread-decoy", "raw-thread-a"],
                "nextCursor": null
            }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        assert_eq!(resume["method"], "thread/resume");
        assert_eq!(resume["params"]["threadId"], "raw-thread-a");
        assert_eq!(resume["params"]["excludeTurns"], true);
        respond(&mut socket, &resume, json!({})).await;
        for _ in 0..2 {
            let usage = receive_json(&mut socket).await;
            assert_eq!(usage["method"], "account/rateLimits/read");
            respond(
                &mut socket,
                &usage,
                json!({
                    "rateLimits": {
                        "primary": { "usedPercent": 100, "resetsAt": 1_900_000_000 }
                    }
                }),
            )
            .await;
        }
    });
    let (handle, commands, ready) = starting_control_channel();
    let control = tokio::spawn(run_control(
        context.clone(),
        record.clone(),
        commands,
        ready,
    ));

    let usage = handle.usage().await.unwrap();
    assert!(usage.authoritative);
    assert!(usage.has_exhausted_windows);
    server.await.unwrap();
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let resume = persisted
        .provider_resume
        .as_ref()
        .expect("control reconnect must persist the loaded thread identity");
    assert_eq!(resume.session_id, "raw-thread-a");
    assert_eq!(resume.capture_method, PROVIDER_RESUME_CAPTURE_METHOD);
    assert!(
        crate::session_dir(&context, &record.id)
            .join(crate::SESSION_RESUME_FILE)
            .is_file()
    );
    drop(handle);
    control.abort();
    let _ = control.await;
}

#[tokio::test]
async fn control_reconnect_applies_external_auth_before_thread_discovery() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(
        &broker,
        "#!/bin/sh\nprintf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":\"acct1\",\"access_token\":\"token-acct1\",\"chatgpt_account_id\":\"workspace-acct1\",\"plan\":\"team\"}'\n",
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![broker.to_string_lossy().into_owned()]).unwrap(),
    );
    let socket_path = tmp.path().join("auth-reconnect.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("auth-reconnect", &socket_path);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "raw-thread-auth").unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let login = receive_json(&mut socket).await;
        assert_eq!(login["method"], "account/login/start");
        assert_eq!(login["params"]["accessToken"], "token-acct1");
        assert_eq!(login["params"]["chatgptAccountId"], "workspace-acct1");
        respond(&mut socket, &login, json!({ "type": "chatgptAuthTokens" })).await;
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({ "data": ["raw-thread-auth"], "nextCursor": null }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        respond(&mut socket, &resume, json!({})).await;
        for _ in 0..2 {
            let usage = receive_json(&mut socket).await;
            assert_eq!(usage["method"], "account/rateLimits/read");
            respond(
                &mut socket,
                &usage,
                json!({ "rateLimits": { "primary": { "usedPercent": 1 } } }),
            )
            .await;
        }
        let prompt = receive_json(&mut socket).await;
        assert_eq!(prompt["method"], "turn/start");
        assert_eq!(
            prompt["params"]["input"][0]["text"],
            "next selected-account prompt"
        );
        respond(
            &mut socket,
            &prompt,
            json!({ "turn": { "id": "selected-account-turn" } }),
        )
        .await;
    });
    let (handle, commands, ready) = starting_control_channel();
    let control_context = context.clone();
    let control_record = record.clone();
    let control = tokio::spawn(run_control(
        control_context,
        control_record,
        commands,
        ready,
    ));
    let usage = handle.usage().await;
    if let Err(error) = usage.as_ref() {
        drop(handle);
        let server_result = server.await;
        let control_result = control.await;
        panic!("usage failed: {error}; server={server_result:?}; control={control_result:?}");
    }
    assert!(usage.unwrap().authoritative);
    assert_eq!(
        handle
            .submit_prompt("next selected-account prompt")
            .await
            .unwrap(),
        "selected-account-turn"
    );
    server.await.unwrap();
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.state, "bound");
    assert_eq!(view.selected_account.as_deref(), Some("acct1"));
    drop(handle);
    control.abort();
    let _ = control.await;
}

#[tokio::test]
async fn proxy_holds_queued_account_turn_until_apply_then_forwards_once() {
    let env_lock = GlobalStateLock::new();
    let broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("queued-turn.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("queued-turn", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    crate::codex_account::queue_next_account_with_unbound(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        "omega",
    )
    .unwrap();
    drop(broker);
    let without_proxy_broker = EnvGuard::remove(&env_lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");

    let (held_tx, held_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), receive_json(&mut socket))
                .await
                .is_err(),
            "turn/start reached the upstream before the queued account applied"
        );
        held_tx.send(()).unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["id"], 1);
        assert_eq!(request["method"], "turn/start");
        respond(
            &mut socket,
            &request,
            json!({ "turn": { "id": "queued-turn", "status": "inProgress" } }),
        )
        .await;
        let request = receive_json(&mut socket).await;
        assert_eq!(request["id"], 2);
        assert_eq!(
            request["method"], "thread/read",
            "the held turn/start must be forwarded exactly once"
        );
        respond(&mut socket, &request, json!({ "ok": true })).await;
        socket.close(None).await.unwrap();
    });
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    held_rx.await.unwrap();

    drop(without_proxy_broker);
    let _daemon_broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let launch_id = &record.runtime.as_ref().unwrap().launch_id;
    let applying = crate::codex_account::begin_next_apply(&context, &record.id, launch_id)
        .unwrap()
        .expect("the queued account should become applying");
    crate::codex_account::finish_next_apply(
        &context,
        &record.id,
        launch_id,
        &applying.account,
        applying.revision,
        applying.intent_id.as_deref().unwrap(),
        Ok(()),
    )
    .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(1), receive_json(&mut tui))
        .await
        .expect("the turn should forward promptly after account apply");
    assert_eq!(response["result"]["turn"]["id"], "queued-turn");
    tui.send(Message::Text(
        json!({ "id": 2, "method": "thread/read", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 2);
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test]
async fn proxy_holds_account_queue_until_in_flight_turn_start_is_acknowledged() {
    let env_lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("in-flight-turn.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("in-flight-turn", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();

    let (forwarded_tx, forwarded_rx) = tokio::sync::oneshot::channel();
    let (collision_tx, collision_rx) = tokio::sync::oneshot::channel();
    let (collision_seen_tx, collision_seen_rx) = tokio::sync::oneshot::channel();
    let (acknowledge_tx, acknowledge_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["method"], "turn/start");
        forwarded_tx.send(()).unwrap();
        collision_rx.await.unwrap();
        send_json(
            &mut socket,
            json!({
                "id": 1,
                "method": "server/ping",
                "params": {}
            }),
        )
        .await
        .unwrap();
        collision_seen_tx.send(()).unwrap();
        acknowledge_rx.await.unwrap();
        send_json(
            &mut socket,
            json!({
                "method": "turn/started",
                "params": {
                    "threadId": "thread-a",
                    "turn": { "id": "in-flight-turn", "status": "inProgress" }
                }
            }),
        )
        .await
        .unwrap();
        respond(
            &mut socket,
            &request,
            json!({ "turn": { "id": "in-flight-turn", "status": "inProgress" } }),
        )
        .await;
        let request = receive_json(&mut socket).await;
        assert_eq!(request["method"], "thread/read");
        respond(&mut socket, &request, json!({ "ok": true })).await;
        socket.close(None).await.unwrap();
    });
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    forwarded_rx.await.unwrap();

    let queue_context = context.clone();
    let queue_id = record.id.clone();
    let queue_launch_id = record.runtime.as_ref().unwrap().launch_id.clone();
    let (queued_tx, queued_rx) = std::sync::mpsc::channel();
    let queue = std::thread::spawn(move || {
        queued_tx
            .send(crate::codex_account::queue_next_account_with_unbound(
                &queue_context,
                &queue_id,
                &queue_launch_id,
                "omega",
            ))
            .unwrap();
    });
    assert!(
        queued_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "account selection must not become durable before the forwarded turn/start is acknowledged"
    );
    assert!(
        crate::codex_account::view_for_record(
            &crate::load_session_record(&context, &record.id).unwrap()
        )
        .next
        .is_none()
    );

    collision_tx.send(()).unwrap();
    collision_seen_rx.await.unwrap();
    assert!(
        queued_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "a colliding server request id must not release the turn/start fence"
    );
    acknowledge_tx.send(()).unwrap();
    loop {
        let value = receive_json(&mut tui).await;
        if value["id"] == 1 && value.get("method").is_none() {
            assert_eq!(value["result"]["turn"]["id"], "in-flight-turn");
            break;
        }
    }
    let queued = queued_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("account selection should proceed after the provider response")
        .unwrap();
    assert_eq!(
        queued
            .next
            .as_ref()
            .and_then(|next| next.account.as_deref()),
        Some("omega")
    );
    queue.join().unwrap();
    tui.send(Message::Text(
        json!({ "id": 2, "method": "thread/read", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 2);
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn proxy_times_out_unanswered_turn_and_releases_account_gate() {
    let env_lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("unanswered-turn.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("unanswered-turn", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();

    let (forwarded_tx, forwarded_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["method"], "turn/start");
        forwarded_tx.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    forwarded_rx.await.unwrap();
    tokio::time::advance(CONTROL_SUBMISSION_TIMEOUT + Duration::from_secs(1)).await;
    let error = proxy.await.unwrap().unwrap_err();
    assert_eq!(error, "Codex turn/start response timed out");

    {
        let _record_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
        let _gate = acquire_account_mutation_gate(&context, &record.id)
            .expect("proxy timeout must release the account mutation gate");
    }
    let launch_id = &record.runtime.as_ref().unwrap().launch_id;
    crate::codex_account::queue_next_account_with_unbound(&context, &record.id, launch_id, "omega")
        .expect("the account selection may remain queued while the runtime is unhealthy");
    assert!(
        crate::codex_account::begin_next_apply(&context, &record.id, launch_id)
            .unwrap()
            .is_none(),
        "a timed-out turn/start must keep the old runtime from applying queued credentials"
    );
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn proxy_rejects_failed_account_apply_without_upstream_forwarding() {
    let env_lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("failed-turn.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("failed-turn", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let launch_id = &record.runtime.as_ref().unwrap().launch_id;
    crate::codex_account::queue_next_account_with_unbound(&context, &record.id, launch_id, "omega")
        .unwrap();
    let applying = crate::codex_account::begin_next_apply(&context, &record.id, launch_id)
        .unwrap()
        .expect("the queued account should become applying");
    crate::codex_account::finish_next_apply(
        &context,
        &record.id,
        launch_id,
        &applying.account,
        applying.revision,
        applying.intent_id.as_deref().unwrap(),
        Err("credential_rejected"),
    )
    .unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(250), receive_json(&mut socket))
                .await
                .is_err(),
            "a failed account apply must not forward turn/start"
        );
    });
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let rejected = tokio::time::timeout(Duration::from_secs(1), receive_json(&mut tui))
        .await
        .expect("failed account apply should reject promptly");
    assert_eq!(rejected["id"], 1);
    assert_eq!(rejected["error"]["code"], -32001);
    server.await.unwrap();
    proxy.abort();
    let _ = proxy.await;
}

#[tokio::test]
async fn proxy_rejects_runtime_replacement_while_account_apply_is_pending() {
    let env_lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("replaced-turn.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("replaced-turn", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    crate::codex_account::queue_next_account_with_unbound(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        "omega",
    )
    .unwrap();

    let (held_tx, held_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), receive_json(&mut socket))
                .await
                .is_err(),
            "turn/start reached upstream while account apply was pending"
        );
        held_tx.send(()).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(250), receive_json(&mut socket))
                .await
                .is_err(),
            "a replacement runtime must not receive the held turn/start"
        );
    });
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    held_rx.await.unwrap();

    let _record_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let mut replacement = crate::load_session_record(&context, &record.id).unwrap();
    let runtime = replacement.runtime.as_mut().unwrap();
    runtime.generation += 1;
    runtime.launch_id = "replacement-runtime".to_string();
    crate::write_session_record(&context, &replacement).unwrap();
    drop(_record_lock);

    let rejected = tokio::time::timeout(Duration::from_secs(1), receive_json(&mut tui))
        .await
        .expect("runtime replacement should reject promptly");
    assert_eq!(rejected["id"], 1);
    assert_eq!(rejected["error"]["code"], -32001);
    server.await.unwrap();
    proxy.abort();
    let _ = proxy.await;
}

#[tokio::test]
async fn managed_handoff_boundary_preserves_thread_and_arms_one_continuation() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    let broker_calls = tmp.path().join("broker-calls");
    fs::write(
        &broker,
        r#"#!/bin/sh
calls=$1
shift
account=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --account)
      account="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
case "$account" in
  acct1|omega) ;;
  *) exit 2 ;;
esac
printf '%s\n' "$account" >> "$calls"
printf '%s\n' "{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":\"$account\",\"access_token\":\"token-$account\",\"chatgpt_account_id\":\"workspace-$account\",\"plan\":\"team\"}"
"#,
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![
            broker.to_string_lossy().into_owned(),
            broker_calls.to_string_lossy().into_owned(),
        ])
        .unwrap(),
    );
    let socket_path = tmp.path().join("apply-next.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("apply-next", &socket_path);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    let turn_started = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "apply-next-turn-start",
        "runtime_id": "runtime-apply-next",
        "provider": "codex",
        "provider_turn_id": "turn-apply-next",
        "kind": "turn_started",
        "confidence": "authoritative"
    }))
    .unwrap();
    crate::activity::ingest_event(&context, &record.id, turn_started).unwrap();
    bind_thread(&record, "raw-thread-apply-next").unwrap();

    let (complete_turn, complete_turn_rx) = tokio::sync::oneshot::channel();
    let (stale_probe_seen, mut stale_probe_seen_rx) = tokio::sync::oneshot::channel();
    let (complete_racing_turn, complete_racing_turn_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let first_login = receive_json(&mut socket).await;
        assert_eq!(first_login["method"], "account/login/start");
        assert_eq!(first_login["params"]["accessToken"], "token-acct1");
        respond(
            &mut socket,
            &first_login,
            json!({ "type": "chatgptAuthTokens" }),
        )
        .await;
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({ "data": ["raw-thread-apply-next"], "nextCursor": null }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        assert_eq!(resume["method"], "thread/resume");
        respond(&mut socket, &resume, json!({})).await;
        for request_index in 0..2 {
            let usage = receive_json(&mut socket).await;
            assert_eq!(usage["method"], "account/rateLimits/read");
            if request_index == 1 {
                send_json(
                    &mut socket,
                    json!({
                        "method": "turn/started",
                        "params": {
                            "threadId": "raw-thread-apply-next",
                            "turn": {
                                "id": "turn-apply-next",
                                "status": "inProgress"
                            }
                        }
                    }),
                )
                .await
                .unwrap();
            }
            respond(
                &mut socket,
                &usage,
                json!({ "rateLimits": { "primary": { "usedPercent": 1 } } }),
            )
            .await;
        }
        let active_turn = receive_json(&mut socket).await;
        assert_eq!(active_turn["method"], "thread/turns/list");
        respond(
            &mut socket,
            &active_turn,
            json!({
                "data": [{"id": "turn-apply-next", "status": "inProgress", "items": []}],
                "nextCursor": null
            }),
        )
        .await;
        complete_turn_rx.await.unwrap();
        // The long-lived control connection missed turn/completed. The
        // provider's latest-turn response must still let it drain the
        // queued account after the turn becomes idle.
        let latest_turn = receive_json(&mut socket).await;
        assert_eq!(latest_turn["method"], "thread/turns/list");
        assert_eq!(latest_turn["params"]["threadId"], "raw-thread-apply-next");
        send_json(
            &mut socket,
            json!({
                "method": "turn/started",
                "params": {
                    "threadId": "raw-thread-apply-next",
                    "turn": {
                        "id": "racing-turn",
                        "status": "inProgress"
                    }
                }
            }),
        )
        .await
        .unwrap();
        respond(
            &mut socket,
            &latest_turn,
            json!({
                "data": [{
                    "id": "turn-apply-next",
                    "status": "completed",
                    "items": []
                }],
                "nextCursor": null
            }),
        )
        .await;
        stale_probe_seen.send(()).unwrap();
        complete_racing_turn_rx.await.unwrap();
        // The racing turn also finishes without a control notification.
        // Only its matching terminal probe can release the queued switch.
        let latest_turn = receive_json(&mut socket).await;
        assert_eq!(latest_turn["method"], "thread/turns/list");
        respond(
            &mut socket,
            &latest_turn,
            json!({
                "data": [{
                    "id": "racing-turn",
                    "status": "completed",
                    "items": []
                }],
                "nextCursor": null
            }),
        )
        .await;
        let next_login = receive_json(&mut socket).await;
        assert_eq!(next_login["method"], "account/login/start");
        assert_eq!(next_login["params"]["accessToken"], "token-omega");
        assert_eq!(next_login["params"]["chatgptAccountId"], "workspace-omega");
        respond(
            &mut socket,
            &next_login,
            json!({ "type": "chatgptAuthTokens" }),
        )
        .await;
    });

    let (handle, commands, ready) = starting_control_channel();
    let control = tokio::spawn(run_control(
        context.clone(),
        record.clone(),
        commands,
        ready,
    ));
    assert!(handle.usage().await.unwrap().authoritative);
    crate::codex_account::queue_next_account(&context, &record.id, "runtime-apply-next", "omega")
        .unwrap();
    handle.apply_next().await.unwrap();
    let active_view = crate::codex_account::view_for_record(
        &crate::load_session_record(&context, &record.id).unwrap(),
    );
    assert_eq!(
        active_view.next.as_ref().map(|next| next.state),
        Some("queued")
    );
    complete_turn.send(()).unwrap();
    assert_eq!(
        crate::activity::activity_status(&context, &record.id)
            .unwrap()
            .turn_state
            .phase,
        crate::activity::TurnPhase::Working,
        "the regression requires live idle while durable activity is stale-working"
    );

    let readiness = ensure_turn_start_account_ready(&context, &record);
    tokio::pin!(readiness);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut readiness)
            .await
            .is_err(),
        "turn/start must remain held while the long-lived control owner has not applied the queued account"
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            handle.apply_next().await.unwrap();
            if tokio::time::timeout(Duration::from_millis(10), &mut stale_probe_seen_rx)
                .await
                .is_ok()
            {
                break;
            }
        }
    })
    .await
    .expect("live idle probe must observe the completed turn");
    let raced_view = crate::codex_account::view_for_record(
        &crate::load_session_record(&context, &record.id).unwrap(),
    );
    assert_eq!(
        raced_view.next.as_ref().map(|next| next.state),
        Some("queued"),
        "an interleaved live turn/start must override a stale idle list response"
    );
    complete_racing_turn.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            handle.apply_next().await.unwrap();
            let view = crate::codex_account::view_for_record(
                &crate::load_session_record(&context, &record.id).unwrap(),
            );
            if view.next.is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("live idle must drain the queued account despite stale activity");
    let applied_view = crate::codex_account::view_for_record(
        &crate::load_session_record(&context, &record.id).unwrap(),
    );
    assert_eq!(applied_view.selected_account.as_deref(), Some("omega"));
    assert!(applied_view.next.is_none());
    assert!(
        readiness.await,
        "turn/start must become ready only after the control owner durably applies the account"
    );
    server.await.unwrap();
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.selected_account.as_deref(), Some("omega"));
    assert!(view.next.is_none());
    assert_eq!(
        persisted
            .provider_resume
            .as_ref()
            .map(|resume| resume.session_id.as_str()),
        Some("raw-thread-apply-next"),
        "managed account binding must retain the exact provider thread"
    );
    let turn_completed = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "apply-next-turn-completed",
        "runtime_id": "runtime-apply-next",
        "provider": "codex",
        "provider_session_id": "raw-thread-apply-next",
        "provider_turn_id": "turn-apply-next",
        "kind": "turn_completed",
        "confidence": "authoritative"
    }))
    .unwrap();
    crate::activity::ingest_event(&context, &record.id, turn_completed).unwrap();
    assert!(
        crate::auto_resume::arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-apply-next".to_string(),
            2,
            "2030-01-01T00:00:01Z",
        )
        .unwrap(),
        "the handoff boundary arms one continuation"
    );
    assert!(
        !crate::auto_resume::arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-apply-next".to_string(),
            2,
            "2030-01-01T00:00:02Z",
        )
        .unwrap(),
        "an exact replay cannot arm a duplicate continuation"
    );
    assert_eq!(
        crate::auto_resume::view_for_record(&context, &persisted).state,
        "armed"
    );
    assert_eq!(
        fs::read_to_string(&broker_calls).unwrap(),
        "acct1\nomega\n",
        "the real fake broker resolves exactly the initial and requested account once"
    );
    drop(handle);
    control.abort();
    let _ = control.await;
}

#[tokio::test]
async fn submit_prompt_rejects_response_without_acknowledged_turn_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket_path = tmp.path().join("missing-turn-id.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("missing-turn-id", &socket_path);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-missing-turn-id").unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({ "data": ["thread-missing-turn-id"], "nextCursor": null }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        assert_eq!(resume["method"], "thread/resume");
        respond(&mut socket, &resume, json!({})).await;
        let usage = receive_json(&mut socket).await;
        assert_eq!(usage["method"], "account/rateLimits/read");
        respond(
            &mut socket,
            &usage,
            json!({ "rateLimits": { "primary": { "usedPercent": 1 } } }),
        )
        .await;
        let prompt = receive_json(&mut socket).await;
        assert_eq!(prompt["method"], "turn/start");
        respond(
            &mut socket,
            &prompt,
            json!({ "turn": { "status": "inProgress" } }),
        )
        .await;
    });
    let (handle, commands, ready) = starting_control_channel();
    let control = tokio::spawn(run_control(context, record, commands, ready));

    let error = handle
        .submit_prompt("must require an acknowledged turn id")
        .await
        .unwrap_err();
    assert_eq!(
        error,
        "Codex turn/start response omitted the acknowledged turn id"
    );
    server.await.unwrap();
    drop(handle);
    control.abort();
    let _ = control.await;
}

#[tokio::test]
async fn failed_reconnect_waits_for_explicit_rebind_before_thread_discovery() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(
        &broker,
        "#!/bin/sh\nprintf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":\"acct1\",\"access_token\":\"token-acct1\",\"chatgpt_account_id\":\"workspace-acct1\",\"plan\":\"team\"}'\n",
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![broker.to_string_lossy().into_owned()]).unwrap(),
    );
    let socket_path = tmp.path().join("failed-reconnect.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("failed-reconnect", &socket_path);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-failed-reconnect",
        "acct1",
        1,
        Err("apply_failed"),
    )
    .unwrap();
    record = crate::load_session_record(&context, &record.id).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::activity::ingest_codex_app_server_failure_with_kind(
        &context,
        &record.id,
        "runtime-failed-reconnect",
        "thread-failed",
        "turn-failed",
        StructuredFailureKind::UsageExhausted,
    )
    .unwrap();
    bind_thread(&record, "raw-thread-failed").unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let login = receive_json(&mut socket).await;
        assert_eq!(login["method"], "account/login/start");
        respond(&mut socket, &login, json!({ "type": "chatgptAuthTokens" })).await;
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({ "data": ["raw-thread-failed"], "nextCursor": null }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        assert_eq!(resume["method"], "thread/resume");
        respond(&mut socket, &resume, json!({})).await;
        let usage = receive_json(&mut socket).await;
        assert_eq!(usage["method"], "account/rateLimits/read");
        respond(
            &mut socket,
            &usage,
            json!({ "rateLimits": { "primary": { "usedPercent": 1 } } }),
        )
        .await;
    });
    let (handle, commands, ready) = starting_control_channel();
    let control = tokio::spawn(run_control(
        context.clone(),
        record.clone(),
        commands,
        ready,
    ));

    assert!(handle.usage().await.is_err());
    let revision = crate::codex_account::begin_switch_binding(
        &context,
        &record.id,
        "runtime-failed-reconnect",
        "acct1",
    )
    .unwrap();
    assert_eq!(revision, 2);
    let view = handle.bind_account("acct1", revision).await.unwrap();
    assert_eq!(view.state, "bound");
    assert_eq!(
        view.applied_runtime_id.as_deref(),
        Some("runtime-failed-reconnect")
    );
    server.await.unwrap();
    control.abort();
    let _ = control.await;
}

#[tokio::test]
async fn tui_proxy_accepts_the_observed_codex_0_145_frame_size() {
    const OBSERVED_CODEX_0_145_FRAME_BYTES: usize = 4_260_254;

    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("large-frame.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("large-frame", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let payload =
            serde_json::to_string(&"x".repeat(OBSERVED_CODEX_0_145_FRAME_BYTES - 2)).unwrap();
        assert_eq!(payload.len(), OBSERVED_CODEX_0_145_FRAME_BYTES);
        socket.send(Message::Text(payload.into())).await.unwrap();
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let tui_config = WebSocketConfig::default()
        .max_message_size(Some(MAX_PROXY_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_PROXY_MESSAGE_BYTES));
    let (mut tui, _) = tokio_tungstenite::client_async_with_config(
        "ws://localhost",
        proxy_stream,
        Some(tui_config),
    )
    .await
    .unwrap();
    let message = tokio::time::timeout(Duration::from_secs(10), tui.next())
        .await
        .expect("the Codex 0.145.0 frame must arrive before the deadline")
        .expect("the proxy must keep the TUI connection open")
        .expect("the proxy must forward the upstream frame");
    assert_eq!(
        message.into_text().unwrap().len(),
        OBSERVED_CODEX_0_145_FRAME_BYTES
    );

    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test]
async fn tui_proxy_projects_exact_failure_from_the_tui_connection_without_content() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("upstream.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let id = "proxy-control";
    let session_dir = crate::session_dir(&context, id);
    fs::create_dir_all(&session_dir).unwrap();
    crate::write_private_file(
        &session_dir.join(crate::STARTUP_DIAGNOSTIC_FILE),
        b"local-only startup detail\n",
    )
    .unwrap();
    let record = record_with_runtime(id, &upstream);
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, id, true, "2030-01-01T00:00:00Z").unwrap();
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: id.to_string(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let start = receive_json(&mut socket).await;
        assert_eq!(start["method"], "thread/start");
        respond(
            &mut socket,
            &start,
            json!({ "thread": { "id": "raw-proxy-thread" } }),
        )
        .await;
        for value in [
            json!({
                "method": "error",
                "params": {
                    "threadId": "raw-proxy-thread",
                    "turnId": "raw-proxy-turn",
                    "willRetry": false,
                    "error": {
                        "message": "localized secret proxy error",
                        "codexErrorInfo": "usageLimitExceeded"
                    }
                }
            }),
            json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "raw-proxy-thread",
                    "turn": { "id": "raw-proxy-turn", "status": "failed" }
                }
            }),
        ] {
            socket
                .send(Message::Text(value.to_string().into()))
                .await
                .unwrap();
        }
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({
            "id": 7,
            "method": "thread/start",
            "params": { "cwd": "/repo", "developerInstructions": "secret prompt" }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    for _ in 0..3 {
        tui.next().await.unwrap().unwrap();
    }
    server.await.unwrap();
    proxy.await.unwrap().unwrap();

    assert_eq!(
        fs::read_to_string(session_dir.join(crate::STARTUP_STAGE_FILE)).unwrap(),
        "initial_connection\n"
    );
    assert!(!session_dir.join(crate::STARTUP_DIAGNOSTIC_FILE).exists());
    let activity = format!(
        "{}\n{}",
        fs::read_to_string(session_dir.join("activity.json")).unwrap(),
        fs::read_to_string(session_dir.join("activity.journal.jsonl")).unwrap()
    );
    assert!(activity.contains("provider_hook"));
    assert!(activity.contains("usage_exhausted"));
    for secret in [
        "raw-proxy-thread",
        "raw-proxy-turn",
        "localized secret proxy error",
        "secret prompt",
    ] {
        assert!(!activity.contains(secret));
    }
}

#[tokio::test]
async fn tui_turn_start_cancels_a_scheduled_resume_before_forwarding() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("manual.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("manual-input", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    crate::activity::ingest_codex_app_server_failure_with_kind(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        "thread-a",
        "failed-turn",
        StructuredFailureKind::UsageExhausted,
    )
    .unwrap();
    assert_eq!(
        crate::auto_resume::tick_for_runtime(
            &context,
            &record.id,
            &record.runtime.as_ref().unwrap().launch_id,
            1_893_456_000,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: true,
                exhausted_reset_epochs: vec![1_893_456_600],
                soonest_reset_epoch: None,
            },
            |_| panic!("blocked usage must not submit"),
        )
        .unwrap(),
        crate::auto_resume::TickOutcome::Scheduled
    );
    bind_thread(&record, "thread-a").unwrap();

    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server_context = context.clone();
    let server_record = record.clone();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let turn = receive_json(&mut socket).await;
        assert_eq!(turn["method"], "turn/start");
        let view = crate::auto_resume::view_for_record(&server_context, &server_record);
        assert_eq!(view.state, "cancelled");
        assert_eq!(view.failure_reason.as_deref(), Some("manual_input"));
        respond(
            &mut socket,
            &turn,
            json!({ "turn": { "id": "manual-turn", "status": "inProgress" } }),
        )
        .await;
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({
            "id": 9,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 9);
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test]
async fn managed_account_proxy_authorizes_tui_turn_without_broker_environment() {
    let lock = GlobalStateLock::new();
    let broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("managed-manual.sock");
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut proxy_record = record_with_runtime("managed-manual", &upstream);
    crate::codex_account::set_initial_binding(&mut proxy_record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &proxy_record.id)).unwrap();
    crate::write_session_record(&context, &proxy_record).unwrap();
    crate::activity::activate_runtime(&context, &proxy_record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &proxy_record.id,
        &proxy_record.runtime.as_ref().unwrap().launch_id,
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();
    bind_thread(&proxy_record, "managed-thread").unwrap();
    let _capability = begin_proxy_capability(&context, &proxy_record).unwrap();
    let record_lock = crate::acquire_session_record_lock(&context, &proxy_record.id).unwrap();
    let current = crate::load_session_record(&context, &proxy_record.id).unwrap();
    let marker = begin_manual_input_section(&context, &current)
        .unwrap()
        .expect("managed Codex input must publish sender authority");
    drop(broker);
    let _without_broker = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let mut bootstrap = FreshBootstrap::Closed;
    let authorization = cancel_before_tui_mutation(
        &context,
        &proxy_record,
        &mut bootstrap,
        &json!({
            "id": 10,
            "method": "turn/start",
            "params": { "threadId": "managed-thread", "input": [] }
        }),
    )
    .await
    .is_some();
    marker.finish(|| drop(record_lock));

    assert!(
        authorization,
        "valid managed-account input must be authorized"
    );
}

#[tokio::test]
async fn automatic_failover_applies_after_structured_failure_without_live_idle_probe() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    let broker_calls = tmp.path().join("broker-calls");
    fs::write(
        &broker,
        r#"#!/bin/sh
calls=$1
shift
account=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --account)
      account="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
case "$account" in
  acct1|omega) ;;
  *) exit 2 ;;
esac
printf '%s\n' "$account" >> "$calls"
printf '%s\n' "{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":\"$account\",\"access_token\":\"token-$account\",\"chatgpt_account_id\":\"workspace-$account\",\"plan\":\"team\"}"
"#,
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![
            broker.to_string_lossy().into_owned(),
            broker_calls.to_string_lossy().into_owned(),
        ])
        .unwrap(),
    );
    let socket_path = tmp.path().join("automatic-failover.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("automatic-failover", &socket_path);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        "runtime-automatic-failover",
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();
    record = crate::load_session_record(&context, &record.id).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::codex_account::authorize_input_locked(&context, &mut record).unwrap();
    crate::auto_resume::set_enabled_with_policy(
        &context,
        &record.id,
        true,
        crate::auto_resume::NEXT_ACCOUNT_THEN_RESUME_POLICY,
        "2030-01-01T00:00:00Z",
    )
    .unwrap();
    crate::activity::ingest_codex_app_server_failure_with_kind(
        &context,
        &record.id,
        "runtime-automatic-failover",
        "thread-automatic-failover",
        "turn-automatic-failover",
        StructuredFailureKind::UsageExhausted,
    )
    .unwrap();
    crate::codex_account::queue_auto_failover_locked(&context, &mut record, "omega").unwrap();
    record = crate::codex_account::prepare_control_reconnect(
        &context,
        &record.id,
        "runtime-automatic-failover",
    )
    .unwrap();
    bind_thread(&record, "thread-automatic-failover").unwrap();
    assert!(
        crate::codex_account::pending_auto_failover_apply(&record)
            .unwrap()
            .is_some()
    );
    let terminal = crate::activity::state_for_view(&context, &record).unwrap();
    assert_eq!(terminal.phase, crate::activity::TurnPhase::Waiting);
    assert!(terminal.current_turn.is_none());
    assert!(crate::auto_resume::has_authoritative_usage_exhaustion_idle(
        &context, &record
    ));
    assert!(
        automatic_failover_has_authoritative_idle(&context, &record)
            .await
            .is_some()
    );

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let first_login = receive_json(&mut socket).await;
        assert_eq!(first_login["method"], "account/login/start");
        respond(
            &mut socket,
            &first_login,
            json!({ "type": "chatgptAuthTokens" }),
        )
        .await;
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({ "data": ["thread-automatic-failover"], "nextCursor": null }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        assert_eq!(resume["method"], "thread/resume");
        respond(&mut socket, &resume, json!({})).await;

        let failover_login = receive_json(&mut socket).await;
        assert_eq!(
            failover_login["method"], "account/login/start",
            "an authoritative structured terminal failure must not depend on a live idle probe"
        );
        assert_eq!(failover_login["params"]["accessToken"], "token-omega");
        respond(
            &mut socket,
            &failover_login,
            json!({ "type": "chatgptAuthTokens" }),
        )
        .await;
    });

    let (handle, commands, ready) = starting_control_channel();
    let control = tokio::spawn(run_control(
        context.clone(),
        record.clone(),
        commands,
        ready,
    ));
    handle.apply_next().await.unwrap();
    server.await.unwrap();
    control.abort();
    let _ = control.await;

    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let view = crate::codex_account::view_for_record(&persisted);
    assert_eq!(view.selected_account.as_deref(), Some("omega"));
    assert!(view.next.is_none());
}

#[tokio::test]
async fn managed_account_proxy_authorizes_raw_tui_turn_without_broker_environment() {
    let env_lock = GlobalStateLock::new();
    let broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("managed-raw.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("managed-raw", &upstream);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();
    bind_thread(&record, "managed-thread").unwrap();
    drop(broker);
    let _without_broker = EnvGuard::remove(&env_lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");

    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["method"], "turn/start");
        respond(
            &mut socket,
            &request,
            json!({ "turn": { "id": "post-switch-turn", "status": "inProgress" } }),
        )
        .await;
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({
            "id": 11,
            "method": "turn/start",
            "params": { "threadId": "managed-thread", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();

    let response = receive_json(&mut tui).await;
    assert_eq!(response["id"], 11);
    assert_eq!(response["result"]["turn"]["id"], "post-switch-turn");
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test]
async fn managed_account_proxy_thread_start_does_not_persist_turn_fence() {
    let env_lock = GlobalStateLock::new();
    let broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("managed-thread-only.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("managed-thread-only", &upstream);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();
    let turn_started = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "thread-only-prior-turn-start",
        "runtime_id": record.runtime.as_ref().unwrap().launch_id,
        "provider": "codex",
        "provider_turn_id": "thread-only-prior-turn",
        "kind": "turn_started",
        "confidence": "authoritative"
    }))
    .unwrap();
    crate::activity::ingest_event(&context, &record.id, turn_started).unwrap();
    let turn_completed = serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": "thread-only-prior-turn-complete",
        "runtime_id": record.runtime.as_ref().unwrap().launch_id,
        "provider": "codex",
        "provider_turn_id": "thread-only-prior-turn",
        "kind": "turn_completed",
        "confidence": "authoritative"
    }))
    .unwrap();
    crate::activity::ingest_event(&context, &record.id, turn_completed).unwrap();
    drop(broker);
    let _without_broker = EnvGuard::remove(&env_lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");

    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["method"], "thread/start");
        respond(
            &mut socket,
            &request,
            json!({ "thread": { "id": "idle-thread" } }),
        )
        .await;
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({ "id": 12, "method": "thread/start", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();

    let response = receive_json(&mut tui).await;
    assert_eq!(response["id"], 12);
    assert_eq!(response["result"]["thread"]["id"], "idle-thread");
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
    drop(_without_broker);
    let _broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );

    crate::codex_account::begin_switch_binding(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        "acct2",
    )
    .unwrap();
}

#[tokio::test]
async fn busy_manual_cancellation_rejects_only_the_turn_and_keeps_proxy_alive() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("busy.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("busy-input", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["id"], 2);
        respond(&mut socket, &request, json!({ "ok": true })).await;
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let rejected = receive_json(&mut tui).await;
    assert_eq!(rejected["id"], 1);
    assert_eq!(rejected["error"]["code"], -32001);
    assert_eq!(
        rejected["error"]["data"]["reason"],
        "manual_cancellation_busy"
    );
    drop(lock);
    tui.send(Message::Text(
        json!({ "id": 2, "method": "thread/read", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 2);
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test]
async fn transient_busy_manual_cancellation_waits_and_forwards_turn() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("transient-busy.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("transient-busy-input", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let interrupt = receive_json(&mut socket).await;
        assert_eq!(interrupt["method"], "turn/interrupt");
        respond(&mut socket, &interrupt, json!({ "ok": true })).await;
        let turn = receive_json(&mut socket).await;
        assert_eq!(turn["method"], "turn/start");
        respond(
            &mut socket,
            &turn,
            json!({ "turn": { "id": "next-turn", "status": "inProgress" } }),
        )
        .await;
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "turn/interrupt",
            "params": { "threadId": "thread-a", "turnId": "current-turn" }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 1);

    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let release_lock = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(lock);
    });
    tui.send(Message::Text(
        json!({
            "id": 2,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let response = receive_json(&mut tui).await;
    assert_eq!(response["id"], 2);
    assert_eq!(response["result"]["turn"]["id"], "next-turn");

    release_lock.join().unwrap();
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test]
async fn manual_input_section_bypasses_reentrant_lock_until_sender_cleanup() {
    let env_lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("owned-input.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("owned-input", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["method"], "turn/start");
        respond(
            &mut socket,
            &request,
            json!({ "turn": { "id": "owned-turn", "status": "inProgress" } }),
        )
        .await;
        let request = receive_json(&mut socket).await;
        assert_eq!(request["id"], 3);
        assert_eq!(request["method"], "thread/read");
        respond(&mut socket, &request, json!({ "ok": true })).await;
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    let record_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    for _ in 0..100 {
        if live_proxy_capability(&context, &record) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let marker = begin_manual_input_section(&context, &record)
        .unwrap()
        .unwrap();
    let queue_context = context.clone();
    let queue_id = record.id.clone();
    let queue_launch_id = record.runtime.as_ref().unwrap().launch_id.clone();
    let (queue_started_tx, queue_started_rx) = std::sync::mpsc::channel();
    let (queued_tx, queued_rx) = std::sync::mpsc::channel();
    let queue = std::thread::spawn(move || {
        queue_started_tx.send(()).unwrap();
        let result = crate::codex_account::queue_next_account_with_unbound(
            &queue_context,
            &queue_id,
            &queue_launch_id,
            "omega",
        );
        queued_tx.send(result).unwrap();
    });
    queue_started_rx.recv().unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(1), receive_json(&mut tui))
        .await
        .expect("the sender-owned record lock must not deadlock proxy authorization");
    assert_eq!(response["result"]["turn"]["id"], "owned-turn");
    assert!(
        queued_rx.try_recv().is_err(),
        "a racing account queue must remain fenced until the forwarded request lands"
    );
    assert!(
        manual_input_section_path(&context, &record).exists(),
        "manual input section remains live until the sender releases its lock"
    );
    marker.finish(|| drop(record_lock));
    let queued = queued_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the account queue should proceed after sender cleanup")
        .unwrap();
    let next = queued.next.expect("the account intent should be queued");
    crate::codex_account::cancel_next_account(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        next.account.as_deref(),
        Some(next.revision),
    )
    .unwrap();
    queue.join().unwrap();
    let unrelated_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    tui.send(Message::Text(
        json!({
            "id": 2,
            "method": "turn/start",
            "params": { "threadId": "thread-a", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let rejected = receive_json(&mut tui).await;
    assert_eq!(rejected["id"], 2);
    assert_eq!(rejected["error"]["code"], -32001);
    drop(unrelated_lock);
    tui.send(Message::Text(
        json!({ "id": 3, "method": "thread/read", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 3);
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[test]
fn local_input_ownership_rejects_expiry_and_runtime_replacement() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("owned-negative", &tmp.path().join("negative.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let turn = json!({
        "id": 1,
        "method": "turn/start",
        "params": { "threadId": "thread-a", "input": [] }
    });

    let expired = write_manual_input_marker(
        &context,
        &record,
        &record.runtime.as_ref().unwrap().launch_id,
        epoch_millis().unwrap().saturating_sub(1),
    );
    assert!(acquire_manual_input_gate(&context, &record, &turn).is_none());
    drop(expired);

    let replacement = write_manual_input_marker(
        &context,
        &record,
        "replacement-runtime",
        epoch_millis()
            .unwrap()
            .saturating_add(u64::try_from(MANUAL_INPUT_SECTION_TTL.as_millis()).unwrap()),
    );
    assert!(acquire_manual_input_gate(&context, &record, &turn).is_none());
    drop(replacement);

    let valid = write_manual_input_marker(
        &context,
        &record,
        &record.runtime.as_ref().unwrap().launch_id,
        epoch_millis()
            .unwrap()
            .saturating_add(u64::try_from(MANUAL_INPUT_SECTION_TTL.as_millis()).unwrap()),
    );
    let wrong_thread = json!({
        "id": 2,
        "method": "turn/start",
        "params": { "threadId": "thread-b", "input": [] }
    });
    assert!(acquire_manual_input_gate(&context, &record, &wrong_thread).is_none());
    assert!(manual_input_section_path(&context, &record).exists());
    let malformed = json!({
        "id": 3,
        "method": "turn/start",
        "params": { "threadId": "thread-a" }
    });
    assert!(acquire_manual_input_gate(&context, &record, &malformed).is_none());
    assert!(manual_input_section_path(&context, &record).exists());
    drop(acquire_manual_input_gate(&context, &record, &turn).unwrap());
    drop(valid);
}

#[test]
fn manual_input_submission_detection_excludes_ordinary_text_and_keys() {
    for key in [
        crate::cli::SpecialKey::Escape,
        crate::cli::SpecialKey::Backspace,
        crate::cli::SpecialKey::CtrlC,
        crate::cli::SpecialKey::Up,
        crate::cli::SpecialKey::Down,
        crate::cli::SpecialKey::Left,
        crate::cli::SpecialKey::ShiftLeft,
        crate::cli::SpecialKey::Right,
        crate::cli::SpecialKey::Tab,
    ] {
        assert!(
            !input_contains_submission(None, &[key]),
            "non-submitting key must not open a manual section: {key:?}"
        );
    }
    assert!(input_contains_submission(
        None,
        &[crate::cli::SpecialKey::Enter]
    ));
    for text in ["\r", "\n", "\r\n"] {
        assert!(
            input_contains_submission(Some(text), &[]),
            "terminal submit must open a manual section: {text:?}"
        );
    }
    for text in [
        "",
        "hi",
        "line one\nline two",
        "\u{1b}[200~pasted\ntext\u{1b}[201~",
    ] {
        assert!(
            !input_contains_submission(Some(text), &[]),
            "ordinary text and multiline paste must not open a manual section: {text:?}"
        );
    }
}

#[test]
fn manual_input_section_requires_live_proxy_capability_and_cleans_up() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("owned-capability", &tmp.path().join("capability.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();

    let error = begin_manual_input_section(&context, &record).unwrap_err();
    assert_eq!(error.code(), "codex-input-section-unavailable");

    let capability = begin_proxy_capability(&context, &record).unwrap();
    let marker = begin_manual_input_section(&context, &record)
        .unwrap()
        .unwrap();
    assert!(manual_input_section_path(&context, &record).exists());
    drop(marker);
    assert!(!manual_input_section_path(&context, &record).exists());
    drop(capability);
    assert!(proxy_capability_path(&context, &record).exists());
    assert!(!live_proxy_capability(&context, &record));
}

#[test]
fn manual_input_gate_finishes_before_lifecycle_unlock_and_marker_cleanup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("owned-gate", &tmp.path().join("gate.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let _capability = begin_proxy_capability(&context, &record).unwrap();
    let lifecycle_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let marker = begin_manual_input_section(&context, &record)
        .unwrap()
        .unwrap();
    let turn = json!({
        "id": 1,
        "method": "turn/start",
        "params": { "threadId": "thread-a", "input": [] }
    });
    let gate = acquire_manual_input_gate(&context, &record, &turn).unwrap();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let teardown = std::thread::spawn(move || {
        marker.finish(|| drop(lifecycle_lock));
        finished_tx.send(()).unwrap();
    });

    assert!(finished_rx.recv_timeout(Duration::from_millis(10)).is_err());
    assert!(
        crate::try_acquire_session_record_lock(&context, &record.id)
            .unwrap()
            .is_none(),
        "lifecycle lock stays held while the proxy forwards through the gate"
    );
    drop(gate);
    finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    teardown.join().unwrap();
    assert!(!manual_input_section_path(&context, &record).exists());
    assert!(
        crate::try_acquire_session_record_lock(&context, &record.id)
            .unwrap()
            .is_some()
    );
}

#[test]
fn manual_input_gate_timeout_releases_lifecycle_and_invalidates_marker() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("owned-timeout", &tmp.path().join("timeout.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let _capability = begin_proxy_capability(&context, &record).unwrap();
    let lifecycle_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let mut marker = begin_manual_input_section(&context, &record)
        .unwrap()
        .unwrap();
    let held_gate =
        open_manual_input_gate_file(&manual_input_gate_path(&context, &record)).unwrap();
    assert!(lock_file_timed(&held_gate, Duration::from_millis(10)));
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let teardown = std::thread::spawn(move || {
        marker.finish_with_timeout(|| drop(lifecycle_lock), Duration::from_millis(20));
        finished_tx.send(()).unwrap();
    });

    finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    teardown.join().unwrap();
    assert!(!manual_input_section_path(&context, &record).exists());
    assert!(
        crate::try_acquire_session_record_lock(&context, &record.id)
            .unwrap()
            .is_some()
    );
    unlock_bootstrap_file(&held_gate);
}

#[test]
fn delayed_proxy_claim_acknowledges_before_sender_retires_section() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("owned-delayed", &tmp.path().join("delayed.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let _capability = begin_proxy_capability(&context, &record).unwrap();
    let lifecycle_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let marker = begin_manual_input_section(&context, &record)
        .unwrap()
        .unwrap();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let teardown = std::thread::spawn(move || {
        marker.finish(|| drop(lifecycle_lock));
        finished_tx.send(()).unwrap();
    });
    std::thread::sleep(Duration::from_millis(25));
    let turn = json!({
        "id": 1,
        "method": "turn/start",
        "params": { "threadId": "thread-a", "input": [] }
    });
    let gate = acquire_manual_input_gate(&context, &record, &turn).unwrap();
    assert!(finished_rx.recv_timeout(Duration::from_millis(10)).is_err());
    assert!(
        crate::try_acquire_session_record_lock(&context, &record.id)
            .unwrap()
            .is_none()
    );
    drop(gate);
    finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    teardown.join().unwrap();
}

#[test]
fn stale_manual_marker_without_owner_lease_cannot_authorize_busy() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("owned-stale", &tmp.path().join("stale.sock"));
    let session_dir = crate::session_dir(&context, &record.id);
    fs::create_dir_all(&session_dir).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let _capability = begin_proxy_capability(&context, &record).unwrap();
    let lifecycle_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let marker = begin_manual_input_section(&context, &record)
        .unwrap()
        .unwrap();
    // `marker.finish` runs between making the directory read-only and
    // restoring it, so a plain restore statement would be skipped if it
    // panicked — and `remove_dir_all` cannot empty a read-only directory, so
    // the fixture would leak. Restore from `Drop` instead.
    let restored_session_dir = nils_test_support::tempdir::RestoredMode::read_only(&session_dir);
    marker.finish(|| drop(lifecycle_lock));
    drop(restored_session_dir);
    assert!(manual_input_section_path(&context, &record).exists());
    let unrelated_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let turn = json!({
        "id": 1,
        "method": "turn/start",
        "params": { "threadId": "thread-a", "input": [] }
    });
    assert!(acquire_manual_input_gate(&context, &record, &turn).is_none());
    drop(unrelated_lock);
}

#[tokio::test]
async fn managed_account_fresh_proxy_authorizes_first_turn_without_broker_environment() {
    let env_lock = GlobalStateLock::new();
    let broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("fresh-start.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("fresh-start", &upstream);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::codex_account::finish_binding(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        "acct1",
        1,
        Ok(()),
    )
    .unwrap();
    drop(broker);
    let _without_broker = EnvGuard::remove(&env_lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    write_create_bootstrap_marker(&record);
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        assert_eq!(request["method"], "thread/start");
        respond(
            &mut socket,
            &request,
            json!({ "thread": { "id": "fresh-thread" } }),
        )
        .await;
        let request = receive_json(&mut socket).await;
        assert_eq!(request["id"], 2);
        assert_eq!(request["method"], "turn/start");
        respond(
            &mut socket,
            &request,
            json!({ "turn": { "id": "first-turn", "status": "inProgress" } }),
        )
        .await;
        let request = receive_json(&mut socket).await;
        assert_eq!(request["id"], 4);
        assert_eq!(request["method"], "thread/read");
        respond(
            &mut socket,
            &request,
            json!({ "thread": { "id": "fresh-thread" } }),
        )
        .await;
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    tui.send(Message::Text(
        json!({
            "id": 1,
            "method": "thread/start",
            "params": { "cwd": "/repo" }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let response = receive_json(&mut tui).await;
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["thread"]["id"], "fresh-thread");
    tui.send(Message::Text(
        json!({
            "id": 2,
            "method": "turn/start",
            "params": { "threadId": "fresh-thread", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let response = receive_json(&mut tui).await;
    assert_eq!(response["id"], 2);
    assert_eq!(response["result"]["turn"]["id"], "first-turn");
    tui.send(Message::Text(
        json!({
            "id": 3,
            "method": "turn/start",
            "params": { "threadId": "fresh-thread", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let rejected = receive_json(&mut tui).await;
    assert_eq!(rejected["id"], 3);
    assert_eq!(rejected["error"]["code"], -32001);
    tui.send(Message::Text(
        json!({ "id": 4, "method": "thread/read", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 4);
    drop(lock);
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[test]
fn fresh_bootstrap_allows_pre_enabled_auto_resume() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("fresh-enabled", &tmp.path().join("enabled.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    write_create_bootstrap_marker(&record);
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();

    let mut bootstrap = FreshBootstrap::for_runtime(&context, &record);
    assert!(bootstrap.bypasses_create_lock(
        &context,
        &record,
        &json!({ "id": 1, "method": "thread/start", "params": {} }),
    ));
    bootstrap.observe_server(&json!({ "id": 1, "result": { "thread": { "id": "fresh-thread" } } }));
    assert!(bootstrap.bypasses_create_lock(
        &context,
        &record,
        &json!({
            "id": 2,
            "method": "turn/start",
            "params": { "threadId": "fresh-thread", "input": [] }
        }),
    ));
}

#[tokio::test]
async fn fresh_profile_without_auto_resume_authorizes_bootstrap_under_create_lock() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("profile-bootstrap", &tmp.path().join("app.sock"));
    let runtime = record.runtime.as_mut().unwrap();
    runtime.extra.insert(
        crate::AGENT_PROFILE_RUNTIME_KEY.to_string(),
        json!("custom-codex"),
    );
    runtime.extra.insert(
        crate::AGENT_PROFILE_AUTO_RESUME_SUPPORTED_RUNTIME_KEY.to_string(),
        json!(false),
    );
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let create = begin_create_bootstrap(&record).unwrap().unwrap();
    let lifecycle = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let mut bootstrap = FreshBootstrap::for_runtime(&context, &record);
    let authorization = cancel_before_tui_mutation_detailed(
        &context,
        &record,
        &mut bootstrap,
        &json!({ "id": 1, "method": "thread/start", "params": {} }),
    )
    .await;
    assert!(
        authorization.is_ok(),
        "disabled automatic continuation must not reject a fresh managed thread: {:?}",
        authorization.err()
    );
    drop(authorization);
    bootstrap.observe_server(&json!({ "id": 1, "result": { "thread": { "id": "fresh-thread" } } }));
    let authorization = cancel_before_tui_mutation_detailed(
        &context,
        &record,
        &mut bootstrap,
        &json!({
            "id": 2, "method": "turn/start",
            "params": { "threadId": "fresh-thread", "input": [] }
        }),
    )
    .await;
    assert!(authorization.is_ok());
    drop(authorization);
    assert_eq!(bootstrap, FreshBootstrap::Closed);
    let continuation = crate::auto_resume::view_for_record(&context, &record);
    assert!(!continuation.supported);
    assert!(!continuation.enabled);
    assert_eq!(continuation.state, "disabled");
    create.finish(|| drop(lifecycle));
}

#[test]
fn fresh_bootstrap_accepts_only_healthy_idle_auto_resume_states() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("fresh-matrix", &tmp.path().join("matrix.sock"));
    let session_dir = crate::session_dir(&context, &record.id);
    fs::create_dir_all(&session_dir).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    write_create_bootstrap_marker(&record);

    for (name, enabled, state, scheduled_at, failure_reason, accepted) in [
        ("healthy-disabled", false, "disabled", None, None, true),
        ("healthy-enabled", true, "enabled", None, None, true),
        (
            "disabled-flag-mismatch",
            true,
            "disabled",
            None,
            None,
            false,
        ),
        ("enabled-flag-mismatch", false, "enabled", None, None, false),
        ("armed", true, "armed", None, None, false),
        (
            "scheduled",
            true,
            "scheduled",
            Some("2030-01-01T01:00:00Z"),
            None,
            false,
        ),
        ("checking", true, "checking", None, None, false),
        (
            "transient-failure",
            true,
            "transient_failure",
            None,
            Some("usage_unavailable"),
            false,
        ),
        (
            "terminal-failure",
            false,
            "terminal_failure",
            None,
            Some("state_unavailable"),
            false,
        ),
        (
            "enabled-with-schedule",
            true,
            "enabled",
            Some("2030-01-01T01:00:00Z"),
            None,
            false,
        ),
        (
            "enabled-with-failure",
            true,
            "enabled",
            None,
            Some("usage_unavailable"),
            false,
        ),
        (
            "disabled-with-schedule",
            false,
            "disabled",
            Some("2030-01-01T01:00:00Z"),
            None,
            false,
        ),
        (
            "disabled-with-failure",
            false,
            "disabled",
            None,
            Some("state_unavailable"),
            false,
        ),
    ] {
        fs::write(
            session_dir.join("auto-resume.json"),
            serde_json::to_vec(&json!({
                "schema_version": "agent-session.auto-resume.v1",
                "enabled": enabled,
                "state": state,
                "updated_at": "2030-01-01T00:00:00Z",
                "scheduled_at": scheduled_at,
                "failure_reason": failure_reason,
                "attempt": 0,
                "ever_scheduled": false,
            }))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            FreshBootstrap::for_runtime(&context, &record) == FreshBootstrap::ThreadStart,
            accepted,
            "unexpected bootstrap decision for {name}",
        );
    }
}

#[test]
fn fresh_bootstrap_rejects_pre_armed_auto_resume() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("fresh-armed", &tmp.path().join("armed.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    write_create_bootstrap_marker(&record);
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    assert!(
        crate::auto_resume::arm_usage_exhaustion(
            &context,
            &record.id,
            "blocked-turn".to_string(),
            1,
            "2030-01-01T00:00:01Z",
        )
        .unwrap()
    );

    assert_eq!(
        FreshBootstrap::for_runtime(&context, &record),
        FreshBootstrap::Closed
    );
}

#[test]
fn fresh_bootstrap_first_turn_must_match_the_successful_thread_start() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("fresh-bound.sock");
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("fresh-bound", &socket);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    write_create_bootstrap_marker(&record);
    let mut bootstrap = FreshBootstrap::for_runtime(&context, &record);
    assert!(bootstrap.bypasses_create_lock(
        &context,
        &record,
        &json!({ "id": 1, "method": "thread/start", "params": {} }),
    ));
    bootstrap.observe_server(&json!({ "id": 1, "result": { "thread": { "id": "fresh-thread" } } }));
    assert!(!bootstrap.bypasses_create_lock(
        &context,
        &record,
        &json!({
            "id": 2,
            "method": "turn/start",
            "params": { "threadId": "different-thread", "input": [] }
        }),
    ));
    assert_eq!(bootstrap, FreshBootstrap::Closed);
}

#[test]
fn closed_fresh_bootstrap_skips_live_filesystem_validation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let record = record_with_runtime("closed-bootstrap", &tmp.path().join("closed.sock"));
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    BOOTSTRAP_LIVE_CHECKS.with(|checks| checks.set(0));
    let mut bootstrap = FreshBootstrap::Closed;
    assert!(!bootstrap.bypasses_create_lock(
        &context,
        &record,
        &json!({ "id": 1, "method": "turn/start", "params": {} }),
    ));
    assert_eq!(BOOTSTRAP_LIVE_CHECKS.with(std::cell::Cell::get), 0);
}

#[tokio::test]
async fn marker_live_lock_free_turn_attempts_normal_cancellation_before_bypass() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("teardown-window.sock");
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("teardown-window", &socket);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    write_create_bootstrap_marker(&record);
    let mut bootstrap = FreshBootstrap::FirstTurn {
        thread_id: "fresh-thread".to_string(),
    };
    normal_cancellation_attempts()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&record.id);
    assert!(
        cancel_before_tui_mutation(
            &context,
            &record,
            &mut bootstrap,
            &json!({
                "id": 2,
                "method": "turn/start",
                "params": { "threadId": "fresh-thread", "input": [] }
            }),
        )
        .await
        .is_some()
    );
    assert_eq!(
        normal_cancellation_attempts()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&record.id)
            .copied()
            .unwrap_or_default(),
        1
    );
    assert_eq!(bootstrap, FreshBootstrap::Closed);
}

#[tokio::test]
async fn turn_start_account_authority_serializes_a_racing_account_queue() {
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
    let record = record_with_runtime("turn-start-account-fence", &tmp.path().join("app.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();

    let authority = lock_turn_start_account_authority(&context, &record)
        .await
        .expect("the current account must be authorized");
    let queue_context = context.clone();
    let queue_id = record.id.clone();
    let queue_launch_id = record.runtime.as_ref().unwrap().launch_id.clone();
    let (queued_tx, queued_rx) = std::sync::mpsc::channel();
    let queue = std::thread::spawn(move || {
        let result = crate::codex_account::queue_next_account_with_unbound(
            &queue_context,
            &queue_id,
            &queue_launch_id,
            "omega",
        );
        queued_tx.send(result).unwrap();
    });

    assert!(
        queued_rx.recv_timeout(Duration::from_millis(10)).is_err(),
        "a new account intent must not become durable before the authorized turn/start is forwarded"
    );
    drop(authority);
    let queued = queued_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the account queue should proceed after forwarding")
        .unwrap();
    assert_eq!(queued.next.as_ref().map(|next| next.state), Some("queued"));
    queue.join().unwrap();
}

#[tokio::test]
async fn turn_start_account_authority_rejects_a_replacement_runtime() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let expected = record_with_runtime("turn-start-runtime-fence", &tmp.path().join("app.sock"));
    fs::create_dir_all(crate::session_dir(&context, &expected.id)).unwrap();
    crate::write_session_record(&context, &expected).unwrap();

    let mut replacement = expected.clone();
    let runtime = replacement.runtime.as_mut().unwrap();
    runtime.generation += 1;
    runtime.launch_id = "replacement-runtime".to_string();
    crate::write_session_record(&context, &replacement).unwrap();

    assert!(
        lock_turn_start_account_authority(&context, &expected)
            .await
            .is_none(),
        "final authority from a replacement runtime must not authorize the old proxy socket"
    );
}

#[tokio::test]
async fn broker_bound_tui_rejects_account_auth_mutations() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let socket = tmp.path().join("broker-bound-auth.sock");
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("broker-bound-auth", &socket);
    crate::codex_account::set_initial_binding(&mut record, Some("acct1")).unwrap();

    for method in [
        "account/login/start",
        "account/login/cancel",
        "account/logout",
    ] {
        let mut bootstrap = FreshBootstrap::Closed;
        assert!(
            cancel_before_tui_mutation(
                &context,
                &record,
                &mut bootstrap,
                &json!({ "id": 1, "method": method, "params": {} }),
            )
            .await
            .is_none(),
            "broker-bound TUI mutation {method} must be rejected"
        );
    }
}

#[tokio::test]
async fn replacement_lock_cannot_reuse_marker_during_gated_teardown() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("gated-teardown.sock");
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("gated-teardown", &socket);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let create_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let bootstrap_guard = begin_create_bootstrap(&record).unwrap().unwrap();
    assert!(lock_bootstrap_file(&bootstrap_guard.file));
    drop(create_lock);
    let replacement_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();

    BOOTSTRAP_GATE_ATTEMPTS.store(0, std::sync::atomic::Ordering::Relaxed);
    let task_context = context.clone();
    let task_record = record.clone();
    let task = tokio::spawn(async move {
        let mut bootstrap = FreshBootstrap::FirstTurn {
            thread_id: "fresh-thread".to_string(),
        };
        let authorization = cancel_before_tui_mutation(
            &task_context,
            &task_record,
            &mut bootstrap,
            &json!({
                "id": 2,
                "method": "turn/start",
                "params": { "threadId": "fresh-thread", "input": [] }
            }),
        )
        .await;
        (authorization.is_some(), bootstrap)
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if BOOTSTRAP_GATE_ATTEMPTS.load(std::sync::atomic::Ordering::Relaxed) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the blocking bootstrap gate attempt should start within the test deadline");
    assert!(
        BOOTSTRAP_GATE_ATTEMPTS.load(std::sync::atomic::Ordering::Relaxed) >= 1,
        "the test-owned gate attempt must be observable even when parallel tests also use the global probe"
    );
    fs::remove_file(thread_handoff_path(&record).unwrap()).unwrap();
    unlock_bootstrap_file(&bootstrap_guard.file);

    let (authorized, _) = task.await.unwrap();
    assert!(!authorized);
    drop(replacement_lock);
    drop(bootstrap_guard);
}

#[tokio::test]
async fn stalled_upstream_write_times_out_and_releases_bootstrap_gate() {
    let tmp = tempfile::TempDir::new().unwrap();
    let record = record_with_runtime("stalled-write", &tmp.path().join("stalled.sock"));
    fs::create_dir_all(crate::session_dir(
        &CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        },
        &record.id,
    ))
    .unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    crate::write_session_record(&context, &record).unwrap();
    let lifecycle_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let bootstrap_guard = begin_create_bootstrap(&record).unwrap().unwrap();
    let authorization = MutationAuthorization {
        _bootstrap_gate: acquire_create_bootstrap_gate(&record),
        _turn_start_gate: None,
        _account_authority: None,
    };
    assert!(authorization._bootstrap_gate.is_some());
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let teardown = std::thread::spawn(move || {
        bootstrap_guard.finish(|| drop(lifecycle_lock));
        finished_tx.send(()).unwrap();
    });
    assert!(
        finished_rx.recv_timeout(Duration::from_millis(10)).is_err(),
        "teardown must wait while the proxy owns the gate"
    );
    let mut upstream = PendingMessageSink;
    let error = send_proxy_upstream(
        &mut upstream,
        Message::Text("stalled".into()),
        Duration::from_millis(10),
    )
    .await
    .unwrap_err();
    assert_eq!(error, "upstream app-server write timed out");
    drop(authorization);
    finished_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    teardown.join().unwrap();
    assert!(
        crate::try_acquire_session_record_lock(&context, &record.id)
            .unwrap()
            .is_some()
    );
    assert!(!thread_handoff_path(&record).unwrap().exists());
}

#[tokio::test]
async fn fresh_tui_first_turn_does_not_bypass_after_create_marker_is_removed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("fresh-expired.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("fresh-expired", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    write_create_bootstrap_marker(&record);
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let start = receive_json(&mut socket).await;
        respond(
            &mut socket,
            &start,
            json!({ "thread": { "id": "fresh-thread" } }),
        )
        .await;
        loop {
            let request = receive_json(&mut socket).await;
            respond(&mut socket, &request, json!({ "ok": true })).await;
            if request["id"] == 3 {
                break;
            }
        }
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    tui.send(Message::Text(
        json!({ "id": 1, "method": "thread/start", "params": { "cwd": "/repo" } })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 1);
    fs::remove_file(thread_handoff_path(&record).unwrap()).unwrap();
    tui.send(Message::Text(
        json!({
            "id": 2,
            "method": "turn/start",
            "params": { "threadId": "fresh-thread", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let rejected = receive_json(&mut tui).await;
    assert_eq!(rejected["id"], 2);
    assert_eq!(rejected["error"]["code"], -32001);
    drop(lock);
    tui.send(Message::Text(
        json!({ "id": 3, "method": "thread/read", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 3);
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test]
async fn fresh_tui_does_not_bypass_when_auto_resume_state_is_unavailable() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("fresh-unavailable.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("fresh-unavailable", &upstream);
    let session_dir = crate::session_dir(&context, &record.id);
    fs::create_dir_all(&session_dir).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    write_create_bootstrap_marker(&record);
    fs::write(session_dir.join("auto-resume.json"), b"not-json").unwrap();
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let request = receive_json(&mut socket).await;
        respond(&mut socket, &request, json!({ "ok": true })).await;
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    tui.send(Message::Text(
        json!({ "id": 1, "method": "thread/start", "params": { "cwd": "/repo" } })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let rejected = receive_json(&mut tui).await;
    assert_eq!(rejected["id"], 1);
    assert_eq!(rejected["error"]["code"], -32001);
    drop(lock);
    tui.send(Message::Text(
        json!({ "id": 2, "method": "thread/read", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 2);
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test]
async fn fresh_tui_first_turn_requires_a_successful_bound_thread_start() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("fresh-correlation.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("fresh-correlation", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    write_create_bootstrap_marker(&record);
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let start = receive_json(&mut socket).await;
        socket
            .send(Message::Text(
                json!({ "id": start["id"], "error": { "code": -32000, "message": "rejected" } })
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        loop {
            let request = receive_json(&mut socket).await;
            respond(&mut socket, &request, json!({ "ok": true })).await;
            if request["id"] == 3 {
                break;
            }
        }
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    tui.send(Message::Text(
        json!({ "id": 1, "method": "thread/start", "params": { "cwd": "/repo" } })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["error"]["code"], -32000);
    tui.send(Message::Text(
        json!({
            "id": 2,
            "method": "turn/start",
            "params": { "threadId": "unbound-thread", "input": [] }
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let rejected = receive_json(&mut tui).await;
    assert_eq!(rejected["id"], 2);
    assert_eq!(rejected["error"]["code"], -32001);
    drop(lock);
    tui.send(Message::Text(
        json!({ "id": 3, "method": "thread/read", "params": {} })
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(receive_json(&mut tui).await["id"], 3);
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
}

#[tokio::test]
async fn proxy_observer_failure_does_not_interrupt_later_tui_frames() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("observer.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("observer-failure", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
    crate::activity::ingest_codex_app_server_failure_with_kind(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        "thread-a",
        "failed-turn",
        StructuredFailureKind::UsageExhausted,
    )
    .unwrap();
    crate::auto_resume::tick_for_runtime(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
        1_893_456_000,
        &UsageSnapshot {
            authoritative: true,
            has_exhausted_windows: true,
            exhausted_reset_epochs: vec![1_893_456_600],
            soonest_reset_epoch: None,
        },
        |_| panic!("blocked usage must not submit"),
    )
    .unwrap();
    bind_thread(&record, "thread-a").unwrap();
    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        for expected_id in [1, 2] {
            let request = receive_json(&mut socket).await;
            assert_eq!(request["id"], expected_id);
            let result = if expected_id == 1 {
                socket.send(Message::Text(json!({
                    "id":"attention-request", "method":"item/tool/requestUserInput",
                    "params":{"threadId":"thread-b", "turnId":"turn-b", "itemId":"item-b", "questions":[]}
                }).to_string().into())).await.unwrap();
                json!({ "ok": true })
            } else {
                json!({ "ok": true })
            };
            respond(&mut socket, &request, result).await;
        }
        socket.close(None).await.unwrap();
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    for (id, method) in [(1, "thread/read"), (2, "thread/read")] {
        tui.send(Message::Text(
            json!({
                "id": id,
                "method": method,
                "params": { "threadId": "thread-b" }
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
        if id == 1 {
            assert_eq!(receive_json(&mut tui).await["id"], "attention-request");
        }
        assert_eq!(receive_json(&mut tui).await["id"], id);
    }
    server.await.unwrap();
    proxy.await.unwrap().unwrap();
    let view = crate::auto_resume::view_for_record(&context, &record);
    assert_eq!(view.state, "terminal_failure");
    assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
}

#[tokio::test]
async fn proxy_transport_loss_durably_fails_closed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let upstream = tmp.path().join("transport-loss.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("transport-loss", &upstream);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();

    let proxy_args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream: upstream.clone(),
        listen: upstream.with_extension("proxy"),
    };
    let proxy_context = context.clone();
    let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, proxy_args).await });
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        drop(socket);
    });
    let proxy_stream = connect_socket(&upstream.with_extension("proxy"))
        .await
        .unwrap();
    let (tui, _) = tokio_tungstenite::client_async("ws://localhost", proxy_stream)
        .await
        .unwrap();
    server.await.unwrap();
    let result = proxy.await.unwrap();
    assert!(
        result.is_err(),
        "unexpected upstream EOF must fail the proxy"
    );
    drop(tui);

    let state = crate::activity::activity_status(&context, &record.id)
        .unwrap()
        .turn_state;
    assert_eq!(state.phase, crate::activity::TurnPhase::Unknown);
    let view = crate::auto_resume::view_for_record(&context, &record);
    assert!(!view.enabled);
    assert_eq!(view.state, "terminal_failure");
    assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
}

#[tokio::test]
async fn unix_control_recovers_raw_active_turn_after_reconnect_before_steering() {
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("reconnect.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("reconnect-control", &socket);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({"data": ["raw-reconnect-thread"], "nextCursor": null}),
        )
        .await;
        let usage = receive_json(&mut socket).await;
        assert_eq!(usage["method"], "account/rateLimits/read");
        respond(
            &mut socket,
            &usage,
            json!({
                "rateLimits": {
                    "primary": {"usedPercent": 0.0, "resetsAt": null},
                    "secondary": null
                }
            }),
        )
        .await;

        let recovery = receive_json(&mut socket).await;
        assert_eq!(recovery["method"], "thread/turns/list");
        assert_eq!(recovery["params"]["threadId"], "raw-reconnect-thread");
        assert_eq!(recovery["params"]["itemsView"], "notLoaded");
        respond(
            &mut socket,
            &recovery,
            json!({
                "data": [{
                    "id": "raw-recovered-active-turn",
                    "status": "inProgress",
                    "items": []
                }],
                "nextCursor": null,
                "backwardsCursor": null
            }),
        )
        .await;
        let steering = receive_json(&mut socket).await;
        assert_eq!(steering["method"], "turn/steer");
        assert_eq!(
            steering["params"]["expectedTurnId"],
            "raw-recovered-active-turn"
        );
        respond(
            &mut socket,
            &steering,
            json!({"turnId": "raw-recovered-active-turn"}),
        )
        .await;
    });

    let (handle, commands, ready) = starting_control_channel();
    let control_context = context.clone();
    let control_record = record.clone();
    let control =
        tokio::spawn(
            async move { run_control(control_context, control_record, commands, ready).await },
        );
    let projected_turn_id = crate::activity::projected_codex_turn_identifier(
        "runtime-reconnect-control",
        "raw-recovered-active-turn",
    )
    .expect("project recovered active turn");
    assert_eq!(
        handle
            .steer_prompt("mailbox checkpoint", &projected_turn_id)
            .await
            .unwrap(),
        projected_turn_id
    );
    server.await.unwrap();
    drop(handle);
    control.abort();
    let _ = control.await;
}

#[tokio::test]
async fn unix_control_projects_live_failure_and_acknowledges_exact_turn_without_content() {
    let env_lock = GlobalStateLock::new();
    let _broker = EnvGuard::set(
        &env_lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("codex.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let id = "codex-control";
    fs::create_dir_all(crate::session_dir(&context, id)).unwrap();
    let record = SessionRecord {
        schema_version: crate::SESSION_DOCUMENT_VERSION.to_string(),
        id: id.to_string(),
        agent: "codex".to_string(),
        mode: "interactive".to_string(),
        coordination_mode: crate::cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        title_revision: 0,
        cwd: "/repo".to_string(),
        tmux_session: "hs-codex-control".to_string(),
        prompt_file: None,
        log_file: None,
        created_at: "2030-01-01T00:00:00Z".to_string(),
        updated_at: "2030-01-01T00:00:00Z".to_string(),
        provider_resume: None,
        runtime: Some(crate::RuntimeInfo {
            kind: RUNTIME_KIND.to_string(),
            tmux_session: "hs-codex-control".to_string(),
            generation: 1,
            started_at: "2030-01-01T00:00:00Z".to_string(),
            launch_id: "runtime-control".to_string(),
            extra: BTreeMap::from([
                (PROTOCOL_KEY.to_string(), json!(PROTOCOL_VERSION)),
                (SOCKET_KEY.to_string(), json!(display_path(&socket))),
                (
                    PROXY_KEY.to_string(),
                    json!(display_path(&socket.with_extension("proxy"))),
                ),
                (
                    THREAD_HANDOFF_KEY.to_string(),
                    json!(display_path(&socket.with_extension("thread"))),
                ),
                (
                    THREAD_ATTACHED_KEY.to_string(),
                    json!(display_path(&socket.with_extension("attached"))),
                ),
            ]),
        }),
        public_metadata: None,
        agent_args: Vec::new(),
        agent_bin: None,
        extra: BTreeMap::new(),
        lineage: None,
        work: None,
        lineage_adoption: None,
        role: None,
        resume_sidecar_extra: BTreeMap::new(),
    };
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    crate::auto_resume::set_enabled(&context, id, true, "2030-01-01T00:00:00Z").unwrap();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        assert_eq!(initialize["method"], "initialize");
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let loaded = receive_json(&mut socket).await;
        respond(
            &mut socket,
            &loaded,
            json!({ "data": [], "nextCursor": null }),
        )
        .await;
        let loaded = receive_json(&mut socket).await;
        assert_eq!(loaded["method"], "thread/loaded/list");
        respond(
            &mut socket,
            &loaded,
            json!({ "data": ["raw-thread-a"], "nextCursor": null }),
        )
        .await;
        socket
            .send(Message::Text(
                json!({
                    "method": "error",
                    "params": {
                        "threadId": "raw-thread-a",
                        "turnId": "raw-turn-a",
                        "willRetry": false,
                        "error": {
                            "message": "localized secret human error",
                            "codexErrorInfo": "usageLimitExceeded"
                        }
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Text(
                json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": "raw-thread-a",
                        "turn": { "id": "raw-turn-a", "status": "failed" }
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let usage = receive_json(&mut socket).await;
        assert_eq!(usage["method"], "account/rateLimits/read");
        respond(
            &mut socket,
            &usage,
            json!({
                "rateLimits": {
                    "primary": { "usedPercent": 100.0, "resetsAt": 1_900_000_000 },
                    "secondary": { "usedPercent": 10.0, "resetsAt": 1_900_000_100 }
                }
            }),
        )
        .await;
        let explicit_usage = receive_json(&mut socket).await;
        assert_eq!(explicit_usage["method"], "account/rateLimits/read");
        respond(
            &mut socket,
            &explicit_usage,
            json!({
                "rateLimits": {
                    "primary": { "usedPercent": 100.0, "resetsAt": 1_900_000_000 },
                    "secondary": { "usedPercent": 10.0, "resetsAt": 1_900_000_100 }
                }
            }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        assert_eq!(resume["method"], "thread/resume");
        assert_eq!(resume["params"]["threadId"], "raw-thread-a");
        respond(&mut socket, &resume, json!({})).await;
        let continuation = receive_json(&mut socket).await;
        assert_eq!(continuation["method"], "turn/start");
        assert_eq!(continuation["params"]["threadId"], "raw-thread-a");
        respond(
            &mut socket,
            &continuation,
            json!({ "turn": { "id": "acknowledged-turn", "status": "inProgress" } }),
        )
        .await;
        let steering = receive_json(&mut socket).await;
        assert_eq!(steering["method"], "turn/steer");
        assert_eq!(steering["params"]["threadId"], "raw-thread-a");
        assert_eq!(steering["params"]["expectedTurnId"], "acknowledged-turn");
        assert_eq!(steering["params"]["input"][0]["text"], "mailbox checkpoint");
        respond(
            &mut socket,
            &steering,
            json!({ "turnId": "acknowledged-turn" }),
        )
        .await;
    });

    let (handle, commands, ready) = starting_control_channel();
    let control_context = context.clone();
    let control_record = record.clone();
    let control =
        tokio::spawn(
            async move { run_control(control_context, control_record, commands, ready).await },
        );
    let usage = handle.usage().await.unwrap();
    assert!(usage.authoritative);
    assert!(usage.has_exhausted_windows);
    assert_eq!(usage.exhausted_reset_epochs, vec![1_900_000_000]);
    assert_eq!(
        handle.submit("private continuation").await.unwrap(),
        "acknowledged-turn"
    );
    assert_eq!(
        crate::auto_resume::pending_sessions(&context, 1_893_456_000)
            .unwrap()
            .usage_ids,
        vec![id.to_string()]
    );
    crate::codex_account::queue_next_account_with_unbound(&context, id, "runtime-control", "omega")
        .expect("queue the next account during the active turn");
    let projected_turn_id =
        crate::activity::projected_codex_turn_identifier("runtime-control", "acknowledged-turn")
            .expect("project active turn id");
    assert_eq!(
        handle
            .steer_prompt("mailbox checkpoint", &projected_turn_id)
            .await
            .unwrap(),
        projected_turn_id
    );
    server.await.unwrap();
    drop(handle);
    control.abort();
    let _ = control.await;

    let session_dir = crate::session_dir(&context, id);
    let activity = format!(
        "{}\n{}",
        fs::read_to_string(session_dir.join("activity.json")).unwrap(),
        fs::read_to_string(session_dir.join("activity.journal.jsonl")).unwrap()
    );
    assert!(activity.contains("provider_hook"));
    assert!(activity.contains("usage_exhausted"));
    for secret in [
        "raw-thread-a",
        "raw-turn-a",
        "localized secret human error",
        "private continuation",
    ] {
        assert!(!activity.contains(secret));
    }
}
