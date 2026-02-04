mod common;

use common::{git, init_bare_remote, init_repo, GitCliHarness};
use nils_test_support::git::commit_file;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn add_remote(repo: &Path, name: &str, remote: &TempDir) {
    let remote_path = remote.path().to_string_lossy().to_string();
    git(repo, &["remote", "add", name, &remote_path]);
}

fn push_main(repo: &Path, remote: &str) {
    git(repo, &["push", "-u", remote, "main"]);
}

fn git_path(repo: &Path, path: &str) -> std::path::PathBuf {
    let raw = git(repo, &["rev-parse", "--git-path", path]);
    let candidate = std::path::PathBuf::from(raw.trim_end_matches(['\n', '\r']));
    if candidate.is_absolute() {
        candidate
    } else {
        repo.join(candidate)
    }
}

#[test]
fn ci_pick_usage_missing_args() {
    let harness = GitCliHarness::new();
    let dir = init_repo();

    let output = harness.run(dir.path(), &["ci", "pick"]);

    assert_eq!(output.code, 2);
    assert_eq!(output.stdout_text(), "");
    assert_eq!(
        output.stderr_text(),
        "❌ Usage: git-pick <target> <commit-or-range> <name>\n   Try: git-pick --help\n"
    );
}

#[test]
fn ci_pick_refuses_in_progress_op() {
    let harness = GitCliHarness::new();
    let dir = init_repo();

    let cherry_pick = git_path(dir.path(), "CHERRY_PICK_HEAD");
    fs::write(&cherry_pick, "conflict").expect("write cherry pick head");

    let output = harness.run(dir.path(), &["ci", "pick", "main", "HEAD", "test"]);

    assert_eq!(output.code, 1);
    assert_eq!(output.stdout_text(), "");
    assert_eq!(
        output.stderr_text(),
        "❌ Refusing to run during an in-progress Git operation:\n   - cherry-pick in progress\n"
    );
}

#[test]
fn ci_pick_refuses_unstaged_changes() {
    let harness = GitCliHarness::new();
    let dir = init_repo();

    fs::write(dir.path().join("README.md"), "dirty\n").expect("write change");

    let output = harness.run(dir.path(), &["ci", "pick", "main", "HEAD", "test"]);

    assert_eq!(output.code, 1);
    assert_eq!(output.stdout_text(), "");
    assert_eq!(
        output.stderr_text(),
        "❌ Unstaged changes detected. Commit or stash before running git-pick.\n"
    );
}

#[test]
fn ci_pick_refuses_staged_changes() {
    let harness = GitCliHarness::new();
    let dir = init_repo();

    fs::write(dir.path().join("README.md"), "staged\n").expect("write change");
    git(dir.path(), &["add", "README.md"]);

    let output = harness.run(dir.path(), &["ci", "pick", "main", "HEAD", "test"]);

    assert_eq!(output.code, 1);
    assert_eq!(output.stdout_text(), "");
    assert_eq!(
        output.stderr_text(),
        "❌ Staged changes detected. Commit or stash before running git-pick.\n"
    );
}

#[test]
fn ci_pick_creates_and_pushes_branch() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    let remote = init_bare_remote();

    add_remote(dir.path(), "origin", &remote);
    push_main(dir.path(), "origin");

    git(dir.path(), &["switch", "-q", "-c", "feature"]);
    let commit_sha = commit_file(dir.path(), "feature.txt", "feature\n", "feature");
    git(dir.path(), &["switch", "-q", "main"]);

    let output = harness.run(
        dir.path(),
        &["ci", "pick", "origin/main", &commit_sha, "try"],
    );

    assert_eq!(output.code, 0);
    assert!(output.stdout_text().contains("🌿 CI branch: ci/main/try\n"));
    assert!(output.stdout_text().contains("🔧 Base     : origin/main\n"));
    assert!(output
        .stdout_text()
        .contains(&format!("🍒 Pick     : {commit_sha} (1 commit(s))\n")));
    assert!(output
        .stdout_text()
        .contains("✅ Pushed: origin/ci/main/try (CI should run on branch push)\n"));

    let remote_refs = git(remote.path(), &["show-ref", "--heads", "ci/main/try"]);
    assert!(remote_refs.contains("ci/main/try"));

    let head = git(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(head.trim(), "main");
}

#[test]
fn ci_pick_reports_cherry_pick_failure() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    let remote = init_bare_remote();

    add_remote(dir.path(), "origin", &remote);
    push_main(dir.path(), "origin");

    git(dir.path(), &["switch", "-q", "-c", "feature"]);
    let commit_sha = commit_file(dir.path(), "conflict.txt", "feature\n", "feature");
    git(dir.path(), &["switch", "-q", "main"]);
    commit_file(dir.path(), "conflict.txt", "main\n", "main");

    let output = harness.run(dir.path(), &["ci", "pick", "main", &commit_sha, "conflict"]);

    assert_eq!(output.code, 1);
    assert!(output
        .stderr_text()
        .contains("❌ Cherry-pick failed on branch: ci/main/conflict\n"));
    assert!(output
        .stderr_text()
        .contains("🧠 Resolve conflicts then run: git cherry-pick --continue\n"));
    assert!(output
        .stderr_text()
        .contains("    Or abort and retry:        git cherry-pick --abort\n"));
}
