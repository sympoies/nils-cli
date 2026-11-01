//! A keyed attempt can recover only its own acknowledged native turn.
use super::*;
use pretty_assertions::assert_eq;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Provider {
    busy: Arc<AtomicBool>,
    visible: Arc<AtomicBool>,
    mutations: Arc<AtomicUsize>,
    acknowledge: Arc<AtomicBool>,
    history_kind: Arc<AtomicUsize>,
    client_id: Arc<std::sync::Mutex<Option<String>>>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    _socket_dir: nils_test_support::tempdir::ScopedTempDir,
}

impl Provider {
    fn start(path: &Path, socket_dir: nils_test_support::tempdir::ScopedTempDir) -> Self {
        let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let busy = Arc::new(AtomicBool::new(true));
        let visible = Arc::new(AtomicBool::new(false));
        let mutations = Arc::new(AtomicUsize::new(0));
        let acknowledge = Arc::new(AtomicBool::new(true));
        let history_kind = Arc::new(AtomicUsize::new(0));
        let client_id = Arc::new(std::sync::Mutex::new(None));
        let (stop, mut stopped) = tokio::sync::oneshot::channel();
        let (working, shown, writes) = (busy.clone(), visible.clone(), mutations.clone());
        let (acks, history) = (acknowledge.clone(), history_kind.clone());
        let sent_id = client_id.clone();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let listener = tokio::net::UnixListener::from_std(listener).unwrap();
                    loop {
                        let stream = tokio::select! {
                            _ = &mut stopped => break,
                            accepted = listener.accept() => accepted.unwrap().0,
                        };
                        let (working, shown, writes) =
                            (working.clone(), shown.clone(), writes.clone());
                        let (acks, history) = (acks.clone(), history.clone());
                        let sent_id = sent_id.clone();
                        tokio::spawn(async move {
                            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                            while let Some(Ok(ClientMessage::Text(text))) = socket.next().await {
                                let request: Value = serde_json::from_str(&text).unwrap();
                                let Some(id) = request.get("id") else { continue };
                                let result = match request["method"].as_str().unwrap() {
                                    "initialize" => json!({}),
                                    "thread/loaded/list" => json!({"data":["receipt-thread"],"nextCursor":null}),
                                    "turn/start" => {
                                        assert_eq!(request["params"]["threadId"], "receipt-thread");
                                        assert_eq!(request["params"]["input"][0]["text"], "inspect the workspace");
                                        assert_eq!(writes.fetch_add(1, Ordering::SeqCst), 0);
                                        *sent_id.lock().unwrap() = request["params"]["clientUserMessageId"]
                                            .as_str()
                                            .map(str::to_owned);
                                        if !acks.load(Ordering::SeqCst) { break; }
                                        json!({"turn":{"id":"accepted-turn"}})
                                    }
                                    "thread/read" => json!({"thread":{
                                        "id":"receipt-thread",
                                        "status":{"type":if working.load(Ordering::SeqCst) {"active"} else {"idle"}},
                                        "turns":if shown.load(Ordering::SeqCst) { turns(history.load(Ordering::SeqCst), sent_id.lock().unwrap().as_deref()) } else {json!([])}
                                    }}),
                                    method => panic!("unexpected native RPC: {method}"),
                                };
                                if socket.send(ClientMessage::Text(json!({"id":id,"result":result}).to_string().into())).await.is_err() {
                                    break;
                                }
                            }
                        });
                    }
                });
        });
        Self {
            busy,
            visible,
            mutations,
            acknowledge,
            history_kind,
            client_id,
            stop: Some(stop),
            thread: Some(thread),
            _socket_dir: socket_dir,
        }
    }
}

fn turns(kind: usize, sent_client_id: Option<&str>) -> Value {
    let mut input = json!({"type":"userMessage", "content":[{"type":"text",
        "text":if kind == 2 {"unrelated input"} else {"inspect the workspace"}}]});
    // Kind 5 records the id the daemon sent; kind 6 records a different one.
    match (kind, sent_client_id) {
        (5, Some(id)) => input["clientId"] = json!(id),
        (6, _) => input["clientId"] = json!("another-client-id"),
        _ => {}
    }
    let turn = json!({"id":if kind == 1 {"sibling-turn"} else {"accepted-turn"},
        "items":if kind == 4 {json!([input.clone(), input])} else {json!([input])}});
    if kind == 3 {
        json!([turn.clone(), turn])
    } else {
        json!([turn])
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.thread.take().unwrap().join().unwrap();
    }
}

fn native_fixture(root: &Path) -> (Arc<ServeState>, String, fs::File, Provider) {
    let launch = seed_codex_app_server_session(root, "native-receipt");
    let context = CliContext {
        state_dir: root.to_path_buf(),
        host: None,
    };
    let mut record = load_session_record(&context, "native-receipt").unwrap();
    let (socket_dir, socket_path) = nils_test_support::short_socket_path("native-receipt.sock");
    let runtime_extra = &mut record.runtime.as_mut().unwrap().extra;
    runtime_extra.insert(
        codex_app_server::SOCKET_KEY.to_string(),
        json!(crate::display_path(&socket_path)),
    );
    runtime_extra.insert(
        codex_app_server::PROXY_KEY.to_string(),
        json!(crate::display_path(&socket_path.with_extension("proxy"))),
    );
    record.provider_resume = Some(
        serde_json::from_value(json!({
            "provider":"codex", "session_id":"receipt-thread",
            "captured_at":"2000-01-01T00:00:00Z", "capture_method":"fixture", "resume_args":[]
        }))
        .unwrap(),
    );
    record.extra.insert(
        "codex_account_binding".into(),
        json!({
            "schema_version":"agent-session.codex-account-binding.v1",
            "selected_account":"fixture-account", "revision":1, "state":"bound",
            "applied_runtime_id":launch, "updated_at":"2000-01-01T00:00:00Z"
        }),
    );
    crate::write_session_record(&context, &record).unwrap();
    codex_app_server::replace_thread_binding(&record, "receipt-thread").unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();
    let capability = root.join("sessions/native-receipt/.codex-app-server-proxy-capability");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    fs::write(
        &capability,
        serde_json::to_vec(&json!({
            "schema_version":"agent-session.codex-manual-input-proxy.v1",
            "launch_id":launch, "token":"00000000-0000-4000-8000-000000000001",
            "owner_pid":std::process::id(), "expires_at_epoch_ms":now + 60_000
        }))
        .unwrap(),
    )
    .unwrap();
    let lease = fs::File::open(capability).unwrap();
    // SAFETY: the fixture owns this descriptor throughout the modeled proxy lifetime.
    assert_eq!(unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_SH) }, 0);
    let provider = Provider::start(&socket_path, socket_dir);
    let tmux = guard_tmux(root, "native-receipt");
    let script = fs::read_to_string(&tmux).unwrap().replacen(
        "print('────────────────────────────\\n❯ ' + text + '\\n────────────────────────────')",
        "print('› ' + text + '\\n\\n  status')",
        1,
    );
    fs::write(&tmux, script).unwrap();
    (state(root, Some(TOKEN), tmux), launch, lease, provider)
}

#[tokio::test]
async fn owner_commands_use_codex_tui_without_a_model_turn() {
    for command in [
        "/model",
        "/effort",
        "/status",
        "/compact",
        "/cost",
        "/permissions",
        "/provider-command argument",
        "/reset",
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let (st, launch, _lease, provider) = native_fixture(tmp.path());
        provider.busy.store(false, Ordering::SeqCst);
        let uri = "/sessions/native-receipt/prompt/v2";
        let body = json!({"text":command, "expected_session_incarnation":launch,
            "request_id":"owner-command", "require_idle":false});
        let (status, first) = call(
            router(st.clone()),
            post_json(uri, Some(TOKEN), body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{command}: {first}");
        assert_eq!(first["data"]["receipt"]["outcome"], "submitted");
        assert_eq!(
            fs::read_to_string(tmp.path().join("submitted")).unwrap(),
            command
        );
        assert_eq!(provider.mutations.load(Ordering::SeqCst), 0);
        let (_, replay) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
        assert_eq!(replay, first);
        assert_eq!(
            fs::read_to_string(tmp.path().join("enter-count")).unwrap(),
            "1"
        );
    }
}

#[tokio::test]
async fn owner_commands_codex_login_keeps_broker_managed_account_refusal() {
    for quarantined in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let (st, launch, _lease, provider) = native_fixture(tmp.path());
        provider.busy.store(false, Ordering::SeqCst);
        if quarantined {
            let record = load_session_record(&st.context, "native-receipt").unwrap();
            crate::auth_incident::observe(
                &st.context,
                &record,
                "synthetic-auth-failure",
                crate::auth_incident::AuthSource::CodexAppServer,
                crate::activity::Confidence::Authoritative,
                &jiff::Timestamp::now().to_string(),
            )
            .unwrap();
            let path = tmp
                .path()
                .join("sessions/native-receipt/auth-incidents.json");
            let mut incident: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            incident["incidents"][0]["failover"] = json!({
                "schema_version":"agent-session.auth-failover.v1", "operation_id":"fixture-failover",
                "state":"refused", "reason":"candidate_unavailable", "source_account":"fixture-account",
                "source_binding_revision":1, "target_account":null, "provider_session_id":"receipt-thread",
                "started_at_epoch":1, "deadline_epoch":2, "proof":null,
                "new_runtime_incarnation":null, "new_runtime_generation":null,
                "unknown_operation_count":0, "unknown_in_flight_effects":[],
                "untracked_effects":"unknown", "replayed":false
            });
            fs::write(path, serde_json::to_vec(&incident).unwrap()).unwrap();
            assert_eq!(
                crate::auth_incident::recovery::guard_input(
                    &st.context,
                    &record,
                    Some("/login"),
                    &[SpecialKey::Enter],
                )
                .unwrap_err()
                .code(),
                "auth-quarantined"
            );
            let (status, ordinary) = call(
                router(st.clone()),
                post_json(
                    "/sessions/native-receipt/prompt/v2",
                    Some(TOKEN),
                    json!({"text":"inspect the workspace", "expected_session_incarnation":launch,
                    "request_id":"ordinary", "require_idle":true}),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(ordinary["error"]["code"], "auth-quarantined");
        }
        let (status, result) = call(
            router(st),
            post_json(
                "/sessions/native-receipt/prompt/v2",
                Some(TOKEN),
                json!({"text":"/login", "expected_session_incarnation":launch,
                "request_id":"managed-login", "require_idle":false}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{result}");
        assert_eq!(
            result["error"]["code"],
            if quarantined {
                "auth-quarantined"
            } else {
                "codex-account-login-managed"
            }
        );
        assert_eq!(result["error"]["details"]["receipt"]["outcome"], "refused");
        assert!(!tmp.path().join("buffer").exists());
        assert!(!tmp.path().join("submitted").exists());
        assert_eq!(provider.mutations.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn owner_commands_codex_context_and_admission_refuse_without_delivery() {
    for scenario in [
        "/new",
        "/clear",
        "/resume previous",
        "/fork",
        "/n",
        "busy",
        "draft",
        "runtime",
        "binding",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let (st, launch, _lease, provider) = native_fixture(tmp.path());
        provider.busy.store(scenario == "busy", Ordering::SeqCst);
        if scenario == "draft" {
            fs::write(tmp.path().join("composer"), "owner draft").unwrap();
        }
        if scenario == "binding" {
            let mut record = load_session_record(&st.context, "native-receipt").unwrap();
            record.extra.get_mut("codex_account_binding").unwrap()["applied_runtime_id"] =
                json!("previous-runtime");
            crate::write_session_record(&st.context, &record).unwrap();
        }
        let command = if scenario.starts_with('/') {
            scenario
        } else {
            "/status"
        };
        let (status, result) = call(router(st), post_json(
            "/sessions/native-receipt/prompt/v2", Some(TOKEN),
            json!({"text":command, "expected_session_incarnation":if scenario == "runtime" {"old-launch"} else {&launch},
                "request_id":"refused", "require_idle":false}),
        )).await;
        assert_eq!(status, StatusCode::CONFLICT, "{scenario}: {result}");
        assert_eq!(result["error"]["details"]["receipt"]["outcome"], "refused");
        assert!(!tmp.path().join("buffer").exists());
        assert!(!tmp.path().join("submitted").exists());
        assert_eq!(provider.mutations.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn acknowledged_native_attempt_reconciles_without_another_dispatch() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, launch, _lease, provider) = native_fixture(tmp.path());
    let uri = "/sessions/native-receipt/prompt/v2";
    let mut body = keyed_body(&launch);
    body["require_idle"] = json!(true);
    body["request_id"] = json!("busy-attempt");
    let (_, busy) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), body.clone()),
    )
    .await;
    assert_eq!(
        busy["error"]["details"]["receipt"]["outcome"], "refused",
        "{busy}"
    );
    assert_eq!(
        busy["error"]["details"]["receipt"]["code"], "busy",
        "{busy}"
    );
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 0);

    provider.busy.store(false, Ordering::SeqCst);
    body["request_id"] = json!("accepted-attempt");
    let (_, first) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), body.clone()),
    )
    .await;
    assert_eq!(
        first["error"]["details"]["receipt"]["outcome"], "unknown",
        "{first}"
    );
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);
    assert_eq!(
        first["error"]["details"]["receipt"]["confirmation_diagnostic"],
        "native_input_unconfirmed"
    );
    assert_private_receipts(tmp.path());
    provider.visible.store(true, Ordering::SeqCst);

    let lookup = format!("{uri}/receipts/accepted-attempt?expected_session_incarnation={launch}");
    let restarted = state(tmp.path(), Some(TOKEN), st.tmux_bin.clone());
    let (status, recovered) = call(router(restarted), bearer_get(&lookup)).await;
    assert_eq!(status, StatusCode::OK, "{recovered}");
    assert_eq!(
        recovered["data"]["receipt"]["outcome"], "submitted",
        "{recovered}"
    );
    let (status, replay) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["data"]["receipt"], recovered["data"]["receipt"]);
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);
    assert!(!recovered.to_string().contains("accepted-turn"));
    assert_private_receipts(tmp.path());
}

fn assert_private_receipts(root: &Path) {
    for entry in fs::read_dir(root.join("sessions/native-receipt")).unwrap() {
        let entry = entry.unwrap();
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("prompt-receipt-")
        {
            assert!(
                !fs::read_to_string(entry.path())
                    .unwrap()
                    .contains("inspect the workspace")
            );
        }
    }
}

#[tokio::test]
async fn native_reconciliation_rejects_unrelated_or_unfenced_evidence() {
    for case in [
        "sibling",
        "wrong-input",
        "duplicate-turn",
        "duplicate-input",
        "runtime",
        "provider",
        "account",
        "no-ack",
        "partial-claim",
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let (st, launch, _lease, provider) = native_fixture(tmp.path());
        provider.busy.store(false, Ordering::SeqCst);
        if case == "no-ack" {
            provider.acknowledge.store(false, Ordering::SeqCst);
        }
        let uri = "/sessions/native-receipt/prompt/v2";
        let mut body = keyed_body(&launch);
        body["require_idle"] = json!(true);
        let (_, first) = call(
            router(st.clone()),
            post_json(uri, Some(TOKEN), body.clone()),
        )
        .await;
        assert_eq!(
            first["error"]["details"]["receipt"]["outcome"], "unknown",
            "{case}: {first}"
        );
        assert_eq!(
            first["error"]["details"]["receipt"]["confirmation_diagnostic"],
            if case == "no-ack" {
                "acceptance_unobservable"
            } else {
                "native_input_unconfirmed"
            }
        );
        provider.visible.store(true, Ordering::SeqCst);
        provider.history_kind.store(
            match case {
                "sibling" => 1,
                "wrong-input" => 2,
                "duplicate-turn" => 3,
                "duplicate-input" => 4,
                _ => 0,
            },
            Ordering::SeqCst,
        );
        let path = tmp.path().join("sessions/native-receipt/session.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match case {
            "runtime" => record["runtime"]["launch_id"] = json!("replacement-runtime"),
            "provider" => record["provider_resume"]["session_id"] = json!("replacement-thread"),
            "account" => record["codex_account_binding"]["revision"] = json!(2),
            _ => {}
        }
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        if case == "partial-claim" {
            for entry in fs::read_dir(tmp.path().join("sessions/native-receipt")).unwrap() {
                let entry = entry.unwrap();
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("prompt-receipt-")
                    && entry.path().extension().is_some_and(|ext| ext == "json")
                {
                    fs::write(entry.path(), b"{").unwrap();
                }
            }
        }
        let lookup = format!("{uri}/receipts/attempt-1?expected_session_incarnation={launch}");
        let (_, receipt) = call(router(st.clone()), bearer_get(&lookup)).await;
        assert_eq!(
            receipt["data"]["receipt"]["outcome"], "unknown",
            "{case}: {receipt}"
        );
        let (_, repeated) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
        assert_eq!(
            repeated["error"]["details"]["receipt"]["outcome"], "unknown",
            "{case}: {repeated}"
        );
        assert_eq!(provider.mutations.load(Ordering::SeqCst), 1, "{case}");
    }
}

#[tokio::test]
async fn duplicate_post_confirms_native_input_and_preserves_a_later_composer_draft() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, launch, _lease, provider) = native_fixture(tmp.path());
    provider.busy.store(false, Ordering::SeqCst);
    let uri = "/sessions/native-receipt/prompt/v2";
    let mut body = keyed_body(&launch);
    body["require_idle"] = json!(true);
    let (_, first) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), body.clone()),
    )
    .await;
    assert_eq!(
        first["error"]["details"]["receipt"]["outcome"], "unknown",
        "{first}"
    );
    fs::write(tmp.path().join("composer"), "later user draft").unwrap();
    provider.visible.store(true, Ordering::SeqCst);
    let (_, recovered) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), body.clone()),
    )
    .await;
    assert_eq!(
        recovered["data"]["receipt"]["outcome"], "submitted",
        "{recovered}"
    );
    let lookup = format!("{uri}/receipts/attempt-1?expected_session_incarnation={launch}");
    let (_, receipt) = call(router(st.clone()), bearer_get(&lookup)).await;
    assert_eq!(receipt["data"]["receipt"], recovered["data"]["receipt"]);
    body["text"] = json!("another request");
    let (status, conflict) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"]["code"], "request-id-conflict");
    assert_eq!(
        fs::read_to_string(tmp.path().join("composer")).unwrap(),
        "later user draft"
    );
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lost_native_acknowledgement_is_confirmed_only_by_its_own_client_id() {
    // History kind 5 carries the id the daemon sent; kind 6 carries another id;
    // kind 0 carries none. Every row has the right text in the right thread.
    for (kind, confirmed) in [(5, true), (6, false), (0, false)] {
        let tmp = tempfile::TempDir::new().unwrap();
        let (st, launch, _lease, provider) = native_fixture(tmp.path());
        provider.busy.store(false, Ordering::SeqCst);
        provider.acknowledge.store(false, Ordering::SeqCst);
        let uri = "/sessions/native-receipt/prompt/v2";
        let mut body = keyed_body(&launch);
        body["require_idle"] = json!(true);
        let (_, first) = call(
            router(st.clone()),
            post_json(uri, Some(TOKEN), body.clone()),
        )
        .await;
        assert_eq!(
            first["error"]["details"]["receipt"]["outcome"], "unknown",
            "kind {kind}: {first}"
        );
        assert!(
            provider.client_id.lock().unwrap().is_some(),
            "kind {kind}: turn/start carried no clientUserMessageId"
        );
        provider.visible.store(true, Ordering::SeqCst);
        provider.history_kind.store(kind, Ordering::SeqCst);
        let (status, replay) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
        if confirmed {
            assert_eq!(status, StatusCode::OK, "{replay}");
            assert_eq!(
                replay["data"]["receipt"]["outcome"], "submitted",
                "{replay}"
            );
            assert!(!replay.to_string().contains("accepted-turn"));
            assert_private_receipts(tmp.path());
        } else {
            assert_eq!(
                replay["error"]["details"]["receipt"]["outcome"], "unknown",
                "kind {kind}: {replay}"
            );
        }
        // Confirmation never dispatches again.
        assert_eq!(provider.mutations.load(Ordering::SeqCst), 1, "kind {kind}");
    }
}
