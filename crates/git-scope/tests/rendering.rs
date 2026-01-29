mod common;

use std::fs;
use std::os::unix::fs::symlink;

#[test]
fn no_color_outputs_no_ansi() {
    let repo = common::init_repo();
    let root = repo.path();

    fs::write(root.join("file.txt"), "base").unwrap();
    common::git(root, &["add", "file.txt"]);
    common::git(root, &["commit", "-m", "base"]);

    fs::write(root.join("file.txt"), "change").unwrap();
    common::git(root, &["add", "file.txt"]);

    let output = common::run_git_scope(root, &["staged"], &[("NO_COLOR", "1")]);
    assert!(!output.contains("\x1b["), "unexpected ANSI codes: {output}");
}

#[test]
fn tree_missing_emits_warning() {
    let repo = common::init_repo();
    let root = repo.path();

    fs::write(root.join("file.txt"), "base").unwrap();
    common::git(root, &["add", "file.txt"]);
    common::git(root, &["commit", "-m", "base"]);

    fs::write(root.join("file.txt"), "change").unwrap();
    common::git(root, &["add", "file.txt"]);

    let temp_path = tempfile::TempDir::new().unwrap();
    let git_path = which_git();
    let link_path = temp_path.path().join("git");
    symlink(&git_path, &link_path).unwrap();

    let output = common::run_git_scope(
        root,
        &["staged"],
        &[("PATH", temp_path.path().to_str().unwrap())],
    );

    assert!(
        output.contains("tree is not installed"),
        "tree missing warning not found: {output}"
    );
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
