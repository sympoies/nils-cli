use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::path::PathBuf;

fn claude_cli_bin() -> PathBuf {
    bin::resolve("claude-cli")
}

fn run_with(args: &[&str], options: CmdOptions) -> CmdOutput {
    let bin = claude_cli_bin();
    cmd::run_with(&bin, args, &options)
}

#[test]
fn auth_state_json_reports_authenticated_and_unauthenticated_states() {
    let unauth = run_with(
        &["auth-state", "show", "--format", "json"],
        CmdOptions::default().with_env_remove("ANTHROPIC_API_KEY"),
    );
    assert_eq!(unauth.code, 0, "stderr={}", unauth.stderr_text());
    let unauth_json: Value = serde_json::from_str(&unauth.stdout_text()).expect("json");
    assert_eq!(unauth_json["schema_version"], "claude-cli.auth-state.v1");
    assert_eq!(unauth_json["command"], "auth-state show");
    assert_eq!(unauth_json["ok"], true);
    assert_eq!(unauth_json["result"]["state"], "unauthenticated");

    let auth = run_with(
        &["auth-state", "show", "--json"],
        CmdOptions::default()
            .with_env("ANTHROPIC_API_KEY", "test-key-1234")
            .with_env("ANTHROPIC_AUTH_SUBJECT", "claude-user@example.com")
            .with_env("ANTHROPIC_AUTH_SCOPES", "messages:read,messages:write"),
    );
    assert_eq!(auth.code, 0, "stderr={}", auth.stderr_text());
    let auth_json: Value = serde_json::from_str(&auth.stdout_text()).expect("json");
    assert_eq!(auth_json["result"]["state"], "authenticated");
    assert_eq!(auth_json["result"]["subject"], "claude-user@example.com");
    assert_eq!(auth_json["result"]["scopes"][0], "messages:read");
    assert_eq!(auth_json["result"]["scopes"][1], "messages:write");
}

#[test]
fn diag_healthcheck_and_unsupported_rate_limits_are_deterministic() {
    let health = run_with(
        &["diag", "healthcheck", "--format", "json"],
        CmdOptions::default().with_env_remove("ANTHROPIC_API_KEY"),
    );
    assert_eq!(health.code, 0, "stderr={}", health.stderr_text());
    let health_json: Value = serde_json::from_str(&health.stdout_text()).expect("json");
    assert_eq!(health_json["schema_version"], "claude-cli.diag.v1");
    assert_eq!(health_json["command"], "diag healthcheck");
    assert_eq!(health_json["ok"], true);
    assert_eq!(health_json["result"]["provider"], "claude");
    assert_eq!(health_json["result"]["status"], "degraded");

    let unsupported = run_with(&["diag", "rate-limits", "--json"], CmdOptions::default());
    assert_eq!(unsupported.code, 64);
    assert!(
        unsupported
            .stderr_text()
            .contains("unsupported-codex-only-command")
            || unsupported.stderr_text().contains("codex-only")
    );
    let unsupported_json: Value = serde_json::from_str(&unsupported.stdout_text()).expect("json");
    assert_eq!(unsupported_json["ok"], false);
    assert_eq!(
        unsupported_json["error"]["code"],
        "unsupported-codex-only-command"
    );
}

#[test]
fn config_show_and_set_surfaces_are_deterministic() {
    let show = run_with(
        &["config", "show", "--format", "json"],
        CmdOptions::default()
            .with_env("CLAUDE_MODEL", "claude-test-model")
            .with_env("CLAUDE_TIMEOUT_MS", "45000"),
    );
    assert_eq!(show.code, 0, "stderr={}", show.stderr_text());
    let show_json: Value = serde_json::from_str(&show.stdout_text()).expect("json");
    assert_eq!(show_json["schema_version"], "claude-cli.config.v1");
    assert_eq!(show_json["result"]["model"], "claude-test-model");
    assert_eq!(show_json["result"]["timeout_ms"], 45000);

    let set_model = run_with(
        &["config", "set", "model", "claude-3-7"],
        CmdOptions::default(),
    );
    assert_eq!(set_model.code, 0, "stderr={}", set_model.stderr_text());
    assert!(
        set_model
            .stdout_text()
            .contains("export CLAUDE_MODEL='claude-3-7'")
    );

    let unknown = run_with(
        &["config", "set", "missing", "value"],
        CmdOptions::default(),
    );
    assert_eq!(unknown.code, 64);
    assert!(unknown.stderr_text().contains("unknown key"));
}

#[test]
fn codex_only_legacy_commands_return_stable_guidance() {
    let starship = run_with(&["starship"], CmdOptions::default());
    assert_eq!(starship.code, 64);
    assert!(starship.stderr_text().contains("codex-only"));

    let auth = run_with(&["auth"], CmdOptions::default());
    assert_eq!(auth.code, 64);
    assert!(auth.stderr_text().contains("auth-state show"));
}
