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
            ("CODEX_CLI_MODEL", "model-a"),
            ("CODEX_CLI_REASONING", "low"),
            ("CODEX_CLI_EPHEMERAL_ENABLED", "true"),
            ("CODEX_ALLOW_DANGEROUS_ENABLED", "true"),
            ("CODEX_SECRET_DIR", "/tmp/secrets"),
            ("CODEX_AUTH_FILE", "/tmp/auth.json"),
            ("CODEX_SECRET_CACHE_DIR", "/tmp/cache/secrets"),
            ("CODEX_PROMPT_SEGMENT_ENABLED", "true"),
            ("CODEX_AUTO_REFRESH_ENABLED", "true"),
            ("CODEX_AUTO_REFRESH_MIN_DAYS", "9"),
            ("CODEX_AUTH_REMOTE_SSH", "host-a"),
            ("CODEX_AUTH_REMOTE_NAME", "acct1"),
            ("CODEX_AUTH_REMOTE_REFRESH", "false"),
        ],
    );
    assert_exit(&output, 0);
    let out = stdout(&output);
    assert!(out.contains("CODEX_CLI_MODEL=model-a\n"));
    assert!(out.contains("CODEX_CLI_REASONING=low\n"));
    assert!(out.contains("CODEX_CLI_EPHEMERAL_ENABLED=true\n"));
    assert!(out.contains("CODEX_ALLOW_DANGEROUS_ENABLED=true\n"));
    assert!(out.contains("CODEX_SECRET_DIR=/tmp/secrets\n"));
    assert!(out.contains("CODEX_AUTH_FILE=/tmp/auth.json\n"));
    assert!(out.contains("CODEX_SECRET_CACHE_DIR=/tmp/cache/secrets\n"));
    assert!(out.contains("CODEX_PROMPT_SEGMENT_ENABLED=true\n"));
    assert!(out.contains("CODEX_AUTO_REFRESH_ENABLED=true\n"));
    assert!(out.contains("CODEX_AUTO_REFRESH_MIN_DAYS=9\n"));
    assert!(out.contains("CODEX_AUTH_REMOTE_SSH=host-a\n"));
    assert!(out.contains("CODEX_AUTH_REMOTE_NAME=acct1\n"));
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
    let output = run(&["config", "set", "remote-ssh", "host-a"], &[]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "export CODEX_AUTH_REMOTE_SSH='host-a'\n");
}

#[test]
fn config_set_remote_name_prints_export() {
    let output = run(&["config", "set", "remote-name", "acct1"], &[]);
    assert_exit(&output, 0);
    assert_eq!(stdout(&output), "export CODEX_AUTH_REMOTE_NAME='acct1'\n");
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

fn persist_rate_limit_notice(home: &std::path::Path) -> CmdOutput {
    run(
        &[
            "config",
            "set",
            "notice.hide_rate_limit_model_nudge",
            "true",
            "--persist",
        ],
        &[("CODEX_HOME", home.to_str().expect("fixture home"))],
    )
}

#[test]
fn config_persist_rate_limit_notice_creates_only_the_selected_home() {
    let root = tempfile::tempdir().expect("tempdir");
    let selected = root.path().join("selected");
    let other = root.path().join("other");
    std::fs::create_dir(&other).expect("other home");
    let untouched = "[notice]\nhide_rate_limit_model_nudge = false\n";
    std::fs::write(other.join("config.toml"), untouched).expect("other config");

    let output = persist_rate_limit_notice(&selected);
    assert_exit(&output, 0);
    let config = std::fs::read_to_string(selected.join("config.toml")).expect("selected config");
    let config = config.parse::<toml::Table>().expect("valid TOML");
    assert_eq!(
        config["notice"]["hide_rate_limit_model_nudge"].as_bool(),
        Some(true)
    );
    assert_eq!(
        std::fs::read_to_string(other.join("config.toml")).expect("other config"),
        untouched
    );
}

#[test]
fn config_persist_rate_limit_notice_preserves_unrelated_toml_and_is_idempotent() {
    let home = tempfile::tempdir().expect("tempdir");
    let path = home.path().join("config.toml");
    let prefix = "# User settings\nmodel   = 'example-model' # main\n\n[notice] # keep header\nother  = true\n";
    let suffix = "\n[features]\nexample = false # keep\n";
    for (before, after) in [
        ("", "hide_rate_limit_model_nudge = true\n"),
        (
            "hide_rate_limit_model_nudge  =   false # keep\n",
            "hide_rate_limit_model_nudge  =   true # keep\n",
        ),
        (
            "hide_rate_limit_model_nudge  =   true # keep\n",
            "hide_rate_limit_model_nudge  =   true # keep\n",
        ),
    ] {
        std::fs::write(&path, format!("{prefix}{before}{suffix}")).expect("fixture config");
        for _ in 0..2 {
            let output = persist_rate_limit_notice(home.path());
            assert_exit(&output, 0);
            assert_eq!(
                std::fs::read_to_string(&path).expect("saved config"),
                format!("{prefix}{after}{suffix}")
            );
        }
    }
}

#[test]
fn config_persist_rate_limit_notice_preserves_crlf_and_inline_table() {
    let home = tempfile::tempdir().expect("tempdir");
    let path = home.path().join("config.toml");
    let before = "# Preferences\r\nnotice = { other = true, hide_rate_limit_model_nudge = false }";
    std::fs::write(&path, before).expect("fixture config");

    assert_exit(&persist_rate_limit_notice(home.path()), 0);
    assert_eq!(
        std::fs::read_to_string(&path).expect("saved config"),
        before.replace("false", "true")
    );
}

#[test]
fn config_persist_rate_limit_notice_preserves_interleaved_dotted_keys() {
    let home = tempfile::tempdir().expect("tempdir");
    let path = home.path().join("config.toml");
    let before =
        "notice.first = true # first\nmodel = 'example' # model\nnotice.second = true # second\n";
    let setting = "notice.hide_rate_limit_model_nudge = true\n";
    for initial in [
        before.to_string(),
        format!("{before}notice.hide_rate_limit_model_nudge = false\n"),
    ] {
        std::fs::write(&path, initial).expect("fixture config");

        assert_exit(&persist_rate_limit_notice(home.path()), 0);
        assert_eq!(
            std::fs::read_to_string(&path).expect("saved config"),
            format!("{before}{setting}")
        );
    }
}

#[test]
fn config_persist_rate_limit_notice_reports_malformed_config_without_overwriting() {
    let home = tempfile::tempdir().expect("tempdir");
    let path = home.path().join("config.toml");
    let malformed = "[notice\nhide_rate_limit_model_nudge = false\n";
    std::fs::write(&path, malformed).expect("fixture config");

    let output = persist_rate_limit_notice(home.path());
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("invalid") && stderr(&output).contains("config.toml"));
    assert_eq!(
        std::fs::read_to_string(&path).expect("original config"),
        malformed
    );
}
