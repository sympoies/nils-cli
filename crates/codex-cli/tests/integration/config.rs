use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use pretty_assertions::assert_eq;
use std::path::PathBuf;

fn codex_cli_bin() -> PathBuf {
    bin::resolve("codex-cli")
}

fn run(args: &[&str], vars: &[(&str, &str)]) -> CmdOutput {
    let mut options = CmdOptions::default();
    for (key, value) in vars {
        options = options.with_env(key, value);
    }
    let bin = codex_cli_bin();
    cmd::run_with(&bin, args, &options)
}

fn stdout(output: &CmdOutput) -> String {
    output.stdout_text()
}

fn stderr(output: &CmdOutput) -> String {
    output.stderr_text()
}

fn assert_exit(output: &CmdOutput, code: i32) {
    assert_eq!(
        output.code,
        code,
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
            ("CODEX_CLI_EPHEMERAL_ENABLED", "true"),
            ("CODEX_ALLOW_DANGEROUS_ENABLED", "true"),
            ("CODEX_SECRET_DIR", "/tmp/secrets"),
            ("CODEX_AUTH_FILE", "/tmp/auth.json"),
            ("CODEX_SECRET_CACHE_DIR", "/tmp/cache/secrets"),
            ("CODEX_PROMPT_SEGMENT_ENABLED", "true"),
            ("CODEX_AUTO_REFRESH_ENABLED", "true"),
            ("CODEX_AUTO_REFRESH_MIN_DAYS", "9"),
            ("CODEX_AUTH_REMOTE_SSH", "g14"),
            ("CODEX_AUTH_REMOTE_NAME", "gamania"),
            ("CODEX_AUTH_REMOTE_REFRESH", "false"),
        ],
    );
    assert_exit(&output, 0);
    let out = stdout(&output);
    assert!(out.contains("CODEX_CLI_MODEL=m1\n"));
    assert!(out.contains("CODEX_CLI_REASONING=low\n"));
    assert!(out.contains("CODEX_CLI_EPHEMERAL_ENABLED=true\n"));
    assert!(out.contains("CODEX_ALLOW_DANGEROUS_ENABLED=true\n"));
    assert!(out.contains("CODEX_SECRET_DIR=/tmp/secrets\n"));
    assert!(out.contains("CODEX_AUTH_FILE=/tmp/auth.json\n"));
    assert!(out.contains("CODEX_SECRET_CACHE_DIR=/tmp/cache/secrets\n"));
    assert!(out.contains("CODEX_PROMPT_SEGMENT_ENABLED=true\n"));
    assert!(out.contains("CODEX_AUTO_REFRESH_ENABLED=true\n"));
    assert!(out.contains("CODEX_AUTO_REFRESH_MIN_DAYS=9\n"));
    assert!(out.contains("CODEX_AUTH_REMOTE_SSH=g14\n"));
    assert!(out.contains("CODEX_AUTH_REMOTE_NAME=gamania\n"));
    assert!(out.contains("CODEX_AUTH_REMOTE_REFRESH=false\n"));
}

#[test]
fn config_show_prints_blank_paths_when_unresolvable() {
    let options = CmdOptions::default()
        .with_env_remove("HOME")
        .with_env_remove("ZDOTDIR")
        .with_env_remove("ZSH_SCRIPT_DIR")
        .with_env_remove("_ZSH_BOOTSTRAP_PRELOAD_PATH")
        .with_env_remove("ZSH_CACHE_DIR")
        .with_env_remove("CODEX_SECRET_DIR")
        .with_env_remove("CODEX_AUTH_FILE")
        .with_env_remove("CODEX_SECRET_CACHE_DIR");
    let bin = codex_cli_bin();
    let output = cmd::run_with(&bin, &["config", "show"], &options);
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
fn config_set_model_persist_writes_config_and_respects_environment_override() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_home = dir.path().to_string_lossy().to_string();
    std::fs::create_dir_all(dir.path().join("codex-cli")).expect("config directory");
    std::fs::write(
        dir.path().join("codex-cli/config.toml"),
        "[future]\nkeep = true\n",
    )
    .expect("existing config");
    let options = CmdOptions::default()
        .with_env("XDG_CONFIG_HOME", &config_home)
        .with_env_remove("CODEX_CLI_MODEL");
    let output = cmd::run_with(
        &codex_cli_bin(),
        &["config", "set", "model", "gpt-6-luna", "--persist"],
        &options,
    );
    assert_exit(&output, 0);
    let config = std::fs::read_to_string(dir.path().join("codex-cli/config.toml"))
        .expect("persisted config");
    assert!(config.contains("model = \"gpt-6-luna\""));
    assert!(config.contains("keep = true"));

    let shown = cmd::run_with(&codex_cli_bin(), &["config", "show"], &options);
    assert_exit(&shown, 0);
    assert!(stdout(&shown).contains("CODEX_CLI_MODEL=gpt-6-luna\n"));

    let overridden = cmd::run_with(
        &codex_cli_bin(),
        &["config", "show"],
        &options.with_env("CODEX_CLI_MODEL", "one-shot-model"),
    );
    assert_exit(&overridden, 0);
    assert!(stdout(&overridden).contains("CODEX_CLI_MODEL=one-shot-model\n"));
}

#[test]
fn config_show_warns_when_persisted_model_is_invalid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_dir = dir.path().join("codex-cli");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(config_dir.join("config.toml"), "model = 123\n").expect("invalid model config");
    let options = CmdOptions::default()
        .with_env("XDG_CONFIG_HOME", &dir.path().to_string_lossy())
        .with_env_remove("CODEX_CLI_MODEL");
    let output = cmd::run_with(&codex_cli_bin(), &["config", "show"], &options);
    assert_exit(&output, 0);
    assert!(stdout(&output).contains("CODEX_CLI_MODEL=gpt-6-luna\n"));
    assert!(stderr(&output).contains("model must be a string"));

    std::fs::write(config_dir.join("config.toml"), "model = [\n").expect("malformed config");
    let malformed = cmd::run_with(&codex_cli_bin(), &["config", "show"], &options);
    assert_exit(&malformed, 0);
    assert!(stderr(&malformed).contains("invalid"));
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
fn config_set_ephemeral_prints_export_for_true() {
    let output = run(&["config", "set", "ephemeral", "true"], &[]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "export CODEX_CLI_EPHEMERAL_ENABLED=true\n");
}

#[test]
fn config_set_remote_ssh_prints_export() {
    let output = run(&["config", "set", "remote-ssh", "g14"], &[]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "export CODEX_AUTH_REMOTE_SSH='g14'\n");
}

#[test]
fn config_set_remote_name_prints_export() {
    let output = run(&["config", "set", "remote-name", "gamania"], &[]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "export CODEX_AUTH_REMOTE_NAME='gamania'\n");
}

#[test]
fn config_set_remote_refresh_prints_export_for_false() {
    let output = run(&["config", "set", "remote-refresh", "false"], &[]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "export CODEX_AUTH_REMOTE_REFRESH=false\n");
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

#[test]
fn config_set_ephemeral_rejects_invalid_values() {
    let output = run(&["config", "set", "ephemeral", "maybe"], &[]);
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("ephemeral must be true|false"));
}
