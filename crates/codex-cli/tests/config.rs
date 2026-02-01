use pretty_assertions::assert_eq;
use std::path::PathBuf;
use std::process::{Command, Output};

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

#[test]
fn config_show_prints_effective_values() {
    let output = run(
        &["config", "show"],
        &[
            ("CODEX_CLI_MODEL", "m1"),
            ("CODEX_CLI_REASONING", "low"),
            ("CODEX_ALLOW_DANGEROUS_ENABLED", "true"),
            ("CODEX_SECRET_DIR", "/tmp/secrets"),
            ("CODEX_AUTH_FILE", "/tmp/auth.json"),
            ("CODEX_SECRET_CACHE_DIR", "/tmp/cache/secrets"),
            ("CODEX_AUTO_REFRESH_ENABLED", "true"),
            ("CODEX_AUTO_REFRESH_MIN_DAYS", "9"),
        ],
    );
    assert_exit(&output, 0);
    let out = stdout(&output);
    assert!(out.contains("CODEX_CLI_MODEL=m1\n"));
    assert!(out.contains("CODEX_CLI_REASONING=low\n"));
    assert!(out.contains("CODEX_ALLOW_DANGEROUS_ENABLED=true\n"));
    assert!(out.contains("CODEX_SECRET_DIR=/tmp/secrets\n"));
    assert!(out.contains("CODEX_AUTH_FILE=/tmp/auth.json\n"));
    assert!(out.contains("CODEX_SECRET_CACHE_DIR=/tmp/cache/secrets\n"));
    assert!(out.contains("CODEX_AUTO_REFRESH_ENABLED=true\n"));
    assert!(out.contains("CODEX_AUTO_REFRESH_MIN_DAYS=9\n"));
}

#[test]
fn config_show_prints_blank_paths_when_unresolvable() {
    let output = Command::new(codex_cli_bin())
        .args(["config", "show"])
        .env_remove("HOME")
        .env_remove("ZDOTDIR")
        .env_remove("ZSH_SCRIPT_DIR")
        .env_remove("_ZSH_BOOTSTRAP_PRELOAD_PATH")
        .env_remove("ZSH_CACHE_DIR")
        .env_remove("CODEX_SECRET_DIR")
        .env_remove("CODEX_AUTH_FILE")
        .env_remove("CODEX_SECRET_CACHE_DIR")
        .output()
        .expect("run codex-cli");
    assert_exit(&output, 0);

    let out = stdout(&output);
    assert!(out.contains("CODEX_SECRET_DIR=\n"));
    assert!(out.contains("CODEX_AUTH_FILE=\n"));
    assert!(out.contains("CODEX_SECRET_CACHE_DIR=\n"));
}

#[test]
fn config_set_model_prints_export() {
    let output = run(&["config", "set", "model", "gpt-test"], &[]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "export CODEX_CLI_MODEL='gpt-test'\n");
}

#[test]
fn config_set_reasoning_prints_export() {
    let output = run(&["config", "set", "reasoning", "high"], &[]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "export CODEX_CLI_REASONING='high'\n");
}

#[test]
fn config_set_dangerous_prints_export_for_true() {
    let output = run(&["config", "set", "dangerous", "true"], &[]);
    assert_exit(&output, 0);
    assert_eq!(
        stdout(&output),
        "export CODEX_ALLOW_DANGEROUS_ENABLED=true\n"
    );
}

#[test]
fn config_set_unknown_key_exits_64() {
    let output = run(&["config", "set", "wat", "x"], &[]);
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("unknown key"));
}

#[test]
fn config_set_model_quotes_empty_value() {
    let output = run(&["config", "set", "model", ""], &[]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "export CODEX_CLI_MODEL=''\n");
}

#[test]
fn config_set_model_escapes_single_quotes() {
    let output = run(&["config", "set", "model", "a'b"], &[]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "export CODEX_CLI_MODEL='a'\"'\"'b'\n");
}

#[test]
fn config_set_dangerous_rejects_invalid_values() {
    let output = run(&["config", "set", "dangerous", "maybe"], &[]);
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("dangerous must be true|false"));
}
