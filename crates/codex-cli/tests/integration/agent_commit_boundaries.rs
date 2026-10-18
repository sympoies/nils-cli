use std::fs;
use std::path::PathBuf;

use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::fs as test_fs;
use nils_test_support::git as test_git;
use nils_test_support::{GlobalStateLock, bin};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::json;

struct Fixture {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    options: CmdOptions,
    log: PathBuf,
    message: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        let stub = temp.path().join("bin");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&stub).unwrap();
        test_git::git(&repo, &["init"]);
        test_git::git(&repo, &["config", "user.name", "Test User"]);
        test_git::git(&repo, &["config", "user.email", "test@example.com"]);
        test_git::git(&repo, &["config", "commit.gpgsign", "false"]);
        fs::write(repo.join("base.txt"), "base\n").unwrap();
        test_git::git(&repo, &["add", "base.txt"]);
        test_git::git(&repo, &["commit", "-m", "chore: base"]);
        fs::write(repo.join("change.txt"), "staged\n").unwrap();
        test_git::git(&repo, &["add", "change.txt"]);
        let log = temp.path().join("operations.log");
        let message = temp.path().join("message.json");
        fs::write(
            &message,
            json!({"type":"fix","scope":"agent","subject":"validate message","body_bullets":[]})
                .to_string(),
        )
        .unwrap();
        test_fs::write_executable(
            &stub.join("codex"),
            r#"#!/bin/sh
set -eu
if [ "${1:-}" = exec ] && [ "${2:-}" = --help ]; then
  printf '%s\n' '--ignore-user-config --ignore-rules --ephemeral --skip-git-repo-check --disable'
  exit 0
fi
if [ "${1:-}" = features ]; then
  printf '%s\n' 'hooks plugins remote_plugin apps memories goals multi_agent workspace_dependencies shell_tool unified_exec'
  exit 0
fi
printf 'model\n' >> "$OPERATIONS_LOG"
last=''
for arg in "$@"; do
  if [ "$last" = --output-last-message ]; then cat "$MESSAGE_FILE" > "$arg"; fi
  last="$arg"
done
case "${DRIFT:-}" in
  index) printf 'raced\n' > change.txt; "$REAL_GIT" add change.txt ;;
  head) "$REAL_GIT" -c commit.gpgsign=false commit --allow-empty -m 'chore: concurrent' >/dev/null ;;
esac
exit "${MODEL_EXIT:-0}"
"#,
        );
        test_fs::write_executable(
            &stub.join("semantic-commit"),
            r#"#!/bin/sh
set -eu
if [ "$1" = staged-context ]; then
  printf 'context\n' >> "$OPERATIONS_LOG"
  printf '%s\n' 'STAGED_CONTEXT_SENTINEL'
  exit "${CONTEXT_EXIT:-0}"
fi
printf 'commit\n' >> "$OPERATIONS_LOG"
for arg in "$@"; do printf 'arg=%s\n' "$arg" >> "$OPERATIONS_LOG"; done
if [ "${COMMIT_EXIT:-0}" = 0 ] && [ "${REAL_COMMIT:-0}" = 1 ]; then
  "$REAL_GIT" -c commit.gpgsign=false commit -m 'fix(agent): validate message' >/dev/null
fi
exit "${COMMIT_EXIT:-0}"
"#,
        );
        test_fs::write_executable(
            &stub.join("git"),
            r#"#!/bin/sh
set -eu
for arg in "$@"; do
  if [ "$arg" = push ]; then printf 'push\n' >> "$OPERATIONS_LOG"; exit "${PUSH_EXIT:-0}"; fi
done
exec "$REAL_GIT" "$@"
"#,
        );
        let real_git = nils_common::process::find_in_path("git").unwrap();
        let options = CmdOptions::default()
            .with_cwd(&repo)
            .with_path_prepend(&stub)
            .with_env("REAL_GIT", &real_git.to_string_lossy())
            .with_env("OPERATIONS_LOG", &log.to_string_lossy())
            .with_env("MESSAGE_FILE", &message.to_string_lossy())
            .with_env("CODEX_CLI_AGENT_RUNTIME", "isolated")
            .with_env("HOME", &temp.path().to_string_lossy())
            .with_env("CODEX_HOME", &temp.path().join(".codex").to_string_lossy())
            .with_env_remove("SEMANTIC_COMMIT_HEADER_WIDTH");
        Self {
            _temp: temp,
            repo,
            options,
            log,
            message,
        }
    }

    fn run(&self, args: &[&str]) -> CmdOutput {
        cmd::run_with(&bin::resolve("codex-cli"), args, &self.options)
    }

    fn operations(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

#[test]
fn isolated_commit_rejects_invalid_message_fields_before_semantic_commit() {
    let _lock = GlobalStateLock::new();
    let cases = [
        json!({"type":"fix","scope":"agent\u{0000}","subject":"valid","body_bullets":[]}),
        json!({"type":"fix","scope":"agent","subject":"invalid\u{0000}subject","body_bullets":[]}),
        json!({"type":"fix","scope":"agent","subject":"valid","body_bullets":["invalid\u{0000}bullet"]}),
        json!({"type":"fix","scope":"agent\u{001b}[31m","subject":"valid","body_bullets":[]}),
        json!({"type":"fix","scope":"agent","subject":"invalid\nsubject","body_bullets":[]}),
        json!({"type":"fix","scope":"agent","subject":"valid","body_bullets":["invalid\nbullet"]}),
        json!({"type":"fix","scope":"agent","subject":"valid","body_bullets":[],"push":true}),
        json!({"type":"invalid","scope":"agent","subject":"valid","body_bullets":[]}),
        json!({"type":"fix","scope":"invalid/scope","subject":"valid","body_bullets":[]}),
    ];
    for (index, message) in cases.iter().enumerate() {
        let fixture = Fixture::new();
        fs::write(&fixture.message, message.to_string()).unwrap();
        let head = test_git::git(&fixture.repo, &["rev-parse", "HEAD"]);
        let output = fixture.run(&["agent", "commit", "--push"]);
        assert_ne!(output.code, 0, "case {index}");
        assert!(
            !fixture.operations().contains("commit\n"),
            "case {index}: invalid fields reached semantic-commit: {}",
            output.stderr_text()
        );
        assert!(!fixture.operations().contains("push\n"));
        assert_eq!(test_git::git(&fixture.repo, &["rev-parse", "HEAD"]), head);
        assert!(
            output.stderr_text().contains("invalid"),
            "case {index}: {}",
            output.stderr_text()
        );
    }
}

#[test]
fn isolated_commit_honors_configured_header_width_before_semantic_commit() {
    let _lock = GlobalStateLock::new();
    let mut fixture = Fixture::new();
    fixture.options = fixture
        .options
        .with_env("SEMANTIC_COMMIT_HEADER_WIDTH", "20");
    let output = fixture.run(&["agent", "commit"]);
    assert_ne!(output.code, 0, "over-width message was accepted");
    assert!(
        output
            .stderr_text()
            .contains("header exceeds 20 characters"),
        "{}",
        output.stderr_text()
    );
    assert!(!fixture.operations().contains("commit\n"));
}

#[test]
fn isolated_commit_allows_messages_within_increased_header_width() {
    let _lock = GlobalStateLock::new();
    let mut fixture = Fixture::new();
    fixture.options = fixture
        .options
        .with_env("SEMANTIC_COMMIT_HEADER_WIDTH", "140");
    fs::write(
        &fixture.message,
        json!({"type":"fix","scope":null,"subject":"x".repeat(110),"body_bullets":[]}).to_string(),
    )
    .unwrap();
    let output = fixture.run(&["agent", "commit"]);
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    assert!(fixture.operations().contains("commit\n"));
}

#[test]
fn isolated_commit_detects_head_and_index_drift_without_committing_or_pushing() {
    let _lock = GlobalStateLock::new();
    for drift in ["index", "head"] {
        let mut fixture = Fixture::new();
        fixture.options = fixture.options.with_env("DRIFT", drift);
        let output = fixture.run(&["agent", "commit", "--push"]);
        assert_ne!(output.code, 0);
        assert!(
            output
                .stderr_text()
                .contains("repository changed during message generation")
        );
        assert!(!fixture.operations().contains("commit\n"));
        assert!(!fixture.operations().contains("push\n"));
    }
}

#[test]
fn isolated_commit_push_is_wrapper_owned_and_follows_successful_commit_only() {
    let _lock = GlobalStateLock::new();
    for (args, fail, pushed) in [
        (vec!["agent", "commit", "please push now"], "0", false),
        (vec!["agent", "commit", "--push"], "0", true),
        (vec!["agent", "commit", "--push"], "1", false),
    ] {
        let mut fixture = Fixture::new();
        fixture.options = fixture.options.with_env("COMMIT_EXIT", fail);
        let output = fixture.run(&args);
        assert_eq!(output.code, if fail == "0" { 0 } else { 1 });
        let log = fixture.operations();
        assert_eq!(log.contains("push\n"), pushed);
        if pushed {
            assert!(log.find("commit\n").unwrap() < log.find("push\n").unwrap());
        }
    }
}

#[test]
fn isolated_commit_auto_stage_survives_model_failure() {
    let _lock = GlobalStateLock::new();
    let mut fixture = Fixture::new();
    fs::write(fixture.repo.join("new.txt"), "unstaged\n").unwrap();
    fixture.options = fixture.options.with_env("MODEL_EXIT", "1");
    let output = fixture.run(&["agent", "commit", "--auto-stage", "--push"]);
    assert_ne!(output.code, 0);
    assert!(output.stderr_text().contains("index remains staged"));
    assert!(test_git::git(&fixture.repo, &["diff", "--cached", "--name-only"]).contains("new.txt"));
    assert!(!fixture.operations().contains("commit\n"));
    assert!(!fixture.operations().contains("push\n"));
}

#[test]
fn isolated_commit_push_failure_reports_and_preserves_the_committed_object() {
    let _lock = GlobalStateLock::new();
    let mut fixture = Fixture::new();
    fixture.options = fixture
        .options
        .with_env("REAL_COMMIT", "1")
        .with_env("PUSH_EXIT", "1");
    let old_head = test_git::git(&fixture.repo, &["rev-parse", "HEAD"]);
    let output = fixture.run(&["agent", "commit", "--push"]);
    let new_head = test_git::git(&fixture.repo, &["rev-parse", "HEAD"]);
    assert_ne!(new_head, old_head);
    assert_ne!(output.code, 0);
    assert!(
        output.stderr_text().contains(new_head.trim()),
        "{}",
        output.stderr_text()
    );
    assert!(output.stderr_text().contains("push failed"));
    assert!(
        test_git::git(&fixture.repo, &["diff", "--cached", "--name-only"])
            .trim()
            .is_empty()
    );
}

#[test]
fn isolated_commit_missing_semantic_commit_never_invokes_codex_or_raw_git_commit() {
    let _lock = GlobalStateLock::new();
    let mut fixture = Fixture::new();
    fs::remove_file(fixture._temp.path().join("bin/semantic-commit")).unwrap();
    fixture.options = fixture.options.with_env(
        "PATH",
        &format!(
            "{}:/usr/bin:/bin",
            fixture._temp.path().join("bin").display()
        ),
    );
    let head = test_git::git(&fixture.repo, &["rev-parse", "HEAD"]);
    let output = fixture.run(&["agent", "commit", "--push"]);
    assert_ne!(output.code, 0);
    assert!(
        output
            .stderr_text()
            .contains("missing dependency: semantic-commit")
    );
    assert!(fixture.operations().is_empty());
    assert_eq!(test_git::git(&fixture.repo, &["rev-parse", "HEAD"]), head);
}

#[test]
fn isolated_commit_header_width_counts_unicode_characters() {
    let _lock = GlobalStateLock::new();
    for (characters, accepted) in [(5, true), (6, false)] {
        let mut fixture = Fixture::new();
        fixture.options = fixture
            .options
            .with_env("SEMANTIC_COMMIT_HEADER_WIDTH", "10");
        fs::write(
            &fixture.message,
            json!({"type":"fix","scope":null,"subject":"é".repeat(characters),"body_bullets":[]})
                .to_string(),
        )
        .unwrap();
        let output = fixture.run(&["agent", "commit"]);
        assert_eq!(output.code == 0, accepted, "{}", output.stderr_text());
        assert_eq!(fixture.operations().contains("commit\n"), accepted);
    }
}
