use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;

#[test]
fn move_requests_fail_closed_until_rollback_is_available() {
    let fixture = tempfile::tempdir().unwrap();
    let state = fixture.path().join("state");
    for mode in [None, Some("--check-resume"), Some("--via-console")] {
        let mut args = vec![
            "--state-dir",
            state.to_str().unwrap(),
            "start",
            "--agent",
            "codex",
            "--account",
            "fixture-account",
            "--resume",
            "00000000-0000-4000-8000-000000000001",
            "--move-id",
            "00000000-0000-4000-8000-000000000002",
            "--format",
            "json",
        ];
        args.extend(mode);
        let output = run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_"),
        );
        let response = output.stdout_json();
        assert_ne!(output.code, 0);
        assert_eq!(response["error"]["code"], "resume-move-unavailable");
        assert_eq!(response["error"]["details"]["session_created"], false);
        assert!(!state.exists());
    }
}
