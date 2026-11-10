//! Guard admission for a stopped turn whose provider completion never arrives.
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn missing_completion_refuses_with_actionable_reason_before_paste() {
    let fixture = Fixture::new("claude");
    let state_dir = fixture.root().join("state");
    let options = CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_");
    for kind in ["turn_started", "progress", "stop_observed"] {
        let mut event = json!({
            "schema_version": "agent-session.turn-event.v1", "event_id": kind,
            "runtime_id": "runtime-one", "provider": "claude",
            "provider_session_id": "provider-one", "provider_turn_id": "stopped-turn",
            "kind": kind, "source_kind": "provider_hook", "confidence": "observed"
        });
        if kind == "stop_observed" {
            event["completion_candidate_reason"] = json!("background_tasks_present");
        }
        let output = run_resolved(
            "agent-session",
            &[
                "--state-dir",
                state_dir.to_str().unwrap(),
                "activity",
                "event",
                "guarded",
                "--stdin",
                "--format",
                "json",
            ],
            &options.clone().with_stdin_str(&event.to_string()),
        );
        assert_eq!(output.code, 0, "{}", output.stdout_text());
    }
    let dir = state_dir.join("sessions/guarded");
    let journal_path = dir.join("activity.journal.jsonl");
    let journal = fs::read(&journal_path).unwrap();
    let path = dir.join("activity.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["state"]["phase"], "working");
    assert_eq!(
        document["state"]["diagnostic"]["reason"],
        "completion_evidence_pending"
    );
    // Cross the quiet window without sleeping; no completion is supplied.
    document["state"]["current_turn"]["started_at"] = json!("2000-01-01T00:00:00Z");
    document["state"]["current_turn"]["last_progress_at"] = json!("2000-01-01T00:01:00Z");
    document["state"]["semantic_event"]["observed_at"] = json!("2000-01-01T00:02:00Z");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let before = fs::read(&path).unwrap();
    let status = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            state_dir.to_str().unwrap(),
            "activity",
            "status",
            "guarded",
            "--format",
            "json",
        ],
        &options,
    );
    assert_eq!(status.code, 0, "{}", status.stderr_text());
    let state = &status.stdout_json()["data"]["turn_state"];
    assert_eq!(state["phase"], "unknown");
    assert_eq!(state["source"]["confidence"], "inferred");
    assert_eq!(state["diagnostic"]["reason"], "completion_evidence_missing");
    assert_eq!(
        state["diagnostic"]["completion_candidate_reason"],
        "background_tasks_present"
    );

    let output = fixture.send(&["--require-idle", "--require-empty-composer"]);
    let refusal = fixture.refused(&output, "send-completion-evidence-missing");
    assert_eq!(refusal["error"]["details"]["guard"], "idle");
    assert_eq!(refusal["error"]["details"]["stage"], "admission");
    let message = refusal["error"]["message"].as_str().unwrap();
    assert!(message.contains("activity status"));
    assert!(message.contains("provider completion"));
    assert!(message.contains("--require-empty-composer"));
    assert!(message.contains("without --require-idle"));
    fixture.no_delivery();
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(&journal_path).unwrap(), journal);
}
