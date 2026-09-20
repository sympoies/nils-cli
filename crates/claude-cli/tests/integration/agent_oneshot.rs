use crate::support::*;
use pretty_assertions::assert_eq;
use std::time::{Duration, Instant};

#[cfg(unix)]
#[test]
fn agent_prompt_safe_runtime_probes_capabilities_and_keeps_input_out_of_argv() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let argv_log = tmp.path().join("argv.log");
    let stdin_log = tmp.path().join("stdin.log");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt'
  exit 0
fi
: > "$CLAUDE_TEST_ARGV_LOG"
for arg in "$@"; do
  printf '%s\n' "$arg" >> "$CLAUDE_TEST_ARGV_LOG"
done
cat > "$CLAUDE_TEST_STDIN_LOG"
printf '%s\n' 'safe model result'
"#,
    );
    let prompt = "--literal; $(touch should-not-run)";
    let output = run(
        &["agent", "prompt", prompt],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_CLI_NO_SESSION_PERSISTENCE", "false")
            .with_env("CLAUDE_TEST_ARGV_LOG", &path_str(&argv_log))
            .with_env("CLAUDE_TEST_STDIN_LOG", &path_str(&stdin_log)),
    );

    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "safe model result\n");
    assert_eq!(stderr(&output), "");
    assert!(!tmp.path().join("should-not-run").exists());
    assert_eq!(
        std::fs::read_to_string(argv_log)
            .expect("argv log")
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "--print",
            "--output-format",
            "text",
            "--safe-mode",
            "--strict-mcp-config",
            "--no-session-persistence",
            "--permission-mode",
            "dontAsk",
            "--disable-slash-commands",
            "--no-chrome",
            "--tools",
            "Read,Glob,Grep",
        ]
    );
    assert_eq!(
        std::fs::read_to_string(stdin_log).expect("stdin log"),
        prompt
    );
}

#[cfg(unix)]
#[test]
fn agent_prompt_accepts_stdin_and_fails_closed_when_capability_is_missing() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --safe-mode'
  exit 0
fi
exit 99
"#,
    );

    let output = run(
        &["agent", "prompt"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_stdin_str("stdin prompt\n"),
    );

    assert_exit(&output, 69);
    assert_eq!(stdout(&output), "");
    assert!(stderr(&output).contains("missing required Claude capabilities"));
    assert!(!stderr(&output).contains("stdin prompt"));
}

#[cfg(unix)]
#[test]
fn agent_capability_probe_stops_unbounded_output_before_launch() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let launched = tmp.path().join("launched");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
if [ "${1:-}" = "--help" ]; then
  while :; do printf '%s\n' 'unbounded help output'; done
fi
: > "$CLAUDE_TEST_LAUNCHED"
exit 99
"#,
    );
    let started = Instant::now();

    let output = run(
        &["agent", "prompt", "hello"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_TEST_LAUNCHED", &path_str(&launched)),
    );

    assert_exit(&output, 69);
    assert!(stderr(&output).contains("bounded `claude --help` output"));
    assert!(!launched.exists());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[cfg(unix)]
#[test]
fn agent_advice_and_knowledge_share_safe_runtime_with_versioned_templates() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let argv_log = tmp.path().join("argv.log");
    let stdin_log = tmp.path().join("stdin.log");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools --append-system-prompt'
  exit 0
fi
: > "$CLAUDE_TEST_ARGV_LOG"
for arg in "$@"; do printf '%s\n' "$arg" >> "$CLAUDE_TEST_ARGV_LOG"; done
cat > "$CLAUDE_TEST_STDIN_LOG"
printf '%s\n' ok
"#,
    );
    let options = base_options(tmp.path())
        .with_fake_claude(&bin_dir)
        .with_env("CLAUDE_TEST_ARGV_LOG", &path_str(&argv_log))
        .with_env("CLAUDE_TEST_STDIN_LOG", &path_str(&stdin_log));

    let advice = run(&["agent", "advice", "review", "this"], &options);
    assert_exit(&advice, 0);
    let advice_argv = std::fs::read_to_string(&argv_log).expect("advice argv");
    assert!(advice_argv.contains("nils-claude-cli.agent-advice.v1"));
    assert!(!advice_argv.contains("review this"));
    assert_eq!(
        std::fs::read_to_string(&stdin_log).expect("advice stdin"),
        "review this"
    );

    let knowledge = run(&["agent", "knowledge", "borrow", "checker"], &options);
    assert_exit(&knowledge, 0);
    let knowledge_argv = std::fs::read_to_string(&argv_log).expect("knowledge argv");
    assert!(knowledge_argv.contains("nils-claude-cli.agent-knowledge.v1"));
    assert!(!knowledge_argv.contains("borrow checker"));
    assert!(knowledge_argv.lines().any(|line| line.is_empty()));
    assert_eq!(
        std::fs::read_to_string(&stdin_log).expect("knowledge stdin"),
        "borrow checker"
    );
}

#[cfg(unix)]
#[test]
fn agent_inherited_runtime_has_explicit_session_persistence_contract() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let argv_log = tmp.path().join("argv.log");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
if [ "${1:-}" = "--help" ]; then
  printf '%s\n' '--print --output-format --safe-mode --strict-mcp-config --no-session-persistence --permission-mode --disable-slash-commands --no-chrome --tools'
  exit 0
fi
: > "$CLAUDE_TEST_ARGV_LOG"
for arg in "$@"; do printf '%s\n' "$arg" >> "$CLAUDE_TEST_ARGV_LOG"; done
cat >/dev/null
"#,
    );
    let base = base_options(tmp.path())
        .with_fake_claude(&bin_dir)
        .with_env("CLAUDE_TEST_ARGV_LOG", &path_str(&argv_log));

    let default = run(
        &["agent", "prompt", "--runtime", "inherited", "hello"],
        &base,
    );
    assert_exit(&default, 0);
    let default_argv = std::fs::read_to_string(&argv_log).expect("default argv");
    assert!(
        default_argv
            .lines()
            .any(|line| line == "--no-session-persistence")
    );
    assert!(!default_argv.lines().any(|line| line == "--safe-mode"));
    assert!(
        !default_argv
            .lines()
            .any(|line| line == "--strict-mcp-config")
    );

    let persistent = run(
        &["agent", "prompt", "--runtime", "inherited", "hello"],
        &base
            .clone()
            .with_env("CLAUDE_CLI_NO_SESSION_PERSISTENCE", "false"),
    );
    assert_exit(&persistent, 0);
    let persistent_argv = std::fs::read_to_string(&argv_log).expect("persistent argv");
    assert!(
        !persistent_argv
            .lines()
            .any(|line| line == "--no-session-persistence")
    );
    assert!(!persistent_argv.lines().any(|line| line == "--safe-mode"));
}

#[cfg(unix)]
#[test]
fn agent_rejects_oversized_stdin_before_launch() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let launched = tmp.path().join("launched");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
: > "$CLAUDE_TEST_LAUNCHED"
exit 99
"#,
    );
    let oversized = "x".repeat(1024 * 1024 + 1);

    let output = run(
        &["agent", "prompt"],
        &base_options(tmp.path())
            .with_fake_claude(&bin_dir)
            .with_env("CLAUDE_TEST_LAUNCHED", &path_str(&launched))
            .with_stdin_str(&oversized),
    );

    assert_exit(&output, 65);
    assert!(stderr(&output).contains("1 MiB safety limit"));
    assert!(!launched.exists());
}
