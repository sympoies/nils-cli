mod common;

use std::fs;
use std::path::Path;

use common::GitCliHarness;
use nils_test_support::cmd::run_with;
use nils_test_support::git::{git, git_with_env};
use nils_test_support::StubBinDir;
use pretty_assertions::assert_eq;
use serde_json::Value;
use tempfile::TempDir;

const GIT_SCOPE_STUB: &str = r#"#!/bin/bash
set -euo pipefail
cat <<'EOF'
.
├── hello.txt
├── package-lock.json
└── yarn.lock
EOF
"#;

const FILE_STUB: &str = r#"#!/usr/bin/env bash
set -euo pipefail

data="$(cat || true)"

if [[ "$data" == *"GIT_CLI_BINARY_MARKER"* ]]; then
  printf "%s\n" "application/octet-stream; charset=binary"
else
  printf "%s\n" "text/plain; charset=us-ascii"
fi
"#;

fn write_scope_stubs() -> StubBinDir {
    let stubs = StubBinDir::new();
    stubs.write_exe("git-scope", GIT_SCOPE_STUB);
    stubs.write_exe("file", FILE_STUB);
    stubs
}

fn git_init_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.name", "Fixture Bot"]);
    git(dir, &["config", "user.email", "fixture@example.invalid"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
    git(dir, &["config", "advice.detachedHead", "false"]);
}

fn git_commit_all(dir: &Path, msg: &str, date: &str) {
    git(dir, &["add", "-A"]);
    let envs = [
        ("GIT_AUTHOR_NAME", "Fixture Bot"),
        ("GIT_AUTHOR_EMAIL", "fixture@example.invalid"),
        ("GIT_COMMITTER_NAME", "Fixture Bot"),
        ("GIT_COMMITTER_EMAIL", "fixture@example.invalid"),
        ("GIT_AUTHOR_DATE", date),
        ("GIT_COMMITTER_DATE", date),
    ];
    git_with_env(
        dir,
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", msg],
        &envs,
    );
}

fn assert_exit_code(id: &str, output: &nils_test_support::cmd::CmdOutput, expected: i32) {
    assert_eq!(output.code, expected, "exit code mismatch for {id}");
}

fn assert_contains(id: &str, stream: &str, haystack: &str, needle: &str) {
    assert!(
        haystack.contains(needle),
        "expected {id} {stream} to contain {needle:?}\n---\n{haystack}"
    );
}

fn assert_stdout_contains_all(id: &str, stdout: &str, needles: &[&str]) {
    for needle in needles {
        assert_contains(id, "stdout", stdout, needle);
    }
}

fn assert_stderr_contains_all(id: &str, stderr: &str, needles: &[&str]) {
    for needle in needles {
        assert_contains(id, "stderr", stderr, needle);
    }
}

#[test]
fn commit_context_fixture_f030() {
    let repo = TempDir::new().expect("tempdir");
    git_init_repo(repo.path());

    fs::write(repo.path().join("hello.txt"), "base\n").expect("write hello");
    git_commit_all(repo.path(), "init", "2000-01-01T00:00:00+0000");

    fs::write(repo.path().join("hello.txt"), "GIT_CLI_BINARY_MARKER\n").expect("write hello");
    fs::write(repo.path().join("yarn.lock"), "yarn-lock\n").expect("write yarn");
    fs::write(
        repo.path().join("package-lock.json"),
        "{\"name\":\"pkg\"}\n",
    )
    .expect("write pkg");
    git(
        repo.path(),
        &["add", "hello.txt", "yarn.lock", "package-lock.json"],
    );

    let harness = GitCliHarness::new();
    let stubs = write_scope_stubs();
    let options = harness
        .cmd_options(repo.path())
        .with_path_prepend(stubs.path())
        .with_env("GIT_CLI_FIXTURE_FILE_MODE", "stub");

    let output = run_with(
        &harness.git_cli_bin(),
        &[
            "commit",
            "context",
            "--both",
            "--no-color",
            "--include",
            "yarn.lock",
        ],
        &options,
    );

    assert_exit_code("F030", &output, 0);
    let stdout = output.stdout_text();
    let stderr = output.stderr_text();
    assert!(
        stderr.trim().is_empty(),
        "expected F030 stderr to be empty, got:\n{stderr}"
    );
    assert!(
        stdout.starts_with("# Commit Context\n"),
        "expected F030 stdout to start with commit context header\n---\n{stdout}"
    );
    assert_stdout_contains_all(
        "F030",
        &stdout,
        &[
            "## 📂 Scope and file tree:",
            "## 📄 Git staged diff:",
            "## 📚 Staged file contents (index version):",
            ".\n├── hello.txt\n├── package-lock.json\n└── yarn.lock",
            "diff --git a/hello.txt b/hello.txt",
            "+GIT_CLI_BINARY_MARKER",
            "### hello.txt (M)",
            "[Binary file content hidden]",
            "Type: application/octet-stream; charset=binary",
            "### yarn.lock (A)",
            "yarn-lock\n",
            "### package-lock.json (A)",
            "[Lockfile content hidden]",
            "Tip: use --include package-lock.json",
        ],
    );
}

#[test]
fn commit_context_fixture_f031_missing_clipboard() {
    let repo = TempDir::new().expect("tempdir");
    git_init_repo(repo.path());

    fs::write(repo.path().join("hello.txt"), "base\n").expect("write hello");
    git_commit_all(repo.path(), "init", "2000-01-01T00:00:00+0000");

    fs::write(repo.path().join("hello.txt"), "base\nchange\n").expect("write hello");
    git(repo.path(), &["add", "hello.txt"]);

    let harness = GitCliHarness::new();
    let stubs = write_scope_stubs();
    let options = harness
        .cmd_options(repo.path())
        .with_path_prepend(stubs.path())
        .with_env("GIT_CLI_FIXTURE_CLIPBOARD_MODE", "missing");

    let output = run_with(&harness.git_cli_bin(), &["commit", "context"], &options);

    assert_exit_code("F031", &output, 0);
    let stdout = output.stdout_text();
    let stderr = output.stderr_text();
    assert_stderr_contains_all(
        "F031",
        &stderr,
        &["⚠️  No clipboard tool found (requires pbcopy, xclip, or xsel)"],
    );
    assert_stdout_contains_all(
        "F031",
        &stdout,
        &[
            "✅ Commit context copied to clipboard with:",
            "• Diff",
            "• Scope summary (via git-scope staged)",
            "• Staged file contents (index version)",
        ],
    );
}

#[test]
fn commit_context_fixture_f032_missing_file() {
    let repo = TempDir::new().expect("tempdir");
    git_init_repo(repo.path());

    fs::write(repo.path().join("hello.txt"), "base\n").expect("write hello");
    git_commit_all(repo.path(), "init", "2000-01-01T00:00:00+0000");

    fs::write(repo.path().join("bin.dat"), b"\x00\x01\x02binary\x00data\n").expect("write bin");
    git(repo.path(), &["add", "bin.dat"]);

    let harness = GitCliHarness::new();
    let stubs = write_scope_stubs();
    let options = harness
        .cmd_options(repo.path())
        .with_path_prepend(stubs.path())
        .with_env("GIT_CLI_FIXTURE_FILE_MODE", "missing");

    let output = run_with(
        &harness.git_cli_bin(),
        &["commit", "context", "--stdout"],
        &options,
    );

    assert_exit_code("F032", &output, 0);
    let stdout = output.stdout_text();
    let stderr = output.stderr_text();
    assert!(
        stderr.trim().is_empty(),
        "expected F032 stderr to be empty, got:\n{stderr}"
    );
    assert_stdout_contains_all(
        "F032",
        &stdout,
        &[
            "# Commit Context",
            "### bin.dat (A)",
            "[Binary file content hidden]",
        ],
    );
}

#[test]
fn commit_context_json_fixture_f033() {
    let root = TempDir::new().expect("tempdir");
    let repo_path = root.path().join("f033-commit-context-json");
    fs::create_dir_all(&repo_path).expect("create repo dir");
    git_init_repo(&repo_path);

    fs::write(repo_path.join("hello.txt"), "base\n").expect("write hello");
    git_commit_all(&repo_path, "init", "2000-01-01T00:00:00+0000");

    fs::write(repo_path.join("hello.txt"), "base\nchange\n").expect("write hello");
    git(&repo_path, &["add", "hello.txt"]);

    let harness = GitCliHarness::new();
    let options = harness
        .cmd_options(&repo_path)
        .with_env("GIT_CLI_FIXTURE_DATE_MODE", "fixed");

    let output = run_with(
        &harness.git_cli_bin(),
        &[
            "commit",
            "context-json",
            "--stdout",
            "--pretty",
            "--bundle",
            "--out-dir",
            "./out/commit-context",
        ],
        &options,
    );

    assert_exit_code("F033", &output, 0);
    let stdout = output.stdout_text();
    let stderr = output.stderr_text();
    assert!(
        stderr.trim().is_empty(),
        "expected F033 stderr to be empty, got:\n{stderr}"
    );
    assert_stdout_contains_all(
        "F033",
        &stdout,
        &[
            "===== commit-context.json =====",
            "===== staged.patch =====",
        ],
    );

    let json_start = stdout
        .strip_prefix("===== commit-context.json =====\n")
        .expect("bundle json header");
    let (json_body, patch_body) = json_start
        .split_once("\n\n===== staged.patch =====\n")
        .expect("bundle patch split");

    let manifest_path = repo_path.join("out/commit-context/commit-context.json");
    let patch_path = repo_path.join("out/commit-context/staged.patch");

    let manifest = fs::read_to_string(manifest_path).expect("read manifest");
    let patch = fs::read_to_string(patch_path).expect("read patch");
    let parsed: Value = serde_json::from_str(json_body).expect("parse commit-context json");
    let generated_at = parsed
        .get("generatedAt")
        .and_then(|value| value.as_str())
        .expect("generatedAt field");

    assert_eq!(manifest, format!("{json_body}\n"));
    assert_eq!(patch, patch_body);
    assert_eq!(generated_at, "2000-01-02T03:04:05Z");
}

#[test]
fn commit_to_stash_fixture_f034() {
    let repo = TempDir::new().expect("tempdir");
    git_init_repo(repo.path());

    fs::write(repo.path().join("hello.txt"), "one\n").expect("write hello");
    git_commit_all(repo.path(), "c1", "2000-01-01T00:00:00+0000");

    fs::write(repo.path().join("hello.txt"), "two\n").expect("write hello");
    git_commit_all(repo.path(), "c2", "2000-01-02T00:00:00+0000");

    let harness = GitCliHarness::new();
    let options = harness.cmd_options(repo.path()).with_stdin_str("y\nn\n");

    let output = run_with(
        &harness.git_cli_bin(),
        &["commit", "to-stash", "HEAD"],
        &options,
    );

    assert_exit_code("F034", &output, 0);
    let stdout = output.stdout_text();
    let stderr = output.stderr_text();
    assert!(
        stderr.trim().is_empty(),
        "expected F034 stderr to be empty, got:\n{stderr}"
    );
    assert_stdout_contains_all(
        "F034",
        &stdout,
        &[
            "🧾 Convert commit → stash",
            "✅ Stash created:",
            "✅ Done. Commit kept; stash saved.",
        ],
    );
}

#[test]
fn commit_to_stash_decline_prompt_aborts() {
    let repo = TempDir::new().expect("tempdir");
    git_init_repo(repo.path());

    fs::write(repo.path().join("hello.txt"), "one\n").expect("write hello");
    git_commit_all(repo.path(), "c1", "2000-01-01T00:00:00+0000");

    fs::write(repo.path().join("hello.txt"), "two\n").expect("write hello");
    git_commit_all(repo.path(), "c2", "2000-01-02T00:00:00+0000");

    let harness = GitCliHarness::new();
    let options = harness.cmd_options(repo.path()).with_stdin_str("n\n");

    let output = run_with(
        &harness.git_cli_bin(),
        &["commit", "to-stash", "HEAD"],
        &options,
    );

    assert_eq!(output.code, 1);
    assert!(
        output
            .stdout_text()
            .contains("❓ Proceed to create stash? [y/N] 🚫 Aborted"),
        "expected abort prompt"
    );
}

#[test]
fn commit_to_stash_merge_commit_prompt() {
    let repo = TempDir::new().expect("tempdir");
    git_init_repo(repo.path());

    fs::write(repo.path().join("hello.txt"), "base\n").expect("write hello");
    git_commit_all(repo.path(), "base", "2000-01-01T00:00:00+0000");

    git(repo.path(), &["checkout", "-q", "-b", "feature"]);
    fs::write(repo.path().join("hello.txt"), "feature\n").expect("write hello");
    git_commit_all(repo.path(), "feat", "2000-01-02T00:00:00+0000");

    git(repo.path(), &["checkout", "-q", "main"]);
    git_with_env(
        repo.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "-q",
            "--no-ff",
            "feature",
            "-m",
            "merge feature",
        ],
        &[
            ("GIT_AUTHOR_NAME", "Fixture Bot"),
            ("GIT_AUTHOR_EMAIL", "fixture@example.invalid"),
            ("GIT_COMMITTER_NAME", "Fixture Bot"),
            ("GIT_COMMITTER_EMAIL", "fixture@example.invalid"),
            ("GIT_AUTHOR_DATE", "2000-01-03T00:00:00+0000"),
            ("GIT_COMMITTER_DATE", "2000-01-03T00:00:00+0000"),
        ],
    );

    let harness = GitCliHarness::new();
    let options = harness.cmd_options(repo.path()).with_stdin_str("n\n");

    let output = run_with(
        &harness.git_cli_bin(),
        &["commit", "to-stash", "HEAD"],
        &options,
    );

    assert_eq!(output.code, 1);
    let stdout = output.stdout_text();
    assert!(stdout.contains("Target commit is a merge commit"));
    assert!(stdout.contains("❓ Proceed? [y/N] 🚫 Aborted"));
}

#[test]
fn commit_to_stash_fallback_refuses_dirty_tree() {
    let repo = TempDir::new().expect("tempdir");
    git_init_repo(repo.path());

    fs::write(repo.path().join("hello.txt"), "one\n").expect("write hello");
    git_commit_all(repo.path(), "c1", "2000-01-01T00:00:00+0000");

    fs::write(repo.path().join("hello.txt"), "two\n").expect("write hello");
    git_commit_all(repo.path(), "c2", "2000-01-02T00:00:00+0000");

    fs::write(repo.path().join("dirty.txt"), "dirty\n").expect("write dirty");

    let harness = GitCliHarness::new();
    let options = harness
        .cmd_options(repo.path())
        .with_env("GIT_CLI_FORCE_STASH_FALLBACK", "1")
        .with_stdin_str("y\ny\n");

    let output = run_with(
        &harness.git_cli_bin(),
        &["commit", "to-stash", "HEAD"],
        &options,
    );

    assert_eq!(output.code, 1);
    let stdout = output.stdout_text();
    assert!(stdout.contains("Fallback would require touching the working tree"));
    assert!(stdout.contains("Working tree is not clean; fallback requires clean state."));
}
