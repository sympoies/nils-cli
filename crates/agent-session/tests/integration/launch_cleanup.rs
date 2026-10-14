use super::*;
use pretty_assertions::assert_eq;

// This fixture must reach its injected kill failure within the product's
// existing probe budget. Metadata responses use only shell builtins; the shared
// fake remains responsible for launch contamination and the failed kill.
fn cleanup_tmux(tmp: &Path) -> (PathBuf, PathBuf) {
    let (original, log) = fake_tmux(tmp);
    let bin = tmp.join("tmux-cleanup");
    let script = r#"#!/bin/sh
case "$1" in
  version) exit 0 ;;
  display-message)
    last_arg=""
    for arg in "$@"; do last_arg="$arg"; done
    case "$last_arg" in
      *'#{pane_pid}')
        printf '%s\t%s\t%s\n' '$77' '%77' "${AGENT_SESSION_FAKE_TMUX_PANE_PID:?}"
        exit 0 ;;
    esac ;;
  show-environment)
    case "$4" in
      AGENT_SESSION_ID) value="${AGENT_SESSION_FAKE_TMUX_AGENT_SESSION_ID:?}" ;;
      AGENT_SESSION_STATE_DIR) value="${AGENT_SESSION_FAKE_TMUX_STATE_DIR:?}" ;;
      AGENT_SESSION_RUNTIME_ID)
        value=""
        while IFS= read -r line; do
          case "$line" in
            *'"launch_id"'*)
              value=${line#*:}
              value=${value#*\"}
              value=${value%%\"*}
              break ;;
          esac
        done < "${AGENT_SESSION_FAKE_TMUX_RUNTIME_RECORD:?}"
        [ -n "$value" ] || exit 1 ;;
      *) exit 1 ;;
    esac
    printf '%s=%s\n' "$4" "$value"
    exit 0 ;;
esac
exec __ORIGINAL__ "$@"
"#;
    write_executable(
        &bin,
        &script.replace(
            "__ORIGINAL__",
            &shell_words::quote(&original.to_string_lossy()),
        ),
    );
    let prewarm = prewarm_fake_tmux(&bin, &log);
    assert!(prewarm.status.success(), "cleanup fixture prewarm failed");
    (bin, log)
}

pub(super) fn run_retains_state_when_malformed_launch_identity_cannot_be_stopped() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).expect("repo dir");
    let (tmux_bin, tmux_log) = cleanup_tmux(tmp.path());
    let codex_bin = fake_agent(tmp.path(), "codex");
    let mut pane_process = spawn_test_process_group();
    let pane_pid = pane_process.pid().to_string();
    let session_dir = state_dir.join("sessions/malformed-run");
    let record_path = session_dir.join("session.json");

    let state_arg = state_dir.to_string_lossy().to_string();
    let cwd_arg = cwd.to_string_lossy().to_string();
    let tmux_arg = tmux_bin.to_string_lossy().to_string();
    let codex_arg = codex_bin.to_string_lossy().to_string();
    let tmux_log_arg = tmux_log.to_string_lossy().to_string();
    let record_arg = record_path.to_string_lossy().to_string();
    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "run",
            "--agent",
            "codex",
            "--id",
            "malformed-run",
            "--cwd",
            &cwd_arg,
            "--prompt",
            "fixture prompt",
            "--tmux-bin",
            &tmux_arg,
            "--agent-bin",
            &codex_arg,
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_FAKE_TMUX_LOG", &tmux_log_arg),
            ("AGENT_SESSION_FAKE_TMUX_MALFORMED_LAUNCH_IDENTITY", "1"),
            ("AGENT_SESSION_FAKE_TMUX_FAIL", "kill-session"),
            ("AGENT_SESSION_FAKE_TMUX_PANE_PID", &pane_pid),
            ("AGENT_SESSION_FAKE_TMUX_AGENT_SESSION_ID", "malformed-run"),
            ("AGENT_SESSION_FAKE_TMUX_STATE_DIR", &state_arg),
            ("AGENT_SESSION_FAKE_TMUX_RUNTIME_RECORD", &record_arg),
        ],
    );

    assert_eq!(output.code, 1, "stdout={}", output.stdout_text());
    let error = output.stdout_json();
    // The malformed launch identity is the primary failure and stays visible.
    // Cleanup could not stop the runtime, but reporting that instead would hide
    // why the launch failed at all.
    assert_eq!(error["error"]["code"], "tmux-runtime-identity-invalid");
    assert_eq!(
        error["error"]["details"]["cleanup"],
        serde_json::json!({ "state": "pending", "reason": "termination_failed" })
    );
    assert!(
        tmux_calls(&tmux_log).iter().any(|call| matches!(
            call.first().map(String::as_str),
            Some("if-shell" | "kill-session")
        )),
        "cleanup must reach the injected tmux kill failure"
    );
    assert!(
        session_dir.exists(),
        "uncertain live runtime must stay discoverable"
    );
    let record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(record["delete_tmux_identity"]["session_id"], "$77");
    assert_eq!(record["delete_tmux_identity"]["pane_id"], "%77");
    assert_eq!(
        record["delete_tmux_identity"]["process_group_id"],
        pane_process.pid()
    );
    pane_process.stop();
}
