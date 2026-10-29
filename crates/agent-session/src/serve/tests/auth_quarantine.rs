use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn owner_commands_login_recovers_without_admitting_generic_input() {
    let tmp = tempfile::tempdir().unwrap();
    let id = "owner-login";
    let launch = seed_claude_prompt_session(tmp.path(), id);
    let st = state(tmp.path(), Some(TOKEN), guard_tmux(tmp.path(), id));
    make_idle(&st.context, id, &launch);
    let mut record = load_session_record(&st.context, id).unwrap();
    record.provider_resume = Some(
        serde_json::from_value(json!({
            "provider":"claude", "session_id":"owner-conversation", "resume_args":[],
            "captured_at":"2000-01-01T00:00:00Z", "capture_method":"fixture"
        }))
        .unwrap(),
    );
    crate::write_session_record(&st.context, &record).unwrap();
    crate::auth_incident::observe(
        &st.context,
        &record,
        "synthetic-auth-failure",
        crate::auth_incident::AuthSource::ClaudeStopFailure,
        crate::activity::Confidence::Authoritative,
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    let uri = "/sessions/owner-login/prompt/v2";
    for (index, text) in [
        "plain text",
        "/status",
        "/model",
        "/effort",
        "/login\nplain text",
    ]
    .iter()
    .enumerate()
    {
        let body = json!({"text":text, "expected_session_incarnation":launch,
            "request_id":format!("refused-{index}"), "require_idle":true});
        let (status, refusal) = call(router(st.clone()), post_json(uri, Some(TOKEN), body)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{refusal}");
        assert_eq!(refusal["error"]["details"]["receipt"]["outcome"], "refused");
        assert!(!tmp.path().join("submitted").exists());
    }
    let body = json!({"text":"/login", "expected_session_incarnation":launch,
        "request_id":"login", "require_idle":false});
    let (status, _) = call(router(st.clone()), post_json(uri, None, body.clone())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let error = crate::send::send_to_session(
        &st.context,
        crate::send::unguarded_args(
            id.into(),
            Some("/login".into()),
            vec![SpecialKey::Enter],
            false,
            st.tmux_bin.clone(),
        ),
    )
    .unwrap_err();
    assert_eq!(error.code(), "auth-quarantined");
    let (status, result) = call(router(st.clone()), post_json(uri, Some(TOKEN), body)).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["receipt"]["outcome"], "submitted");
    assert_eq!(
        fs::read_to_string(tmp.path().join("submitted")).unwrap(),
        "/login"
    );
    assert_eq!(
        crate::auth_incident::recovery::ensure_input_allowed(&st.context, &record)
            .unwrap_err()
            .code(),
        "auth-quarantined"
    );
}

#[tokio::test]
async fn owner_commands_use_claude_tui_without_waiting_for_a_model_turn() {
    for command in [
        "/model",
        "/effort",
        "/status",
        "/compact",
        "/cost",
        "/permissions",
        "/provider-command argument",
        "/fork",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let id = "owner-command";
        let launch = seed_claude_prompt_session(tmp.path(), id);
        let st = state(tmp.path(), Some(TOKEN), guard_tmux(tmp.path(), id));
        make_idle(&st.context, id, &launch);
        let mut record = load_session_record(&st.context, id).unwrap();
        record.provider_resume = Some(
            serde_json::from_value(json!({
                "provider":"claude", "session_id":"owner-conversation", "resume_args":[],
                "captured_at":"2000-01-01T00:00:00Z", "capture_method":"fixture"
            }))
            .unwrap(),
        );
        crate::write_session_record(&st.context, &record).unwrap();
        let (status, result) = call(
            router(st.clone()),
            post_json(
                "/sessions/owner-command/prompt/v2",
                Some(TOKEN),
                json!({"text":format!("  {command}"), "expected_session_incarnation":launch,
                "request_id":"owner-command", "require_idle":false}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{command}: {result}");
        assert_eq!(result["data"]["receipt"]["outcome"], "submitted");
        assert_eq!(
            fs::read_to_string(tmp.path().join("submitted")).unwrap(),
            command
        );
        let record = load_session_record(&st.context, id).unwrap();
        assert!(
            activity::state_for_view(&st.context, &record)
                .unwrap()
                .current_turn
                .is_none()
        );
    }
}

#[tokio::test]
async fn owner_commands_refuse_context_replacement_before_tui_input() {
    for command in [
        "/clear",
        "/new",
        "/reset",
        "/resume previous",
        "/branch",
        "/teleport",
        "/tp",
        "/n",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let id = "context-command";
        let launch = seed_claude_prompt_session(tmp.path(), id);
        let st = state(tmp.path(), Some(TOKEN), guard_tmux(tmp.path(), id));
        make_idle(&st.context, id, &launch);
        let (status, result) = call(
            router(st),
            post_json(
                "/sessions/context-command/prompt/v2",
                Some(TOKEN),
                json!({"text":command, "expected_session_incarnation":launch,
                "request_id":"context", "require_idle":true}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{command}: {result}");
        assert_eq!(result["error"]["code"], "send-context-command-unsupported");
        assert_eq!(result["error"]["details"]["receipt"]["outcome"], "refused");
        assert!(!tmp.path().join("buffer").exists());
        assert!(!tmp.path().join("submitted").exists());
    }
}

#[tokio::test]
async fn auth_quarantine_http_receipt_is_refused_without_pasting_or_replay() {
    for require_idle in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let id = "auth-quarantine-http";
        let launch = seed_claude_prompt_session(tmp.path(), id);
        let st = state(tmp.path(), Some(TOKEN), guard_tmux(tmp.path(), id));
        let record = load_session_record(&st.context, id).unwrap();
        crate::auth_incident::observe(
            &st.context,
            &record,
            "synthetic-auth-failure",
            crate::auth_incident::AuthSource::ClaudeStopFailure,
            crate::activity::Confidence::Authoritative,
            &jiff::Timestamp::now().to_string(),
        )
        .unwrap();
        let mut body = keyed_body(&launch);
        body["require_idle"] = json!(require_idle);
        let uri = "/sessions/auth-quarantine-http/prompt/v2";
        let (status, first) = call(
            router(st.clone()),
            post_json(uri, Some(TOKEN), body.clone()),
        )
        .await;
        assert_eq!(
            first["error"]["details"]["receipt"]["outcome"], "refused",
            "{first}"
        );
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(first["error"]["code"], "auth-quarantined");
        let (status, repeat) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(repeat, first);
        assert!(!tmp.path().join("submitted").exists());
        assert!(!tmp.path().join("buffer").exists());
    }
}
