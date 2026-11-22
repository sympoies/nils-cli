use crate::serve::*;
use axum::http::Request;
use pretty_assertions::assert_eq;
use tower::ServiceExt;

const TOKEN: &str = "fixture-metrics-token";

async fn request(app: Router, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::builder().uri(uri);
    if let Some(token) = token {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn metrics_authentication_precedes_lookup_and_unconfigured_auth_fails_closed() {
    let tmp = tempfile::tempdir().unwrap();
    for configured in [None, Some(TOKEN)] {
        let state = tests::state(tmp.path(), configured, PathBuf::from("/unused-tmux"));
        for token in [None, Some("wrong")] {
            let (status, _) =
                request(router(state.clone()), "/sessions/missing/metrics/v1", token).await;
            assert_eq!(
                status,
                if configured.is_some() {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                }
            );
        }
        if configured.is_some() {
            let (status, _) =
                request(router(state), "/sessions/missing/metrics/v1", Some(TOKEN)).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
        }
    }
}

#[tokio::test]
async fn metrics_response_has_only_typed_allowlisted_fields_and_explicit_unrecorded_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let st = tests::state(tmp.path(), Some(TOKEN), PathBuf::from("/unused-tmux"));
    let record: crate::SessionRecord = serde_json::from_value(json!({
        "schema_version":"agent-session.session.v1","id":"synthetic","agent":"claude","mode":"interactive",
        "cwd":"/private/PATH_CANARY","tmux_session":"synthetic","created_at":"invalid-TIMESTAMP_CANARY","updated_at":"2026-01-01T00:00:00Z",
        "title":"TITLE_CANARY","account":"ACCOUNT_CANARY","unknown":{"text":"FIELD_CANARY"}
    })).unwrap();
    crate::write_session_record(&st.context, &record).unwrap();
    let (status, body) = request(router(st), "/sessions/synthetic/metrics/v1", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    let data = &body["data"];
    let metrics = &data["metrics"];
    let expected = json!({
        "schema_version":"agent-session.session-metrics.v1","observed_at":metrics["observed_at"],"provider":"claude",
        "transcript_scope":"selected_primary_file","compaction_scope":"retained_transcript",
        "transcript_bytes":{"value":null,"unknown_reason":"not_bound"},
        "transcript_lines":{"value":null,"unknown_reason":"not_bound"},
        "completed_compactions":{"value":null,"unknown_reason":"not_bound"},
        "managed_age_seconds":{"value":null,"unknown_reason":"invalid_timestamp"},"managed_created_at":null,
        "conversation_age_seconds":{"value":null,"unknown_reason":"provider_creation_unavailable"},"conversation_created_at":null,
        "resume":{"kind":"unknown","outcome":"unknown","unknown_reason":"not_recorded","started_at":null,"completed_at":null,"deadline_at":null,"duration_ms":null}
    });
    assert_eq!(metrics, &expected);
    assert_eq!(data.as_object().unwrap().len(), 2);
    assert!(
        metrics["observed_at"]
            .as_str()
            .unwrap()
            .parse::<jiff::Timestamp>()
            .is_ok()
    );
    assert!(!body.to_string().contains("CANARY"));
}

#[tokio::test]
async fn selected_file_scan_is_shared_and_list_board_show_add_zero_metric_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("CONFIG_CANARY");
    let path = config.join("projects/SOURCE_CANARY/conversation.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let content = format!(
        "{}\n{}\n",
        json!({"type":"user","sessionId":"conversation","cwd":"/workspace/PATH_CANARY","message":{"content":"TEXT_CANARY"},"unknown":"FIELD_CANARY"}),
        json!({"type":"system","subtype":"compact_boundary","sessionId":"conversation","uuid":"boundary"})
    );
    fs::write(&path, &content).unwrap();
    let mut state = tests::state(tmp.path(), Some(TOKEN), PathBuf::from("/unused-tmux"));
    Arc::get_mut(&mut state).unwrap().board = true;
    let record: crate::SessionRecord = serde_json::from_value(json!({
        "schema_version":"agent-session.session.v1","id":"synthetic","agent":"claude","mode":"interactive",
        "cwd":"/workspace/example","tmux_session":"synthetic","created_at":"2026-01-02T00:00:00Z","updated_at":"2026-01-02T00:00:00Z",
        "provider_resume":{"provider":"claude","session_id":"conversation","captured_at":"2026-01-02T00:00:00Z","capture_method":"fixture","resume_args":[]},
        "runtime":{"kind":"tmux","tmux_session":"synthetic","generation":1,"started_at":"2026-01-02T00:00:00Z","launch_id":"runtime","agent_profile_provider_config_dir":config,"account":"ACCOUNT_CANARY"}
    })).unwrap();
    crate::write_session_record(&state.context, &record).unwrap();
    let scans = crate::context_usage::session_metrics::scans_for;
    let app = router(state.clone());
    let (status, _) = request(app.clone(), "/sessions/synthetic/metrics/v1", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(scans(&path), 0);
    let (status, body) = request(app.clone(), "/sessions/synthetic/metrics/v1", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["data"]["metrics"]["transcript_bytes"],
        json!({"value":content.len(),"unknown_reason":null})
    );
    assert_eq!(
        body["data"]["metrics"]["transcript_lines"],
        json!({"value":2,"unknown_reason":null})
    );
    assert_eq!(
        body["data"]["metrics"]["completed_compactions"],
        json!({"value":1,"unknown_reason":null})
    );
    assert!(!body.to_string().contains("CANARY"));
    assert_eq!(scans(&path), 1, "one scan supplies both full-file counts");
    for uri in ["/sessions", "/board/v1"] {
        let (status, body) = request(app.clone(), uri, Some(TOKEN)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body.to_string().contains("session-metrics.v1"));
    }
    let show = crate::session_view(
        &state.context,
        &record,
        Some("stopped".into()),
        Some(&state.tmux_bin),
    );
    assert!(
        !serde_json::to_string(&show)
            .unwrap()
            .contains("session-metrics.v1")
    );
    assert_eq!(
        scans(&path),
        1,
        "list/board/show must not invoke the metric reader"
    );
    fs::remove_file(&path).unwrap();
    let (status, body) = request(app, "/sessions/synthetic/metrics/v1", Some(TOKEN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["data"]["metrics"]["transcript_lines"],
        json!({"value":null,"unknown_reason":"source_unavailable"})
    );
    assert!(!body.to_string().contains("CANARY"));
}

#[tokio::test]
async fn resume_metrics_projects_claude_timing_and_explicit_unknowns() {
    assert_resume_metrics("claude").await;
}

#[tokio::test]
async fn resume_metrics_projects_codex_timing_and_explicit_unknowns() {
    assert_resume_metrics("codex").await;
}

async fn assert_resume_metrics(provider: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let state = tests::state(tmp.path(), Some(TOKEN), PathBuf::from("/unused-tmux"));
    let app = router(state.clone());
    let config = tmp.path().join("CONFIG_CANARY");
    let uri = "/sessions/synthetic/metrics/v1";
    let (transcript, header) = if provider == "claude" {
        (
            config.join("projects/fixture/CONVERSATION_CANARY.jsonl"),
            json!({"type":"user","sessionId":"CONVERSATION_CANARY","cwd":tmp.path(),
                "message":{"content":"TRANSCRIPT_CANARY"}}),
        )
    } else {
        (
            config.join("sessions/rollout.jsonl"),
            json!({"type":"session_meta","payload":{"id":"CONVERSATION_CANARY","cwd":tmp.path(),
                "source":"cli","timestamp":"2026-01-01T00:00:00Z"},"text":"TRANSCRIPT_CANARY"}),
        )
    };
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(&transcript, format!("{header}\n")).unwrap();
    let mut record: crate::SessionRecord = serde_json::from_value(json!({
        "schema_version":"agent-session.session.v1","id":"synthetic","agent":provider,
        "mode":"interactive","cwd":tmp.path(),"tmux_session":"synthetic",
        "created_at":"2026-01-02T00:00:00Z","updated_at":"2026-01-02T00:00:00Z",
        "provider_resume":{"provider":provider,"session_id":"CONVERSATION_CANARY","captured_at":"2026-01-02T00:00:00Z","capture_method":"fixture","resume_args":[]},
        "runtime":{"kind":if provider == "claude" {"tmux"} else {"codex_app_server"},
            "tmux_session":"synthetic","generation":1,"started_at":"2026-01-02T00:00:00Z",
            "launch_id":"runtime","agent_profile_provider_config_dir":config}
    })).unwrap();
    let (_socket_dir, upstream) = nils_test_support::short_socket_path("metrics.sock");
    let listen = upstream.with_extension("proxy");
    if provider == "codex" {
        let extra = &mut record.runtime.as_mut().unwrap().extra;
        for (key, value) in [
            (
                crate::codex_app_server::PROTOCOL_KEY,
                json!(crate::codex_app_server::PROTOCOL_VERSION),
            ),
            (crate::codex_app_server::SOCKET_KEY, json!(upstream)),
            (crate::codex_app_server::PROXY_KEY, json!(listen)),
            (
                crate::codex_app_server::THREAD_HANDOFF_KEY,
                json!(tmp.path().join("handoff")),
            ),
            (
                crate::codex_app_server::THREAD_ATTACHED_KEY,
                json!(tmp.path().join("attached")),
            ),
        ] {
            extra.insert(key.into(), value);
        }
    }
    crate::write_session_record(&state.context, &record).unwrap();
    let path = crate::session_dir(&state.context, &record.id).join("resume-metrics.json");
    let (_, baseline) = request(app.clone(), uri, Some(TOKEN)).await;
    assert_eq!(baseline["data"]["metrics"]["transcript_lines"]["value"], 1);
    assert!(!baseline.to_string().contains("CANARY"));
    assert_eq!(
        baseline["data"]["metrics"]["resume"]["unknown_reason"],
        "not_recorded"
    );

    let runtime = record.runtime.as_ref().unwrap();
    let resume = record.provider_resume.as_ref().unwrap();
    let binding = |kind| {
        crate::coordination::digest_bytes(
            &serde_json::to_vec(&(
                &record.id,
                &record.created_at,
                &record.agent,
                &runtime.kind,
                &runtime.launch_id,
                runtime.generation,
                (kind == "resume").then_some((&resume.provider, &resume.session_id)),
                crate::session_provider_config_dir(&record),
                crate::session_effective_provider_config_dir(&record),
                crate::session_agent_profile(&record),
            ))
            .unwrap(),
        )
    };
    let mut clock = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut clock) },
        0
    );
    let tick = clock.tv_sec as u64 * 1_000_000_000 + clock.tv_nsec as u64;
    #[cfg(target_os = "linux")]
    let boot_id = crate::linux_boot_id().unwrap();
    #[cfg(target_os = "macos")]
    let boot_id = crate::capture_macos_boot_id().unwrap();
    let now = jiff::Timestamp::now();
    let started_at = now.checked_sub(jiff::SignedDuration::from_secs(1)).unwrap();
    let start = json!({"at":started_at.to_string(),"tick_ns":tick - 1_000_000_000});
    let end = json!({"at":now.to_string(),"tick_ns":tick - 832_000_000});
    let save = |receipt: Value| {
        crate::write_private_file(&path, &serde_json::to_vec(&receipt).unwrap()).unwrap();
    };
    for (kind, finish, outcome, duration, reason) in [
        (
            "initial",
            Value::Null,
            "not_applicable",
            Value::Null,
            json!("not_applicable"),
        ),
        (
            "resume",
            Value::Null,
            "pending",
            Value::Null,
            json!("pending"),
        ),
        (
            "resume",
            json!({"outcome":"succeeded","sample":end}),
            "succeeded",
            json!(168),
            Value::Null,
        ),
        (
            "resume",
            json!({"outcome":"provider_failed","sample":end}),
            "failed",
            Value::Null,
            json!("provider_failed"),
        ),
        (
            "resume",
            json!({"outcome":"launch_failed","sample":end}),
            "failed",
            Value::Null,
            json!("launch_failed"),
        ),
        (
            "resume",
            json!({"outcome":"unsupported_runtime","sample":null}),
            "unknown",
            Value::Null,
            json!("unsupported_runtime"),
        ),
    ] {
        save(
            json!({"binding":binding(kind), "kind":kind, "boot_id":boot_id,
            "start":start, "finish":finish}),
        );
        let (status, body) = request(app.clone(), uri, Some(TOKEN)).await;
        assert_eq!(status, StatusCode::OK);
        let timing = &body["data"]["metrics"]["resume"];
        assert_eq!(&timing["kind"], kind, "{provider}");
        assert_eq!(&timing["outcome"], outcome, "{provider}");
        assert_eq!(timing["duration_ms"], duration, "{provider}");
        assert_eq!(timing["unknown_reason"], reason, "{provider}");
        assert_eq!(timing.as_object().unwrap().len(), 7);
        assert!(!body.to_string().contains("CANARY"));
    }
    save(
        json!({"binding":binding("resume"), "kind":"resume", "boot_id":"foreign-boot",
        "start":start, "finish":null}),
    );
    let (_, body) = request(app.clone(), uri, Some(TOKEN)).await;
    assert_eq!(
        body["data"]["metrics"]["resume"]["unknown_reason"],
        "clock_unavailable"
    );

    if provider == "claude" {
        save(
            json!({"binding":binding("resume"), "kind":"resume", "boot_id":boot_id,
            "start":start, "finish":null}),
        );
        let send_hook = |source, runtime_id, provider_id, subagent| {
            let mut event = json!({"hook_event_name":"SessionStart", "source":source,
                "session_id":provider_id, "extra":"HOOK_CANARY"});
            if subagent {
                event["agent_id"] = json!("subagent");
            }
            crate::activity::ingest_provider_hook_input(
                &state.context,
                crate::AgentKind::Claude,
                None,
                crate::activity::ProviderHookInput {
                    id: &record.id,
                    runtime_id,
                    payload: &serde_json::to_vec(&event).unwrap(),
                    attention_authority: None,
                },
            )
        };
        crate::activity::activate_runtime(&state.context, &record).unwrap();
        let conversation = &record.provider_resume.as_ref().unwrap().session_id;
        for (source, runtime, id, subagent) in [
            ("startup", "runtime", conversation.as_str(), false),
            ("compact", "runtime", conversation.as_str(), false),
            ("resume", "old-runtime", conversation.as_str(), false),
            ("resume", "runtime", "different-conversation", false),
            ("resume", "runtime", conversation.as_str(), true),
        ] {
            let _ = send_hook(source, runtime, id, subagent);
            let (_, body) = request(app.clone(), uri, Some(TOKEN)).await;
            assert_eq!(body["data"]["metrics"]["resume"]["outcome"], "pending");
        }
        let projected = crate::activity::projected_provider_identifier(
            "runtime",
            crate::AgentKind::Claude,
            "session",
            conversation,
        )
        .unwrap();
        assert!(send_hook("resume", "runtime", &projected, false).unwrap());
        let (_, body) = request(app.clone(), uri, Some(TOKEN)).await;
        let timing = &body["data"]["metrics"]["resume"];
        assert_eq!(timing["outcome"], "succeeded");
        assert!(timing["duration_ms"].as_u64().is_some());
        assert!(!body.to_string().contains("CANARY"));
        assert!(send_hook("resume", "runtime", &projected, false).unwrap());
        let (_, duplicate) = request(app.clone(), uri, Some(TOKEN)).await;
        assert_eq!(&duplicate["data"]["metrics"]["resume"], timing);
    }

    if provider == "codex" {
        save(
            json!({"binding":binding("resume"), "kind":"resume", "boot_id":boot_id,
            "start":start, "finish":null}),
        );
        observe_synthetic_codex_resume(app.clone(), &state.context, &record, upstream, listen)
            .await;
        let (_, body) = request(app.clone(), uri, Some(TOKEN)).await;
        let timing = &body["data"]["metrics"]["resume"];
        assert_eq!(timing["outcome"], "succeeded");
        assert!(timing["duration_ms"].as_u64().is_some());
        assert!(!body.to_string().contains("CANARY"));
    }

    record.provider_resume.as_mut().unwrap().session_id = "replacement".into();
    crate::write_session_record(&state.context, &record).unwrap();
    let (_, body) = request(app.clone(), uri, Some(TOKEN)).await;
    assert_eq!(
        body["data"]["metrics"]["resume"]["unknown_reason"],
        "identity_changed"
    );
    fs::remove_file(path).unwrap();
}

async fn observe_synthetic_codex_resume(
    app: Router,
    context: &crate::CliContext,
    record: &crate::SessionRecord,
    upstream: PathBuf,
    listen: PathBuf,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let conversation = record.provider_resume.as_ref().unwrap().session_id.clone();
    let args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream,
        listen: listen.clone(),
    };
    let context = context.clone();
    let proxy =
        tokio::task::spawn_blocking(move || crate::codex_app_server::run_proxy(&context, args));
    let (wrong_reply_checked, confirm) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let mut confirm = Some(confirm);
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let message = socket.next().await.unwrap().unwrap();
        let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        assert_eq!(request["method"], "thread/resume");
        assert_eq!(request["params"]["threadId"], conversation);
        for (id, thread) in [(1, conversation.as_str()), (2, conversation.as_str())] {
            socket
                .send(Message::Text(
                    json!({"id":id,"result":{"thread":{
                        "id":thread,"turns":[{"text":"PROVIDER_CANARY"}]
                    }}})
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            if let Some(confirm) = confirm.take() {
                confirm.await.unwrap();
            }
        }
        socket.close(None).await.unwrap();
    });
    let stream = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(stream) = tokio::net::UnixStream::connect(&listen).await {
                break stream;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", stream)
        .await
        .unwrap();
    let thread = &record.provider_resume.as_ref().unwrap().session_id;
    tui.send(Message::Text(
        json!({"id":2,"method":"thread/resume","params":{"threadId":thread}})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let mut wrong_reply_checked = Some(wrong_reply_checked);
    for id in [1, 2] {
        let message = tui.next().await.unwrap().unwrap();
        let reply: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        assert_eq!(reply["id"], id);
        if let Some(checked) = wrong_reply_checked.take() {
            let (_, body) =
                request(app.clone(), "/sessions/synthetic/metrics/v1", Some(TOKEN)).await;
            assert_eq!(body["data"]["metrics"]["resume"]["outcome"], "pending");
            checked.send(()).unwrap();
        }
    }
    peer.await.unwrap();
    drop(tui);
    let _ = proxy.await.unwrap();
}
