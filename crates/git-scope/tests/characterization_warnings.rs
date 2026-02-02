mod common;

use std::fs;
use std::os::unix::fs::symlink;

#[test]
fn commit_invalid_parent_warning_is_stable() {
    let repo = common::init_repo();
    let root = repo.path();

    fs::write(root.join("base.txt"), "base").unwrap();
    common::git(root, &["add", "base.txt"]);
    common::git(root, &["commit", "-m", "base"]);

    common::git(root, &["checkout", "-b", "feature"]);
    fs::write(root.join("feature.txt"), "feature").unwrap();
    common::git(root, &["add", "feature.txt"]);
    common::git(root, &["commit", "-m", "feature"]);

    common::git(root, &["checkout", "main"]);
    fs::write(root.join("main.txt"), "main").unwrap();
    common::git(root, &["add", "main.txt"]);
    common::git(root, &["commit", "-m", "main"]);

    common::git(
        root,
        &["merge", "--no-ff", "feature", "-m", "merge feature"],
    );
    let merge_hash = common::git(root, &["rev-parse", "HEAD"]).trim().to_string();

    let output = common::run_git_scope(
        root,
        &["commit", &merge_hash, "--parent", "nope"],
        &[("NO_COLOR", "1")],
    );

    assert!(
        output.contains("  ⚠️  Invalid --parent value 'nope' — falling back to parent #1"),
        "invalid parent warning missing: {output}"
    );
}

#[test]
fn tree_missing_warning_is_stable() {
    let repo = common::init_repo();
    let root = repo.path();

    fs::write(root.join("file.txt"), "base").unwrap();
    common::git(root, &["add", "file.txt"]);
    common::git(root, &["commit", "-m", "base"]);

    fs::write(root.join("file.txt"), "change").unwrap();
    common::git(root, &["add", "file.txt"]);

    let stub = tempfile::TempDir::new().unwrap();
    let git_path = which_cmd("git");
    symlink(&git_path, stub.path().join("git")).unwrap();

    let path_env = stub.path().to_string_lossy().to_string();
    let output = common::run_git_scope(
        root,
        &["staged"],
        &[("NO_COLOR", "1"), ("PATH", path_env.as_str())],
    );

    assert!(
        output.contains("⚠️  tree is not installed. Install it to see the directory tree."),
        "missing tree warning not found: {output}"
    );
}

#[test]
fn missing_file_warning_is_stable() {
    let repo = common::init_repo();
    let root = repo.path();

    fs::write(root.join("vanish.txt"), "gone").unwrap();
    common::git(root, &["add", "vanish.txt"]);
    common::git(root, &["commit", "-m", "add vanish"]);
    let old_commit = common::git(root, &["rev-parse", "HEAD"]).trim().to_string();

    common::git(root, &["rm", "vanish.txt"]);
    common::git(root, &["commit", "-m", "remove vanish"]);

    let output = common::run_git_scope(root, &["commit", &old_commit, "-p"], &[("NO_COLOR", "1")]);

    assert!(
        output.contains("❗ File not found: vanish.txt"),
        "missing file warning not found: {output}"
    );
}

fn which_cmd(cmd: &str) -> String {
    let output = std::process::Command::new("which")
        .arg(cmd)
        .output()
        .unwrap_or_else(|_| panic!("which {cmd}"));
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(!path.is_empty(), "{cmd} not found in PATH for tests");
    path
}
