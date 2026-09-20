use crate::support::*;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::time::{Duration, Instant};

#[cfg(unix)]
#[test]
fn agent_doctor_reports_secret_free_bounded_readiness_without_a_model_call() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let invoked = tmp.path().join("doctor.log");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt --model --effort'
  exit 0
fi
if [ "${1:-}" = "doctor" ]; then
  printf '%s\n' 'private@example.com secret-token /private/path'
  printf '%s\n' 'private-stderr-token' >&2
  printf '%s\n' doctor >> "$CLAUDE_TEST_DOCTOR_LOG"
  exit 0
fi
printf '%s\n' model-call >> "$CLAUDE_TEST_DOCTOR_LOG"
exit 91
"#,
    );
    nils_test_support::write_exe(
        &bin_dir,
        "semantic-commit",
        r#"#!/bin/sh
set -eu
case "$*" in
  "staged-context --help")
    printf '%s\n' '  --format <mode>' '  --repo <path>'
    ;;
  "commit --help")
    printf '%s\n' \
      '  --type <type>' \
      '  --scope <scope>' \
      '  --subject <subject>' \
      '  --body-bullet <text>' \
      '  --expect-head <rev>' \
      '  --repo <path>' \
      '  --summary <mode>' \
      '  --automation'
    ;;
  *) exit 91 ;;
esac
"#,
    );
    let output = run(
        &["agent", "doctor", "--format", "json"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_TEST_DOCTOR_LOG", &path_str(&invoked)),
    );

    assert_exit(&output, 0);
    let payload: Value = serde_json::from_str(stdout(&output).trim()).expect("doctor json");
    assert_eq!(payload["schema_version"], "claude-cli.agent.doctor.v1");
    assert_eq!(payload["command"], "agent doctor");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["ready"], true);
    assert_eq!(payload["result"]["dependencies"]["claude"], true);
    assert_eq!(payload["result"]["dependencies"]["git"], true);
    assert_eq!(payload["result"]["dependencies"]["semantic_commit"], true);
    assert_eq!(
        payload["result"]["dependencies"]["semantic_commit_compatible"],
        true
    );
    assert_eq!(payload["result"]["upstream_doctor"], true);
    assert_eq!(payload["result"]["commit_profile"], true);
    assert_eq!(payload["result"]["configured_commit_profile"], true);
    assert_eq!(payload["result"]["flags"]["--model"], true);
    assert_eq!(payload["result"]["flags"]["--effort"], true);
    assert_eq!(
        std::fs::read_to_string(invoked).expect("doctor log"),
        "doctor\n"
    );
    for secret in [
        "private@example.com",
        "secret-token",
        "/private/path",
        "private-stderr-token",
    ] {
        assert!(!stdout(&output).contains(secret));
        assert!(!stderr(&output).contains(secret));
    }
}

#[cfg(unix)]
#[test]
fn agent_doctor_requires_compatible_semantic_commit_surface() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt --model --effort'
  exit 0
fi
if [ "${1:-}" = "doctor" ]; then exit 0; fi
exit 91
"#,
    );
    nils_test_support::write_exe(&bin_dir, "semantic-commit", "#!/bin/sh\nexit 0\n");
    let output = run(
        &["agent", "doctor", "--format", "json"],
        &base_options(tmp.path()).with_fake_claude(&bin_dir),
    );

    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(stdout(&output).trim()).expect("doctor json");
    assert_eq!(payload["result"]["dependencies"]["semantic_commit"], true);
    assert_eq!(
        payload["result"]["dependencies"]["semantic_commit_compatible"],
        false
    );
    assert_eq!(payload["result"]["ready"], false);
}

#[cfg(unix)]
#[test]
fn agent_doctor_bounds_upstream_output_and_reports_stable_failure() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt'
  exit 0
fi
if [ "${1:-}" = "doctor" ]; then
  while :; do printf '%s\n' 'private-secret-growing-output'; done
fi
exit 91
"#,
    );
    nils_test_support::write_exe(&bin_dir, "semantic-commit", "#!/bin/sh\nexit 0\n");
    let started = Instant::now();
    let output = run(
        &["agent", "doctor", "--format", "json"],
        &base_options(tmp.path()).with_fake_claude(&bin_dir),
    );

    assert_exit(&output, 1);
    assert!(started.elapsed() < Duration::from_secs(3));
    let payload: Value = serde_json::from_str(stdout(&output).trim()).expect("doctor json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["ready"], false);
    assert_eq!(payload["result"]["upstream_doctor"], false);
    assert_eq!(
        payload["result"]["upstream_doctor_status"],
        "output-too-large"
    );
    assert!(!stdout(&output).contains("private-secret-growing-output"));
    assert!(!stderr(&output).contains("private-secret-growing-output"));
}

#[cfg(unix)]
#[test]
fn agent_doctor_reports_configured_capability_and_upstream_failures() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --json-schema --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt --model'
  exit 0
fi
if [ "${1:-}" = "doctor" ]; then exit 7; fi
exit 91
"#,
    );
    nils_test_support::write_exe(
        &bin_dir,
        "semantic-commit",
        r#"#!/bin/sh
case "$*" in
  "staged-context --help") printf '%s\n' '  --format <mode>' '  --repo <path>' ;;
  "commit --help") printf '%s\n' '--type --scope --subject --body-bullet --expect-head --repo --summary --automation' ;;
  *) exit 91 ;;
esac
"#,
    );
    let output = run(
        &["agent", "doctor", "--format", "json"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_CLI_EFFORT", "high"),
    );

    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(stdout(&output).trim()).expect("doctor json");
    assert_eq!(payload["result"]["commit_profile"], true);
    assert_eq!(payload["result"]["configured_commit_profile"], false);
    assert_eq!(payload["result"]["flags"]["--effort"], false);
    assert_eq!(payload["result"]["upstream_doctor_status"], "failed");
    assert_eq!(payload["result"]["ready"], false);
}

#[test]
fn agent_doctor_reports_launch_failed_when_claude_is_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("missing-claude");
    let output = run(
        &["agent", "doctor", "--format", "json"],
        &base_options(tmp.path()).with_env("CLAUDE_CLI_BIN", &path_str(&missing)),
    );

    assert_exit(&output, 1);
    let payload: Value = serde_json::from_str(stdout(&output).trim()).expect("doctor json");
    assert_eq!(payload["result"]["dependencies"]["claude"], false);
    assert_eq!(payload["result"]["upstream_doctor_status"], "launch-failed");
    assert_eq!(payload["result"]["ready"], false);
}
