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
fn staged_context_outside_git_repo_errors() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let output = common::run_semantic_commit_output(dir.path(), &["staged-context"], &[], None);

    assert_eq!(output.status.code(), Some(1));
    assert!(as_str(&output.stderr).contains("error: must run inside a git work tree"));
}

#[test]
fn staged_context_no_staged_changes_exits_2() {
    let repo = common::init_repo();
    let output = common::run_semantic_commit_output(repo.path(), &["staged-context"], &[], None);

    assert_eq!(output.status.code(), Some(2));
    assert!(as_str(&output.stderr)
        .contains("error: no staged changes (stage files with git add first)"));
}

#[test]
fn staged_context_fallback_prints_diff() {
    let repo = common::init_repo();
    stage_file(repo.path(), "a.txt", "hello\n");

    let output = common::run_semantic_commit_output(
        repo.path(),
        &["staged-context"],
        &[("CODEX_COMMANDS_PATH", "/nonexistent")],
        None,
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(as_str(&output.stderr).contains("warning: printing fallback staged diff only"));
    assert!(as_str(&output.stdout).contains("diff --git a/a.txt b/a.txt"));
}

#[test]
fn staged_context_prefers_git_commit_context_json_when_available() {
    let repo = common::init_repo();
    stage_file(repo.path(), "a.txt", "hello\n");

    let codex_home = common::make_codex_home();
    common::write_executable(
        codex_home.path(),
        "commands/git-commit-context-json",
        r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${1-}" != "--stdout" || "${2-}" != "--bundle" ]]; then
  echo "unexpected args: $*" >&2
  exit 2
fi
echo "BUNDLE_OK"
"#,
    );

    let commands_dir = codex_home.path().join("commands");
    let commands_dir = commands_dir.to_str().unwrap();
    let output = common::run_semantic_commit_output(
        repo.path(),
        &["staged-context"],
        &[("CODEX_COMMANDS_PATH", commands_dir)],
        None,
    );

    assert_eq!(output.status.code(), Some(0));
    assert!(as_str(&output.stdout).contains("BUNDLE_OK"));
    assert!(as_str(&output.stderr).is_empty());
}

#[test]
fn staged_context_tool_failure_falls_back_to_diff() {
    let repo = common::init_repo();
    stage_file(repo.path(), "a.txt", "hello\n");

    let codex_home = common::make_codex_home();
    common::write_executable(
        codex_home.path(),
        "commands/git-commit-context-json",
        r#"#!/usr/bin/env bash
set -euo pipefail
echo "tool failed" >&2
exit 1
"#,
    );

    let commands_dir = codex_home.path().join("commands");
    let commands_dir = commands_dir.to_str().unwrap();
    let output = common::run_semantic_commit_output(
        repo.path(),
        &["staged-context"],
        &[("CODEX_COMMANDS_PATH", commands_dir)],
        None,
    );

    assert_eq!(output.status.code(), Some(0));
    let stderr = as_str(&output.stderr);
    assert!(stderr.contains("warning: git-commit-context-json failed; falling back"));
    assert!(stderr.contains("warning: printing fallback staged diff only"));
    assert!(as_str(&output.stdout).contains("diff --git a/a.txt b/a.txt"));
}
