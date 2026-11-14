//! Permission answering uses operator auth, including before request parsing.
use super::super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn permission_http_requires_operator_bearer_before_read_or_answer() {
    let tmp = tempfile::tempdir().unwrap();
    let st = state(tmp.path(), Some(TOKEN), PathBuf::from("/unavailable/tmux"));
    for token in [None, Some("coordination-only-credential")] {
        for request in [
            Request::builder()
                .uri("/sessions/absent/permissions")
                .method("GET")
                .header(
                    "authorization",
                    format!("Bearer {}", token.unwrap_or_default()),
                )
                .body(Body::empty())
                .unwrap(),
            post_json(
                "/sessions/absent/permissions",
                token,
                json!({"request_id":"request","call_id":"call","decision":"allow"}),
            ),
        ] {
            let (status, body) = call(router(st.clone()), request).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(body["error"]["code"], "unauthorized");
        }
    }
    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/absent/permissions",
            Some(TOKEN),
            json!({"decision":"approve-all"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid-json-body");
}

#[tokio::test]
async fn claude_permission_http_refuses_keys_even_with_allow_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    let (tmux, journal) = recording_tmux(tmp.path());
    seed_session_with_runtime(tmp.path(), "blocked", "claude", "hs-claude-blocked");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let record = load_session_record(&st.context, "blocked").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    crate::activity::ingest_event(&st.context, "blocked", serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION, "event_id":"clarification-first",
        "runtime_id":"launch-blocked", "provider":"claude", "kind":"attention_requested",
        "confidence":"authoritative", "attention_id":"question-first", "attention_kind":"clarification"
    })).unwrap()).unwrap();
    block_on_provider_attention(&st, "blocked", "claude");
    let (status, body) = call(
        router(st),
        post_json(
            "/sessions/blocked/send",
            Some(TOKEN),
            json!({"keys":["enter"],"allow_blocked":true}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "claude-permission-hook-required");
    assert!(tmux_never_ran(&journal));
}

#[test]
fn claude_permission_ingress_cannot_overtake_structured_prompt_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let (tmux, _) = super::recording::tmux(tmp.path());
    let launch = seed_claude_prompt_session(tmp.path(), "structured");
    let st = state(tmp.path(), Some(TOKEN), tmux.clone());
    let record = load_session_record(&st.context, "structured").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let probe = tmp.path().join("probe.py");
    fs::write(
        &probe,
        r#"import fcntl, pathlib, sys
root = pathlib.Path(sys.argv[1])
verb = sys.argv[2]
(root / ('seen-' + verb)).touch()
with (root / 'sessions/structured/.activity-ingress.lock').open('a+') as ingress:
    try:
        fcntl.flock(ingress, fcntl.LOCK_SH | fcntl.LOCK_NB)
    except BlockingIOError:
        pass
    else:
        (root / ('overtook-' + verb)).touch()
"#,
    )
    .unwrap();
    let script = fs::read_to_string(&tmux).unwrap().replacen(
        "case \"$1\" in",
        &format!("case \"$1\" in\n  paste-buffer|send-keys) python3 '{}' '{}' \"$1\"; [ \"$1\" = paste-buffer ] && exit 0; exit 73 ;;", probe.display(), tmp.path().display()),
        1,
    );
    fs::write(&tmux, script).unwrap();
    let _ = crate::prompt_delivery::deliver_claude_structured_prompt_locked(
        &st.context,
        "structured",
        &launch,
        "please continue",
        &tmux,
    );
    for verb in ["paste-buffer", "send-keys"] {
        assert!(
            tmp.path().join(format!("seen-{verb}")).exists(),
            "fixture did not reach {verb}"
        );
        assert!(
            !tmp.path().join(format!("overtook-{verb}")).exists(),
            "provider ingress overtook structured {verb}"
        );
    }
}
