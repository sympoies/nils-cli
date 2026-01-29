mod common;

use common::{init_repo, repo_id, run_git_lock, run_git_lock_output};
use std::path::PathBuf;
use tempfile::TempDir;

fn cache_dir() -> TempDir {
    tempfile::TempDir::new().expect("cache dir")
}

fn lock_path(cache: &TempDir, repo: &str, label: &str) -> PathBuf {
    cache
        .path()
        .join("git-locks")
        .join(format!("{repo}-{label}.lock"))
}

#[test]
fn lock_default() {
    let repo = init_repo();
    let cache = cache_dir();
    let repo_name = repo_id(repo.path());

    let output = run_git_lock(
        repo.path(),
        &["lock"],
        &[("ZSH_CACHE_DIR", cache.path().to_str().unwrap())],
        None,
    );

    assert!(output.contains(&format!("🔐 [{repo_name}:default] Locked:")));
    assert!(output.contains("    at "));

    let lock_file = lock_path(&cache, &repo_name, "default");
    assert!(lock_file.exists());
}

#[test]
fn unlock_cancel() {
    let repo = init_repo();
    let cache = cache_dir();
    let repo_name = repo_id(repo.path());

    run_git_lock(
        repo.path(),
        &["lock", "wip"],
        &[("ZSH_CACHE_DIR", cache.path().to_str().unwrap())],
        None,
    );

    let output = run_git_lock_output(
        repo.path(),
        &["unlock", "wip"],
        &[("ZSH_CACHE_DIR", cache.path().to_str().unwrap())],
        Some("n\n"),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("🔐 Found [{repo_name}:wip]")));
    assert!(stdout.contains("Hard reset to [wip]?"));
    assert!(stdout.contains("🚫 Aborted"));
    assert!(!output.status.success());
}
