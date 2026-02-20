use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn gemini_cli_bin() -> PathBuf {
    bin::resolve("gemini-cli")
}

fn run(repo: &Path, args: &[&str], path_env: &str, stdin: Option<&str>) -> CmdOutput {
    let mut options = CmdOptions::default()
        .with_cwd(repo)
        .with_env("PATH", path_env);
    if let Some(input) = stdin {
        options = options.with_stdin_str(input);
    }
    cmd::run_with(&gemini_cli_bin(), args, &options)
}

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_repo(repo: &Path) {
    git(repo, &["init"]);
    git(repo, &["config", "user.name", "Test User"]);
    git(repo, &["config", "user.email", "test@example.com"]);
    git(repo, &["config", "commit.gpgSign", "false"]);
    git(repo, &["config", "tag.gpgSign", "false"]);
}

fn real_git_path() -> String {
    let output = Command::new("sh")
        .arg("-c")
        .arg("command -v git")
        .output()
        .expect("which git");
    assert!(output.status.success());
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write_git_proxy(dir: &Path) {
    let git = real_git_path();
    let proxy = dir.join("git");
    fs::write(&proxy, format!("#!/bin/sh\nexec \"{git}\" \"$@\"\n")).expect("write proxy");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&proxy).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&proxy, perms).expect("chmod");
    }
}

#[test]
fn agent_commit_fallback_commits_with_prompted_header() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_repo(&repo);

    fs::create_dir_all(repo.join("src")).expect("src dir");
    fs::write(repo.join("src/lib.rs"), "pub fn hello() {}\n").expect("write file");
    git(&repo, &["add", "src/lib.rs"]);

    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_git_proxy(&bin_dir);
    let path_env = bin_dir.display().to_string();

    let output = run(
        &repo,
        &["agent", "commit"],
        &path_env,
        Some("feat\ncore\nAdd fallback commit\ny\n"),
    );
    assert_eq!(output.code, 0);
    assert!(String::from_utf8_lossy(&output.stderr).contains("fallback mode"));

    let subject = git_stdout(&repo, &["log", "-1", "--pretty=%s"]);
    assert_eq!(subject, "feat(core): Add fallback commit");
}

#[test]
fn agent_commit_fallback_defaults_type_and_scope() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_repo(&repo);

    fs::create_dir_all(repo.join("src")).expect("src dir");
    fs::write(repo.join("src/main.rs"), "fn main() {}\n").expect("write file");
    git(&repo, &["add", "src/main.rs"]);

    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_git_proxy(&bin_dir);
    let path_env = bin_dir.display().to_string();

    let output = run(
        &repo,
        &["agent", "commit"],
        &path_env,
        Some("\n\nUse defaults\ny\n"),
    );
    assert_eq!(output.code, 0);

    let subject = git_stdout(&repo, &["log", "-1", "--pretty=%s"]);
    assert_eq!(subject, "chore(src): Use defaults");
}

#[test]
fn agent_commit_fallback_aborts_on_confirmation_reject() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_repo(&repo);

    fs::write(repo.join("README.md"), "hello\n").expect("write file");
    git(&repo, &["add", "README.md"]);

    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_git_proxy(&bin_dir);
    let path_env = bin_dir.display().to_string();

    let output = run(
        &repo,
        &["agent", "commit"],
        &path_env,
        Some("fix\nrepo\nAbort this\nn\n"),
    );
    assert_eq!(output.code, 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Aborted."));
}

#[test]
fn agent_commit_fallback_push_flag_returns_1_when_push_fails() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_repo(&repo);

    fs::write(repo.join("README.md"), "hello\n").expect("write file");
    git(&repo, &["add", "README.md"]);

    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_git_proxy(&bin_dir);
    let path_env = bin_dir.display().to_string();

    let output = run(
        &repo,
        &["agent", "commit", "--push"],
        &path_env,
        Some("fix\nrepo\nCommit then fail push\ny\n"),
    );
    assert_eq!(output.code, 1);

    // Commit still succeeded before push failed.
    let subject = git_stdout(&repo, &["log", "-1", "--pretty=%s"]);
    assert_eq!(subject, "fix(repo): Commit then fail push");
}

#[test]
fn agent_commit_without_staged_changes_returns_1() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_repo(&repo);

    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_git_proxy(&bin_dir);
    let path_env = bin_dir.display().to_string();

    let output = run(&repo, &["agent", "commit"], &path_env, None);
    assert_eq!(output.code, 1);
    assert!(String::from_utf8_lossy(&output.stderr).contains("no staged changes"));
}

#[test]
fn agent_commit_auto_stage_in_fallback_stages_and_commits() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_repo(&repo);

    fs::write(repo.join("README.md"), "hello\n").expect("write file");

    let bin_dir = dir.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir");
    write_git_proxy(&bin_dir);
    let path_env = bin_dir.display().to_string();

    let output = run(
        &repo,
        &["agent", "commit", "--auto-stage"],
        &path_env,
        Some("chore\nrepo\nAuto stage commit\ny\n"),
    );
    assert_eq!(output.code, 0);

    let subject = git_stdout(&repo, &["log", "-1", "--pretty=%s"]);
    assert_eq!(subject, "chore(repo): Auto stage commit");
}
