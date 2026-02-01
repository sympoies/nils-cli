use std::fs;
use std::path::PathBuf;
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

fn run(args: &[&str], vars: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(codex_cli_bin());
    cmd.args(args);
    for (key, value) in vars {
        cmd.env(key, value);
    }
    cmd.output().expect("run codex-cli")
}

fn run_with_stdin(args: &[&str], vars: &[(&str, &str)], stdin: &str) -> Output {
    let mut cmd = Command::new(codex_cli_bin());
    cmd.args(args);
    for (key, value) in vars {
        cmd.env(key, value);
    }
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn codex-cli");
    if let Some(mut handle) = child.stdin.take() {
        use std::io::Write;
        handle.write_all(stdin.as_bytes()).expect("write stdin");
    }
    child.wait_with_output().expect("wait")
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

fn make_exe(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod");
    }
}

fn write_stub_codex(stub_dir: &std::path::Path, out_dir: &std::path::Path) {
    fs::create_dir_all(stub_dir).expect("stub dir");
    fs::create_dir_all(out_dir).expect("out dir");
    let script = r#"#!/usr/bin/env bash
set -euo pipefail
out_dir="${CODEX_STUB_OUT_DIR}"
mkdir -p "$out_dir"
i=0
for arg in "$@"; do
  printf '%s' "$arg" > "$out_dir/arg-$i"
  i=$((i+1))
done
exit 0
"#;
    let path = stub_dir.join("codex");
    fs::write(&path, script).expect("write codex stub");
    make_exe(&path);
}

#[test]
fn agent_advice_substitutes_arguments() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let zdotdir = dir.path().join("zdotdir");
    let prompts = zdotdir.join("prompts");
    fs::create_dir_all(&prompts).expect("prompts dir");
    fs::write(prompts.join("actionable-advice.md"), "Advice: $ARGUMENTS\n").expect("template");

    let zdotdir_str = zdotdir.to_string_lossy().to_string();

    let stub_dir = dir.path().join("bin");
    let out_dir = dir.path().join("out");
    write_stub_codex(&stub_dir, &out_dir);

    let out_dir_str = out_dir.to_string_lossy().to_string();

    let path = std::env::var("PATH").unwrap_or_default();
    let combined_path = format!("{}:{}", stub_dir.to_string_lossy(), path);

    let output = run(
        &["agent", "advice", "hello", "world"],
        &[
            ("CODEX_ALLOW_DANGEROUS_ENABLED", "true"),
            ("ZDOTDIR", &zdotdir_str),
            ("PATH", &combined_path),
            ("CODEX_STUB_OUT_DIR", &out_dir_str),
        ],
    );
    assert_exit(&output, 0);

    let prompt_arg = fs::read_to_string(out_dir.join("arg-9")).expect("prompt arg");
    assert_eq!(prompt_arg, "Advice: hello world\n");
}

#[test]
fn agent_knowledge_missing_template_prints_error_prefix() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let zdotdir = dir.path().join("zdotdir");
    let prompts = zdotdir.join("prompts");
    fs::create_dir_all(&prompts).expect("prompts dir");

    let zdotdir_str = zdotdir.to_string_lossy().to_string();

    let output = run(
        &["agent", "knowledge", "x"],
        &[
            ("CODEX_ALLOW_DANGEROUS_ENABLED", "true"),
            ("ZDOTDIR", &zdotdir_str),
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("codex-tools: prompt template not found:"));
}

#[test]
fn agent_advice_blank_question_exits_1_with_missing_question_message() {
    let output = run_with_stdin(
        &["agent", "advice"],
        &[("CODEX_ALLOW_DANGEROUS_ENABLED", "true")],
        "\n",
    );
    assert_exit(&output, 1);
    assert!(stdout(&output).contains("Question: "));
    assert!(stderr(&output).contains("codex-tools: missing question"));
}
