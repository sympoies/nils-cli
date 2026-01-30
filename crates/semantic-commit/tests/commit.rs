mod common;

use std::path::Path;

fn as_str(output: &[u8]) -> String {
    String::from_utf8_lossy(output).to_string()
}

fn stage_file(repo: &Path, name: &str, contents: &str) {
    common::write_file(repo, name, contents);
    common::git(repo, &["add", name]);
}

#[test]
fn commit_outside_git_repo_errors() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let output = common::run_semantic_commit_output(
        dir.path(),
        &["commit", "--message", "chore: test"],
        &[],
        None,
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(as_str(&output.stderr).contains("error: must run inside a git work tree"));
}

#[test]
fn commit_no_staged_changes_exits_2() {
    let repo = common::init_repo();
    let output = common::run_semantic_commit_output(
        repo.path(),
        &["commit", "--message", "chore: test"],
        &[],
        None,
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(as_str(&output.stderr)
        .contains("error: no staged changes (stage files with git add first)"));
}

#[test]
fn commit_invalid_header_format_is_hard_fail() {
    let repo = common::init_repo();
    stage_file(repo.path(), "a.txt", "hello\n");

    let output = common::run_semantic_commit_output(
        repo.path(),
        &["commit", "--message", "Feat: bad"],
        &[],
        None,
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(as_str(&output.stderr).contains("error: invalid header format"));

    let head = common::git_output(repo.path(), &["rev-parse", "--verify", "HEAD"]);
    assert!(
        !head.status.success(),
        "expected no commit to have been created"
    );
}

#[test]
fn commit_body_requires_blank_line() {
    let repo = common::init_repo();
    stage_file(repo.path(), "a.txt", "hello\n");

    let message = "feat: test\n- Bad body without blank line\n";
    let output = common::run_semantic_commit_output(repo.path(), &["commit"], &[], Some(message));

    assert_eq!(output.status.code(), Some(1));
    assert!(as_str(&output.stderr)
        .contains("error: commit body must be separated from header by a blank line"));
}

#[test]
fn commit_body_line_requires_capitalized_bullet() {
    let repo = common::init_repo();
    stage_file(repo.path(), "a.txt", "hello\n");

    let message = "feat: test\n\n- bad\n";
    let output = common::run_semantic_commit_output(repo.path(), &["commit"], &[], Some(message));

    assert_eq!(output.status.code(), Some(1));
    assert!(as_str(&output.stderr)
        .contains("error: commit body line 3 must start with '- ' followed by uppercase letter"));
}

#[test]
fn commit_message_and_message_file_are_mutually_exclusive() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let output = common::run_semantic_commit_output(
        dir.path(),
        &[
            "commit",
            "--message",
            "chore: test",
            "--message-file",
            "message.txt",
        ],
        &[],
        None,
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(as_str(&output.stderr).contains("error: use only one of --message or --message-file"));
}

#[test]
fn commit_message_file_missing_errors() {
    let repo = common::init_repo();
    stage_file(repo.path(), "a.txt", "hello\n");

    let output = common::run_semantic_commit_output(
        repo.path(),
        &["commit", "--message-file", "missing.txt"],
        &[],
        None,
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(as_str(&output.stderr).contains("error: message file not found: missing.txt"));
}

#[test]
fn commit_success_stdin_falls_back_to_git_show() {
    let repo = common::init_repo();
    stage_file(repo.path(), "a.txt", "hello\n");

    let output = common::run_semantic_commit_output(
        repo.path(),
        &["commit"],
        &[
            ("CODEX_COMMANDS_PATH", "/nonexistent"),
            ("GIT_AUTHOR_DATE", "Thu, 01 Jan 1970 00:00:00 +0000"),
            ("GIT_COMMITTER_DATE", "Thu, 01 Jan 1970 00:00:00 +0000"),
        ],
        Some("feat(core): add thing\n\n- Add thing\n"),
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(as_str(&output.stderr)
        .contains("warning: git-scope not found; falling back to git show --stat"));
    let stdout = as_str(&output.stdout);
    assert!(stdout.contains("a.txt |"));
    assert!(stdout.contains("file changed") || stdout.contains("files changed"));
}

#[test]
fn commit_success_uses_git_scope_when_available() {
    let repo = common::init_repo();
    stage_file(repo.path(), "a.txt", "hello\n");

    let codex_home = common::make_codex_home();
    common::write_executable(
        codex_home.path(),
        "commands/git-scope",
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${1-}" != "commit" || "${2-}" != "HEAD" || "${3-}" != "--no-color" ]]; then
  echo "unexpected args: $*" >&2
  exit 2
fi
echo "GIT_SCOPE_OK"
"#,
    );

    let commands_dir = codex_home.path().join("commands");
    let commands_dir = commands_dir.to_str().unwrap();
    let output = common::run_semantic_commit_output(
        repo.path(),
        &["commit", "--message", "feat(core): add thing"],
        &[
            ("CODEX_COMMANDS_PATH", commands_dir),
            ("GIT_AUTHOR_DATE", "Thu, 01 Jan 1970 00:00:00 +0000"),
            ("GIT_COMMITTER_DATE", "Thu, 01 Jan 1970 00:00:00 +0000"),
        ],
        None,
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(as_str(&output.stdout).contains("GIT_SCOPE_OK"));
    assert!(!as_str(&output.stderr).contains("warning: git-scope not found"));
}
