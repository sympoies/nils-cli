//! Modeled launch proves the notice encoder reaches the existing initial prompt staging path.
use nils_test_support::cmd::run_resolved;
use pretty_assertions::assert_eq;
use std::fs;

#[test]
fn notice_start_stages_exact_encoded_stdin_for_both_providers() {
    let body = "  Synthetic initial task\r\n\nUnicode 本文\n";
    let expected =
        agent_session::notice::NoticeEnvelope::new("dispatch.bootstrap", "scheduler", None)
            .unwrap()
            .encode(body)
            .unwrap();
    for provider in ["claude", "codex"] {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let (tmux, log) = super::cli::fake_tmux(temp.path());
        let agent = super::cli::fake_agent(temp.path(), provider);
        let output = run_resolved(
            "agent-session",
            &[
                "--state-dir",
                state.to_str().unwrap(),
                "--host",
                "example.invalid",
                "start",
                "--agent",
                provider,
                "--cwd",
                temp.path().to_str().unwrap(),
                "--id",
                "synthetic",
                "--prompt-stdin",
                "--notice-kind",
                "dispatch.bootstrap",
                "--notice-source",
                "scheduler",
                "--tmux-bin",
                tmux.to_str().unwrap(),
                "--agent-bin",
                agent.to_str().unwrap(),
                "--paste-delay-ms",
                "0",
                "--format",
                "json",
            ],
            &super::hermetic_provider_options(temp.path())
                .with_env_remove_prefix("AGENT_SESSION_")
                .with_env("AGENT_SESSION_NOTICE_WRITERS", "1")
                .with_env("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap())
                .with_stdin_str(body),
        );
        assert_eq!(
            output.code,
            0,
            "{} {}",
            output.stdout_text(),
            output.stderr_text()
        );
        assert_eq!(
            fs::read(state.join("sessions/synthetic/prompt.md")).unwrap(),
            expected.as_bytes()
        );
        assert!(!output.stdout_text().contains(body));
        assert!(
            super::cli::tmux_calls(&log)
                .iter()
                .any(|call| call[0] == "load-buffer")
        );
    }
}
