mod common;

use common::{commit_file, init_repo, run_git_lock, run_git_lock_output};
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
