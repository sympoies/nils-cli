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

#[test]
fn tracked_print_works_when_file_missing() {
    let repo = common::init_repo();
    let root = repo.path();

    fs::write(root.join("tracked.txt"), "HELLO_FROM_TEXT").unwrap();
    let bytes = [0u8, 159u8, 146u8, 150u8];
    fs::write(root.join("bin.dat"), bytes).unwrap();
    common::git(root, &["add", "."]);
    common::git(root, &["commit", "-m", "add files"]);

    let stub = tempfile::TempDir::new().unwrap();
    let git_path = which_cmd("git");
    let mktemp_path = which_cmd("mktemp");
    symlink(&git_path, stub.path().join("git")).unwrap();
    symlink(&mktemp_path, stub.path().join("mktemp")).unwrap();

    let path_env = stub.path().to_string_lossy().to_string();
    let output = common::run_git_scope(
        root,
        &["tracked", "-p"],
        &[("NO_COLOR", "1"), ("PATH", path_env.as_str())],
    );

    assert!(
        output.contains("📄 tracked.txt (working tree)"),
        "text label missing: {output}"
    );
    assert!(
        output.contains("HELLO_FROM_TEXT"),
        "text content missing: {output}"
    );
    assert!(
        output.contains("📄 bin.dat (binary file in working tree)"),
        "binary label missing: {output}"
    );
    assert!(
        output.contains("[Binary file content omitted]"),
        "binary placeholder missing: {output}"
    );
}

fn which_git() -> String {
    which_cmd("git")
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
