use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn codex_cli_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_codex-cli")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_codex_cli"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("codex-cli");
    if bin.exists() {
        return bin;
    }

    panic!("codex-cli binary path: NotPresent");
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_exit(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "unexpected exit code.\nstdout:\n{}\nstderr:\n{}",
        stdout(output),
        stderr(output)
    );
}

fn make_exe(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }
}

fn real_git_path() -> String {
    let out = Command::new("sh")
        .arg("-c")
        .arg("command -v git")
        .output()
        .expect("which git");
    assert!(out.status.success());
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!path.is_empty());
    path
}

fn write_stub_git(dir: &Path) {
    let git = real_git_path();
    let script = format!(
        r#"#!/bin/sh
exec "{git}" "$@"
"#
    );
    let path = dir.join("git");
    fs::write(&path, script).expect("write git stub");
    make_exe(&path);
}

fn write_stub_semantic_commit(dir: &Path) {
    let script = r#"#!/bin/sh
exit 0
"#;
    let path = dir.join("semantic-commit");
    fs::write(&path, script).expect("write semantic-commit stub");
    make_exe(&path);
}

fn write_stub_codex(dir: &Path) {
    let script = r#"#!/bin/bash
set -euo pipefail
out_dir="${CODEX_STUB_OUT_DIR:?missing CODEX_STUB_OUT_DIR}"
i=0
for arg in "$@"; do
  printf '%s' "$arg" > "$out_dir/arg-$i"
  i=$((i+1))
done
"#;
    let path = dir.join("codex");
    fs::write(&path, script).expect("write codex stub");
    make_exe(&path);
}

fn init_repo(dir: &Path) {
    let status = Command::new("git")
        .current_dir(dir)
        .arg("init")
        .status()
        .expect("git init");
    assert!(status.success());

    let status = Command::new("git")
        .current_dir(dir)
        .args(["config", "user.name", "Test User"])
        .status()
        .expect("git config name");
    assert!(status.success());

    let status = Command::new("git")
        .current_dir(dir)
        .args(["config", "user.email", "test@example.com"])
        .status()
        .expect("git config email");
    assert!(status.success());

    let status = Command::new("git")
        .current_dir(dir)
        .args(["config", "commit.gpgsign", "false"])
        .status()
        .expect("git config gpgsign");
    assert!(status.success());
}

#[test]
fn agent_commit_fallback_creates_commit() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_repo(&repo);

    fs::write(repo.join("a.txt"), "hello").expect("write file");
    let status = Command::new("git")
        .current_dir(&repo)
        .args(["add", "a.txt"])
        .status()
        .expect("git add");
    assert!(status.success());

    let stub_dir = dir.path().join("bin");
    fs::create_dir_all(&stub_dir).expect("stub dir");
    write_stub_git(&stub_dir);

    let mut cmd = Command::new(codex_cli_bin());
    cmd.current_dir(&repo);
    cmd.args(["agent", "commit"]);
    cmd.env("PATH", stub_dir.to_string_lossy().to_string());
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("spawn codex-cli");
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(b"\n\nmy subject\ny\n")
            .expect("write stdin");
    }
    let output = child.wait_with_output().expect("wait");
    assert_exit(&output, 0);
    assert!(stderr(&output).contains("fallback mode"));

    let out = Command::new("git")
        .current_dir(&repo)
        .args(["log", "-1", "--pretty=%s"])
        .output()
        .expect("git log");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "chore: my subject"
    );
}

#[test]
fn agent_commit_semantic_mode_executes_codex_with_template_and_push_note() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_repo(&repo);

    fs::write(repo.join("a.txt"), "hello").expect("write file");

    let zdotdir = dir.path().join("zdotdir");
    let prompts_dir = zdotdir.join("prompts");
    fs::create_dir_all(&prompts_dir).expect("prompts dir");
    fs::write(
        prompts_dir.join("semantic-commit-autostage.md"),
        "SEMANTIC_AUTOSTAGE\n",
    )
    .expect("write template");

    let stub_dir = dir.path().join("bin");
    fs::create_dir_all(&stub_dir).expect("stub dir");
    write_stub_git(&stub_dir);
    write_stub_semantic_commit(&stub_dir);
    write_stub_codex(&stub_dir);

    let out_dir = dir.path().join("out");
    fs::create_dir_all(&out_dir).expect("out dir");
    let out_dir_str = out_dir.to_string_lossy().to_string();

    let mut cmd = Command::new(codex_cli_bin());
    cmd.current_dir(&repo);
    cmd.args([
        "agent",
        "commit",
        "--push",
        "--auto-stage",
        "extra",
        "words",
    ]);
    cmd.env("PATH", stub_dir.to_string_lossy().to_string());
    cmd.env("CODEX_ALLOW_DANGEROUS_ENABLED", "true");
    cmd.env("CODEX_CLI_MODEL", "m-test");
    cmd.env("CODEX_CLI_REASONING", "low");
    cmd.env("ZDOTDIR", zdotdir.to_string_lossy().to_string());
    cmd.env("CODEX_STUB_OUT_DIR", &out_dir_str);

    let output = cmd.output().expect("run codex-cli");
    assert_exit(&output, 0);

    let prompt = fs::read_to_string(out_dir.join("arg-9")).expect("prompt");
    assert!(prompt.contains("SEMANTIC_AUTOSTAGE"));
    assert!(prompt.contains("Furthermore, please push the committed changes"));
    assert!(prompt.contains("Additional instructions from user:"));
    assert!(prompt.contains("extra words"));

    let arg5 = fs::read_to_string(out_dir.join("arg-5")).expect("model");
    assert_eq!(arg5, "m-test");
    let arg7 = fs::read_to_string(out_dir.join("arg-7")).expect("reasoning");
    assert_eq!(arg7, "model_reasoning_effort=\"low\"");
}
