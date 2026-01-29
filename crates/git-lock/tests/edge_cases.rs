mod common;

use common::{init_repo, run_git_lock_output};

#[test]
fn not_a_repo() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let output = run_git_lock_output(
        dir.path(),
        &["list"],
        &[("ZSH_CACHE_DIR", dir.path().to_str().unwrap())],
        None,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Not a Git repository"));
    assert!(!output.status.success());
}

#[test]
fn invalid_commit() {
    let repo = init_repo();
    let cache = tempfile::TempDir::new().expect("cache");
    let output = run_git_lock_output(
        repo.path(),
        &["lock", "bad", "note", "BADCOMMIT"],
        &[("ZSH_CACHE_DIR", cache.path().to_str().unwrap())],
        None,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("❌ Invalid commit: BADCOMMIT"));
    assert!(!output.status.success());
}

#[test]
fn missing_latest_unlock() {
    let repo = init_repo();
    let cache = tempfile::TempDir::new().expect("cache");
    let output = run_git_lock_output(
        repo.path(),
        &["unlock"],
        &[("ZSH_CACHE_DIR", cache.path().to_str().unwrap())],
        Some("n\n"),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No recent git-lock found"));
    assert!(!output.status.success());
}

#[test]
fn diff_too_many_labels() {
    let repo = init_repo();
    let cache = tempfile::TempDir::new().expect("cache");
    let output = run_git_lock_output(
        repo.path(),
        &["diff", "a", "b", "c"],
        &[("ZSH_CACHE_DIR", cache.path().to_str().unwrap())],
        None,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Too many labels provided"));
    assert!(stdout.contains("Usage: git-lock diff"));
    assert!(!output.status.success());
}
