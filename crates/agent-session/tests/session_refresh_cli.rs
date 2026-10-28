use std::{fs, os::unix::fs::PermissionsExt};

use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn refresh_command_reports_the_changed_fields_as_a_json_envelope() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state = tmp.path().join("state");
    let provider = tmp.path().join("provider");
    let dir = state.join("sessions/refresh-cli");
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        dir.join("session.json"),
        serde_json::to_vec(&json!({
            "schema_version": "agent-session.session.v1", "id": "refresh-cli", "agent": "claude",
            "mode": "interactive", "cwd": "/workspace/project", "tmux_session": "refresh-cli",
            "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
            "agent_args": ["--model", "old-model"],
            "model_settings": {"model": "old-model", "reasoning_effort": null, "runtime_id": "refresh-runtime"},
            "provider_resume": {
                "provider": "claude", "session_id": "provider-session",
                "captured_at": "2030-01-01T00:00:00Z", "capture_method": "fixture", "resume_args": []
            },
            "runtime": {
                "kind": "tmux", "tmux_session": "refresh-cli", "generation": 1,
                "started_at": "2030-01-01T00:00:00Z", "launch_id": "refresh-runtime",
                "agent_profile_provider_config_dir": provider,
            },
        }))
        .unwrap(),
    )
    .unwrap();
    let projects = provider.join("projects/project");
    fs::create_dir_all(&projects).unwrap();
    let row = json!({
        "type": "assistant", "sessionId": "provider-session", "cwd": "/workspace/project",
        "timestamp": "2030-01-01T00:00:00Z", "message": {"model": "claude-opus-5-5"},
    });
    fs::write(projects.join("provider-session.jsonl"), format!("{row}\n")).unwrap();
    let options = CmdOptions::new()
        .with_cwd(tmp.path())
        .without_ambient_managed_session_env();
    let refresh = || {
        run_resolved(
            "agent-session",
            &[
                "--state-dir",
                &state.to_string_lossy(),
                "refresh",
                "refresh-cli",
                "--format",
                "json",
            ],
            &options,
        )
    };

    let first = refresh();
    assert_eq!(first.code, 0, "{}", first.stderr_text());
    let body = first.stdout_json();
    assert_eq!(body["data"]["changed"], json!(["model"]));
    assert_eq!(body["data"]["session"]["model"], "claude-opus-5-5");

    let second = refresh();
    assert_eq!(second.code, 0, "{}", second.stderr_text());
    assert_eq!(second.stdout_json()["data"]["changed"], json!([]));
}
