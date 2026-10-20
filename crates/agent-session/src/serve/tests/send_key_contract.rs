//! The CLI's printable keys must not widen the web named-key contract.
use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn send_rejects_unknown_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tmux = minimal_tmux(tmp.path());
    seed_session(tmp.path(), "steer", "codex", "hs-codex-steer");
    let st = state(tmp.path(), Some(TOKEN), tmux);
    for key in ["banana", "s"] {
        let (status, body) = call(
            router(st.clone()),
            post_json(
                "/sessions/steer/send",
                Some(TOKEN),
                json!({ "keys": [key] }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "key={key}, body={body}");
        assert_eq!(body["error"]["code"], "invalid-key");
    }
}

#[tokio::test]
async fn websocket_ignores_unknown_keys_and_delivers_named_keys() {
    let tmp = tempfile::TempDir::new().unwrap();
    let log = tmp.path().join("calls.log");
    let tmux = logging_tmux(tmp.path(), &log);
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = test_record("key-contract", "hs-codex-key-contract");
    crate::write_session_record(&context, &record).unwrap();
    let target = "hs-codex-key-contract:0.0";
    let mut repaint = false;
    let resize_lock = tokio::sync::Mutex::new(());

    for key in ["banana", "s"] {
        for frame in [json!({"key": key}), json!({"keys": [key]})] {
            handle_input(
                &context,
                &tmux,
                &record,
                target,
                &frame.to_string(),
                &mut repaint,
                &resize_lock,
            )
            .await
            .unwrap();
            assert_eq!(fs::read_to_string(&log).unwrap_or_default(), "");
        }
    }

    handle_input(
        &context,
        &tmux,
        &record,
        target,
        r#"{"key":"backspace"}"#,
        &mut repaint,
        &resize_lock,
    )
    .await
    .unwrap();
    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .contains("send-keys -t hs-codex-key-contract:0.0 BSpace")
    );
}
