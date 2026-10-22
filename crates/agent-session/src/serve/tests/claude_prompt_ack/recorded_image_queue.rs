// Recorded Claude Code 2.1.308 image-path rewrite and enqueue ordering.
use super::*;
use pretty_assertions::assert_eq;
use std::io::Write;

const RECORDED: &str = include_str!("../../../../tests/fixtures/claude-busy-image-2.1.308.jsonl");

async fn recorded_receipt(composer: &str, rows: &str, stale: bool) -> (StatusCode, Value) {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "recorded-image";
    let launch = seed_claude_prompt_session(tmp.path(), id);
    let root = tmp.path().join("provider");
    let project = root.join("projects/project");
    fs::create_dir_all(&project).unwrap();
    let transcript = project.join("provider-fixture.jsonl");
    let mut baseline = "{\"type\":\"user\",\"sessionId\":\"provider-fixture\",\"cwd\":\"/repo\"}\n".to_string();
    if stale {
        baseline.push_str(rows);
        baseline.push('\n');
    }
    fs::write(&transcript, baseline).unwrap();
    let record_path = tmp.path().join(format!("sessions/{id}/session.json"));
    let mut persisted: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    persisted["runtime"]["agent_profile_provider_config_dir"] = json!(root);
    persisted["provider_resume"] = json!({"provider":"claude", "session_id":"provider-fixture", "capture_method":"test", "captured_at":"2000-01-01T00:00:00Z", "resume_args":[]});
    fs::write(record_path, serde_json::to_vec(&persisted).unwrap()).unwrap();
    let image = tmp.path().join("synthetic.png");
    fs::write(&image, b"synthetic image").unwrap();
    let text = format!("\"{}\"", image.display());
    let (tmux, entered) = empty_composer_tmux_marking_enter(tmp.path());
    let pasted = tmp.path().join("pasted");
    let script = fs::read_to_string(&tmux).unwrap().replace(
        "  capture-pane)",
        &format!("  paste-buffer) touch '{}'; exit 0 ;;\n  capture-pane) if [ -f '{}' ] && [ ! -f '{}' ]; then printf '────────────────────\\n❯ {composer}\\n────────────────────\\n'; exit 0; fi;\n",
            pasted.display(), pasted.display(), entered.display()),
    );
    fs::write(&tmux, script).unwrap();
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let record = load_session_record(&st.context, id).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    let rows = rows.to_string();
    let writer = tokio::spawn(async move {
        while !entered.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // The enqueue arrives immediately. The attachment has the same provider
        // timestamp but is written only when the running tool finishes later.
        if !stale {
            writeln!(fs::OpenOptions::new().append(true).open(transcript).unwrap(), "{rows}").unwrap();
        }
    });
    let request = || post_json(&format!("/sessions/{id}/prompt/v2"), Some(TOKEN),
        json!({"text":text,"expected_session_incarnation":launch,"request_id":"recorded-image-attempt"}));
    let (status, body) = call(router(st.clone()), request()).await;
    writer.await.unwrap();
    let (replay_status, replay) = call(router(st), request()).await;
    assert_eq!(replay_status, status);
    assert_eq!(replay, body);
    (status, body)
}

#[tokio::test]
async fn recorded_image_enqueue_resolves_receipt_before_attachment_is_written() {
    let (status, body) = recorded_receipt("[Image #2]", RECORDED.lines().next().unwrap(), false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["receipt"]["outcome"], "submitted");
    assert_eq!(body["data"]["queued"], true);
}

#[tokio::test]
async fn recorded_image_attachment_matches_the_observed_composer_label() {
    let (status, body) = recorded_receipt("[Image #2]", RECORDED.lines().nth(1).unwrap(), false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["queued"], true);
}

#[tokio::test]
async fn recorded_image_label_without_composer_observation_stays_unknown() {
    let (status, body) = recorded_receipt("", RECORDED.lines().next().unwrap(), false).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["error"]["details"]["receipt"]["outcome"], "unknown");
}

#[tokio::test]
async fn recorded_image_label_from_another_submission_stays_unknown() {
    let (status, body) = recorded_receipt("[Image #3]", RECORDED.lines().next().unwrap(), false).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
}

#[tokio::test]
async fn recorded_image_queue_before_submission_stays_unknown() {
    let (status, body) = recorded_receipt("[Image #2]", RECORDED, true).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
}

#[tokio::test]
async fn recorded_image_queue_from_another_session_stays_unknown() {
    let foreign = RECORDED.replace("provider-fixture", "foreign-provider");
    let (status, body) = recorded_receipt("[Image #2]", &foreign, false).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
}
