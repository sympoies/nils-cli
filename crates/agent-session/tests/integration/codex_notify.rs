use std::fs;
use std::path::Path;

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use super::cli::{data, fake_agent, fake_tmux, run};

fn run_with_stdin(dir: &Path, args: &[&str], envs: &[(&str, &str)], stdin: &str) -> CmdOutput {
    let options = CmdOptions::new()
        .with_cwd(dir)
        .with_envs(envs)
        .with_stdin_str(stdin);
    run_resolved("agent-session", args, &options)
}

#[test]
fn codex_notify_with_oversized_content_still_records_completion() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).expect("repo dir");
    let (tmux_bin, tmux_log) = fake_tmux(tmp.path());
    let codex_bin = fake_agent(tmp.path(), "codex");
    let state_arg = state_dir.to_string_lossy().to_string();
    let cwd_arg = cwd.to_string_lossy().to_string();
    let tmux_arg = tmux_bin.to_string_lossy().to_string();
    let codex_arg = codex_bin.to_string_lossy().to_string();
    let tmux_log_arg = tmux_log.to_string_lossy().to_string();
    let start = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "start",
            "--agent",
            "codex",
            "--cwd",
            &cwd_arg,
            "--tmux-bin",
            &tmux_arg,
            "--agent-bin",
            &codex_arg,
            "--paste-delay-ms",
            "0",
            "--format",
            "json",
        ],
        &[("AGENT_SESSION_FAKE_TMUX_LOG", tmux_log_arg.as_str())],
    );
    assert_eq!(start.code, 0, "stderr={}", start.stderr_text());
    let id = data(&start.stdout_json())["id"]
        .as_str()
        .expect("id")
        .to_string();
    let record: Value = serde_json::from_str(
        &fs::read_to_string(state_dir.join("sessions").join(&id).join("session.json"))
            .expect("record"),
    )
    .expect("record json");
    let runtime_id = record["runtime"]["launch_id"]
        .as_str()
        .expect("runtime id")
        .to_string();
    let hook_env = [
        ("AGENT_SESSION_ID", id.as_str()),
        ("AGENT_SESSION_RUNTIME_ID", runtime_id.as_str()),
        ("AGENT_SESSION_STATE_DIR", state_arg.as_str()),
    ];
    let thread_id = "oversized-notify-thread";
    let secret = "long-lived-session-content-must-not-persist";
    // Above the 64 KiB metadata limit, below the per-argument OS limit.
    let large = secret.repeat(90 * 1024 / secret.len());
    let status = || {
        let output = run(
            tmp.path(),
            &[
                "--state-dir",
                &state_arg,
                "activity",
                "status",
                &id,
                "--format",
                "json",
            ],
            &[],
        );
        data(&output.stdout_json())["turn_state"].clone()
    };

    for (turn_id, content_field) in [
        ("turn-large-input", "input-messages"),
        ("turn-large-reply", "last-assistant-message"),
    ] {
        let prompt = json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": thread_id,
            "turn_id": turn_id,
            "prompt": "next"
        })
        .to_string();
        let hook = run_with_stdin(
            tmp.path(),
            &["activity", "hook", "--agent", "codex"],
            &hook_env,
            &prompt,
        );
        assert_eq!(hook.code, 0);
        assert_eq!(status()["phase"], "working");

        let mut notification = json!({
            "type": "agent-turn-complete",
            "thread-id": thread_id,
            "turn-id": turn_id,
            "input-messages": ["next"],
            "last-assistant-message": "done"
        });
        notification[content_field] = if content_field == "input-messages" {
            json!([large])
        } else {
            json!(large)
        };
        let payload = notification.to_string();
        assert!(payload.len() > 64 * 1024);
        let notify = run(
            tmp.path(),
            &["activity", "notify", "--agent", "codex", &payload],
            &hook_env,
        );
        assert_eq!(notify.code, 0, "stderr={}", notify.stderr_text());
        let state = status();
        assert_eq!(state["phase"], "waiting", "{content_field}: {state}");
        assert_eq!(state["last_turn"]["outcome"], "completed");
    }

    let session_dir = state_dir.join("sessions").join(&id);
    for file in [
        "activity.json",
        "activity.journal.jsonl",
        "activity.diagnostic.json",
    ] {
        let contents = fs::read_to_string(session_dir.join(file)).unwrap_or_default();
        assert!(!contents.contains(secret), "{file} retained content");
        assert!(
            !contents.contains("provider-notification-too-large"),
            "{file}"
        );
    }
}
