//! Original native command-UI anchors protect owner-only dispatch receipts.
use super::*;
use pretty_assertions::assert_eq;

/// Rows of one retained native capture, in their original order. Omitted body
/// rows are not reconstructed, so a pane is only ever a subset of a real frame.
fn rows(capture: &Value) -> Vec<&str> {
    capture["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["styled"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn owner_command_ui_is_dispatched_without_a_model_submission() {
    let status: Value = serde_json::from_str(include_str!(
        "../../send_submit/composer/command-ui-native.json"
    ))
    .unwrap();
    let claude: Value = serde_json::from_str(include_str!(
        "../../send_submit/composer/command-ui-claude-2.1.296.json"
    ))
    .unwrap();
    let frame = |name: &str| rows(&claude["frames"][name]).join("\n");
    // (scenario, owner text, retained pane, positive native command UI)
    let scenarios = [
        ("owner", "/status", rows(&status).join("\n"), true),
        (
            "generic-prompt",
            "inspect the workspace",
            rows(&status).join("\n"),
            false,
        ),
        (
            "unknown-ui",
            "/status",
            "unrecognized pane".to_owned(),
            false,
        ),
        (
            "model-picker",
            "/model",
            frame("model_picker_settled"),
            true,
        ),
        (
            "effort-picker",
            "/effort",
            frame("effort_picker_settled"),
            true,
        ),
        // Pre-picker autocomplete, the dismissed picker's composer, and a
        // picker left in history above a live composer are not command UI.
        (
            "model-autocomplete",
            "/model",
            frame("model_prepicker_autocomplete"),
            false,
        ),
        (
            "effort-composer-after-escape",
            "/effort",
            frame("plain_composer_after_escape_effort"),
            false,
        ),
        (
            "historical-picker",
            "/model",
            format!(
                "{}\n{}",
                frame("model_picker_settled"),
                frame("plain_composer_after_escape_model")
            ),
            false,
        ),
    ];
    for (scenario, text, pane, dispatched) in scenarios {
        let tmp = tempfile::tempdir().unwrap();
        let id = "owner-command-ui";
        let launch = seed_claude_prompt_session(tmp.path(), id);
        let tmux = guard_tmux(tmp.path(), id);
        let st = state(tmp.path(), Some(TOKEN), tmux.clone());
        make_idle(&st.context, id, &launch);
        let mut record = load_session_record(&st.context, id).unwrap();
        record.provider_resume = Some(
            serde_json::from_value(json!({"provider":"claude", "session_id":"owner-conversation",
                "resume_args":[], "captured_at":"2000-01-01T00:00:00Z", "capture_method":"fixture"}))
            .unwrap(),
        );
        crate::write_session_record(&st.context, &record).unwrap();
        fs::write(tmp.path().join("command-ui"), &pane).unwrap();
        let script = fs::read_to_string(&tmux).unwrap().replace(
            "if args[0] == 'capture-pane':\n",
            "if args[0] == 'capture-pane':\n    if (root / 'submitted').exists():\n        import re\n        shown = (root / 'command-ui').read_text()\n        print(shown if '-e' in args else re.sub(r'\\x1b\\[[0-9;]*m', '', shown)); sys.exit(0)\n",
        );
        fs::write(&tmux, script).unwrap();
        let request = json!({"text":text, "expected_session_incarnation":launch,
            "request_id":"command-ui", "require_idle":true});
        let uri = "/sessions/owner-command-ui/prompt/v2";
        let (status, first) = call(
            router(st.clone()),
            post_json(uri, Some(TOKEN), request.clone()),
        )
        .await;
        if dispatched {
            assert_eq!(status, StatusCode::OK, "{scenario}: {first}");
            assert_eq!(first["data"]["submitted"], false, "{scenario}");
            assert_eq!(first["data"]["command_dispatched"], true, "{scenario}");
            assert_eq!(first["data"]["receipt"]["outcome"], "command_dispatched");
            assert_eq!(first["data"]["receipt"]["code"], Value::Null);
            let (_, replay) = call(router(st.clone()), post_json(uri, Some(TOKEN), request)).await;
            assert_eq!(replay, first, "{scenario}");
            let lookup = format!("{uri}/receipts/command-ui?expected_session_incarnation={launch}");
            let (status, lookup) = call(router(st.clone()), bearer_get(&lookup)).await;
            assert_eq!(status, StatusCode::OK, "{scenario}");
            assert_eq!(lookup["data"]["receipt"], first["data"]["receipt"]);
        } else {
            assert_eq!(status, StatusCode::CONFLICT, "{scenario}: {first}");
            assert_eq!(first["error"]["details"]["receipt"]["outcome"], "unknown");
            assert!(first["data"]["command_dispatched"].is_null(), "{scenario}");
        }
        assert_eq!(
            fs::read_to_string(tmp.path().join("enter-count")).unwrap(),
            "1",
            "{scenario}"
        );
        let calls = fs::read_to_string(tmp.path().join("transport-calls")).unwrap();
        assert!(
            !calls.contains("BSpace"),
            "{scenario}: command UI must not be answered or erased"
        );
        let record = load_session_record(&st.context, id).unwrap();
        assert!(
            activity::state_for_view(&st.context, &record)
                .unwrap()
                .current_turn
                .is_none(),
            "{scenario}"
        );
    }
}
