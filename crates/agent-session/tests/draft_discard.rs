//! Saved draft discard is independent of a running terminal or incarnation.
use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;

#[test]
fn discard_cli_removes_record_and_returns_typed_errors() {
    let root = tempfile::tempdir().unwrap();
    let session = root.path().join("sessions/example");
    let saved = session.join("drafts/saved");
    fs::create_dir_all(&saved).unwrap();
    let id = "00000000-0000-4000-8000-000000000001";
    fs::write(
        session.join("session.json"),
        serde_json::to_vec(&json!({
            "schema_version":"agent-session.session.v1", "id":"example", "agent":"codex",
            "mode":"interactive", "title":"Example", "cwd":root.path(), "tmux_session":"example",
            "prompt_file":session.join("prompt.txt"), "log_file":session.join("session.log"),
            "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();
    let draft = json!({"schema_version":"agent-session.saved-draft.v1", "event":"stale_draft_stashed",
        "draft_id":id, "timestamp":"2030-01-01T00:00:00Z", "session_incarnation":"previous-runtime",
        "text":"user draft", "cleared":true});
    fs::write(
        saved.join(format!("{id}.json")),
        serde_json::to_vec(&draft).unwrap(),
    )
    .unwrap();
    fs::write(
        session.join("drafts/latest.json"),
        serde_json::to_vec(&json!({"draft_id":id})).unwrap(),
    )
    .unwrap();
    let options = CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_");
    let invoke = |id| {
        run_resolved(
            "agent-session",
            &[
                "--state-dir",
                root.path().to_str().unwrap(),
                "--host",
                "fixture",
                "drafts",
                "discard",
                "example",
                id,
                "--format",
                "json",
            ],
            &options,
        )
    };
    let output = invoke(id);
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    let body = output.stdout_json();
    assert_eq!(
        body["schema_version"],
        "cli.agent-session.drafts.discard.v1"
    );
    assert_eq!(body["data"]["discarded"], id);
    assert_eq!(body["data"]["remaining"], 0);
    assert_eq!(body["data"]["stale_draft"], Value::Null);
    assert!(!saved.join(format!("{id}.json")).exists());
    assert!(!session.join("drafts/latest.json").exists());
    assert_eq!(invoke(id).stdout_json()["error"]["code"], "draft-not-found");
    assert_eq!(
        invoke("invalid").stdout_json()["error"]["code"],
        "invalid-draft-id"
    );
}
