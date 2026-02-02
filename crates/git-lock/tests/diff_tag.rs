mod common;

use common::{commit_file, init_repo, run_git_lock, run_git_lock_output};
use std::path::Path;
use tempfile::TempDir;

fn cache_dir() -> TempDir {
    tempfile::TempDir::new().expect("cache dir")
}

#[test]
fn diff_no_color() {
    let repo = init_repo();
    let cache = cache_dir();
    let env = [("ZSH_CACHE_DIR", cache.path().to_str().unwrap())];

    run_git_lock(repo.path(), &["lock", "base"], &env, None);
    commit_file(repo.path(), "file.txt", "change", "change");
    run_git_lock(repo.path(), &["lock", "next"], &env, None);

    let output = run_git_lock_output(
        repo.path(),
        &["diff", "base", "next", "--no-color"],
        &env,
        None,
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("🧮 Comparing commits"));
    assert!(!stdout.contains("\u{1b}["));
}

#[test]
fn tag_overwrite_prompt() {
    let repo = init_repo();
    let cache = cache_dir();
    let env = [("ZSH_CACHE_DIR", cache.path().to_str().unwrap())];

    run_git_lock(repo.path(), &["lock", "wip"], &env, None);

    let status = std::process::Command::new("git")
        .args(["tag", "-a", "v1.0.0", "-m", "tag"])
        .current_dir(repo.path())
        .status()
        .expect("tag command");
    assert!(status.success());

    let output = run_git_lock_output(repo.path(), &["tag", "wip", "v1.0.0"], &env, Some("n\n"));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Git tag [v1.0.0] already exists."));
    assert!(stdout.contains("Overwrite it?"));
    assert!(stdout.contains("🚫 Aborted"));
    assert!(!output.status.success());
}

#[test]
fn tag_defaults_message_from_commit_subject() {
    let repo = init_repo();
    let cache = cache_dir();
    let env = [("ZSH_CACHE_DIR", cache.path().to_str().unwrap())];

    run_git_lock(repo.path(), &["lock", "wip"], &env, None);

    let subject = common::git(repo.path(), &["log", "-1", "--pretty=format:%s"])
        .trim()
        .to_string();

    let output = run_git_lock_output(repo.path(), &["tag", "wip", "v2.0.0"], &env, None);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(stdout.contains(&format!("📝 Message: {subject}")));
}

#[test]
fn tag_missing_args_usage() {
    let repo = init_repo();
    let cache = cache_dir();
    let env = [("ZSH_CACHE_DIR", cache.path().to_str().unwrap())];

    let output = run_git_lock_output(repo.path(), &["tag", "wip"], &env, None);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage: git-lock tag"));
    assert!(!output.status.success());
}

#[test]
fn tag_missing_lock_file_reports_error() {
    let repo = init_repo();
    let cache = cache_dir();
    let env = [("ZSH_CACHE_DIR", cache.path().to_str().unwrap())];
    let repo_name = common::repo_id(repo.path());

    let output = run_git_lock_output(repo.path(), &["tag", "missing", "v0.0.1"], &env, None);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("git-lock [missing] not found in"));
    assert!(stdout.contains(&format!("for [{repo_name}]")));
    assert!(!output.status.success());
}

#[test]
fn tag_pushes_and_deletes_local_tag() {
    let repo = init_repo();
    let cache = cache_dir();
    let env = [("ZSH_CACHE_DIR", cache.path().to_str().unwrap())];

    run_git_lock(repo.path(), &["lock", "wip"], &env, None);

    let remote = tempfile::TempDir::new().expect("remote");
    init_bare_repo(remote.path());
    common::git(
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );

    let output = run_git_lock_output(
        repo.path(),
        &["tag", "wip", "v3.0.0", "-m", "release", "--push"],
        &env,
        None,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(stdout.contains("Created tag [v3.0.0]"));
    assert!(stdout.contains("📝 Message: release"));
    assert!(stdout.contains("Pushed tag [v3.0.0] to origin"));
    assert!(stdout.contains("Deleted local tag [v3.0.0]"));
}

fn init_bare_repo(path: &Path) {
    common::git(path, &["init", "--bare", "-q"]);
}
