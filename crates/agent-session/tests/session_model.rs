use std::{fs, os::unix::fs::PermissionsExt};

use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

#[test]
fn session_model_launch_arguments_reach_list_and_board() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state = tmp.path().join("state");
    let tmux = tmp.path().join("tmux");
    fs::write(&tmux, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
    let cases = [
        (
            "codex",
            json!([
                "-m",
                "example-model",
                "-c",
                "model_reasoning_effort=\"medium\""
            ]),
            json!("example-model"),
            json!("medium"),
        ),
        (
            "claude",
            json!(["--model=opus", "--effort", "medium"]),
            json!("opus"),
            json!("medium"),
        ),
        (
            "codex",
            json!(["--oss", "--model", "local-model"]),
            json!("local-model"),
            Value::Null,
        ),
        ("claude", json!([]), Value::Null, Value::Null),
    ];
    for (index, (provider, args, _, _)) in cases.iter().enumerate() {
        let id = format!("model-test-{index}");
        let dir = state.join("sessions").join(&id);
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            dir.join("session.json"),
            serde_json::to_vec(&json!({
                "schema_version": "agent-session.session.v1", "id": id, "agent": provider,
                "mode": "interactive", "cwd": tmp.path(), "tmux_session": id,
                "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
                "agent_args": args
            }))
            .unwrap(),
        )
        .unwrap();
    }
    let options = CmdOptions::new()
        .with_cwd(tmp.path())
        .without_ambient_managed_session_env()
        .with_env_remove_many(&[
            "AGENT_SESSION_BOARD",
            "AGENT_SESSION_MACHINE",
            "AGENT_SESSION_HOST",
        ])
        .with_env("AGENT_SESSION_TMUX_BIN", &tmux.to_string_lossy());
    for command in ["list", "board"] {
        let output = run_resolved(
            "agent-session",
            &[
                "--state-dir",
                &state.to_string_lossy(),
                command,
                "--format",
                "json",
            ],
            &options,
        );
        assert_eq!(output.code, 0, "{}", output.stderr_text());
        let body = output.stdout_json();
        let rows = if command == "list" {
            &body["data"]
        } else {
            &body["data"]["board"]["records"]
        };
        for (index, (_, _, model, effort)) in cases.iter().enumerate() {
            let row = rows
                .as_array()
                .unwrap()
                .iter()
                .find(|row| {
                    row[if command == "list" {
                        "id"
                    } else {
                        "session_id"
                    }] == format!("model-test-{index}")
                })
                .unwrap();
            assert!(
                row.get("model").is_some(),
                "{command} must explicitly represent unknown model"
            );
            assert_eq!(&row["model"], model);
            assert!(row.get("reasoning_effort").is_some());
            assert_eq!(&row["reasoning_effort"], effort);
        }
    }
}

#[test]
fn session_model_projected_claude_hook_reaches_real_cli_and_preserves_identity_fences() {
    let tmp = tempfile::TempDir::new().unwrap();
    let state = tmp.path().join("state");
    let dir = state.join("sessions/hook-model-test");
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = "hook-model-runtime";
    let primary = "local:v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    fs::write(dir.join("session.json"), serde_json::to_vec(&json!({
        "schema_version": "agent-session.session.v1", "id": "hook-model-test", "agent": "claude", "mode": "interactive", "cwd": tmp.path(), "tmux_session": "hook-model-test",
        "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
        "runtime": {"kind": "tmux", "tmux_session": "hook-model-test", "generation": 1, "started_at": "2030-01-01T00:00:00Z", "launch_id": runtime}
    })).unwrap()).unwrap();
    let options = CmdOptions::new().with_cwd(tmp.path()).without_ambient_managed_session_env()
        .with_env("AGENT_SESSION_ID", "hook-model-test").with_env("AGENT_SESSION_RUNTIME_ID", runtime)
        .with_stdin_bytes(&serde_json::to_vec(&json!({"hook_event_name": "SessionStart", "session_id": primary, "model": "resolved-model", "reasoning_effort": "high"})).unwrap());
    let output = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            &state.to_string_lossy(),
            "activity",
            "hook",
            "--agent",
            "claude",
        ],
        &options,
    );
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    for (stale_runtime, session_id) in [
        ("stale-runtime", primary),
        (
            runtime,
            "local:v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
    ] {
        let stale = options.clone().with_env("AGENT_SESSION_RUNTIME_ID", stale_runtime).with_stdin_bytes(&serde_json::to_vec(&json!({"hook_event_name": "SessionStart", "session_id": session_id, "model": "foreign-model"})).unwrap());
        let ignored = run_resolved(
            "agent-session",
            &[
                "--state-dir",
                &state.to_string_lossy(),
                "activity",
                "hook",
                "--agent",
                "claude",
            ],
            &stale,
        );
        assert_eq!(ignored.code, 0, "{}", ignored.stderr_text());
    }
    let record: Value =
        serde_json::from_slice(&fs::read(dir.join("session.json")).unwrap()).unwrap();
    assert_eq!(record["model_settings"]["model"], "resolved-model");
    assert_eq!(record["model_settings"]["reasoning_effort"], "high");
}
