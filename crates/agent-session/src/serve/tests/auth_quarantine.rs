use super::*;
use pretty_assertions::assert_eq;

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
