//! Claude's stopped-runtime fallback applies only when acknowledgement is unknown.
use super::*;
use pretty_assertions::assert_eq;

fn stopping_tmux(dir: &Path, id: &str, stop_after_submit: bool) -> PathBuf {
    let bin = guard_tmux(dir, id);
    let mut script = fs::read_to_string(&bin).unwrap().replace(
        "if args[0] == 'capture-pane':",
        "if args[0] == 'has-session':\n    if (root / 'stopped').exists(): print(\"can't find session: $77\", file=sys.stderr); sys.exit(1)\n    sys.exit(0)\nif args[0] == 'capture-pane':",
    );
    if stop_after_submit {
        fs::write(dir.join("stop-after-submit"), "").unwrap();
        script = script.replace(
            "(root / 'submitted').write_text((root / 'composer').read_text())",
            "(root / 'submitted').write_text((root / 'composer').read_text())\n    if (root / 'stop-after-submit').exists(): (root / 'stopped').touch()",
        );
    }
    fs::write(&bin, script).unwrap();
    bin
}

async fn submit_without_ack(id: &str) -> (StatusCode, Value) {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_claude_prompt_session(tmp.path(), id);
    let st = state(tmp.path(), Some(TOKEN), stopping_tmux(tmp.path(), id, true));
    let mut record = load_session_record(&st.context, id).unwrap();
    record.extra.insert(
        "delete_tmux_identity".into(),
        json!({
            "launch_id": launch,
            "session_id": "$77",
            "pane_id": "%77",
            "pane_pid": 77
        }),
    );
    crate::write_session_record(&st.context, &record).unwrap();
    activity::activate_runtime(&st.context, &record).unwrap();
    let (status, body) = response_parts(
        submit_claude_structured_prompt(
            &st,
            &record,
            &launch,
            Some(&launch),
            "inspect the completed session",
            Duration::from_millis(30),
        )
        .await,
    )
    .await;
    (status, body)
}

#[tokio::test]
async fn unknown_claude_ack_with_stopped_runtime_is_submitted() {
    let (status, body) = submit_without_ack("claude-stopped-unknown").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["submitted"], true);
}

#[tokio::test]
async fn running_claude_prompt_without_ack_stays_unknown() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "claude-stopped-running";
    let launch = seed_claude_prompt_session(tmp.path(), id);
    let st = state(
        tmp.path(),
        Some(TOKEN),
        stopping_tmux(tmp.path(), id, false),
    );
    let record = load_session_record(&st.context, id).unwrap();
    activity::activate_runtime(&st.context, &record).unwrap();
    let (status, body) = response_parts(
        submit_claude_structured_prompt(
            &st,
            &record,
            &launch,
            Some(&launch),
            "inspect the queued session",
            Duration::from_millis(30),
        )
        .await,
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body={body}");
    assert_eq!(body["error"]["code"], "structured-prompt-outcome-unknown");
}
