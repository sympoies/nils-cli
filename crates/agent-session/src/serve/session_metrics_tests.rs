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
