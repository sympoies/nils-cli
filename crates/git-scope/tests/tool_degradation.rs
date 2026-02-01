mod common;

use std::fs;
use std::os::unix::fs::symlink;

#[test]
fn tracked_warns_when_tree_missing() {
    let repo = common::init_repo();
    let root = repo.path();

    fs::write(root.join("tracked.txt"), "hello").unwrap();
    common::git(root, &["add", "tracked.txt"]);
    common::git(root, &["commit", "-m", "tracked"]);

    let stub = tempfile::TempDir::new().unwrap();
    let git_path = which_git();
    let link_path = stub.path().join("git");
    symlink(&git_path, &link_path).unwrap();

    let path_env = stub.path().to_string_lossy().to_string();
    let output = common::run_git_scope(
        root,
        &["tracked"],
        &[("NO_COLOR", "1"), ("PATH", path_env.as_str())],
    );

    assert!(output.contains("tree is not installed"));
}

fn which_git() -> String {
    let output = std::process::Command::new("which")
        .arg("git")
        .output()
        .expect("which git");
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(!path.is_empty(), "git not found in PATH for tests");
    path
}
