use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::path::PathBuf;

fn agentctl_bin() -> PathBuf {
    bin::resolve("agentctl")
}

fn run_with(args: &[&str], options: CmdOptions) -> CmdOutput {
    let bin = agentctl_bin();
    cmd::run_with(&bin, args, &options)
}

#[test]
fn provider_list_json_reports_builtin_providers_and_maturity() {
    let output = run_with(
        &["provider", "list", "--format", "json"],
        CmdOptions::default().with_env_remove("AGENTCTL_PROVIDER"),
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let parsed: Value = serde_json::from_str(&output.stdout_text()).expect("provider list json");
    assert_eq!(parsed["default_provider"], "codex");
    assert_eq!(parsed["selected_provider"], "codex");
    assert_eq!(parsed["selected_source"], "default");

    let providers = parsed
        .get("providers")
        .and_then(Value::as_array)
        .expect("providers array");

    let expected = [
        ("codex", "stable", true),
        ("claude", "stable", false),
        ("gemini", "stub", false),
    ];

    for (provider_id, maturity, is_default) in expected {
        let provider = providers
            .iter()
            .find(|provider| provider.get("id").and_then(Value::as_str) == Some(provider_id))
            .expect("provider should exist");

        assert_eq!(provider["contract_version"], "provider-adapter.v1");
        assert_eq!(provider["maturity"], maturity);
        assert_eq!(provider["is_default"], is_default);
        assert!(
            provider.get("status").is_some(),
            "provider status should exist"
        );
    }
}

#[test]
fn provider_healthcheck_json_supports_claude_provider_selection() {
    let output = run_with(
        &[
            "provider",
            "healthcheck",
            "--provider",
            "claude",
            "--format",
            "json",
        ],
        CmdOptions::default().with_env_remove("AGENTCTL_PROVIDER"),
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let parsed: Value =
        serde_json::from_str(&output.stdout_text()).expect("provider healthcheck json");
    assert_eq!(parsed["provider"], "claude");
    assert_eq!(parsed["selected_source"], "cli-argument");
    assert_eq!(parsed["status"], "degraded");
    assert!(
        parsed["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("partially ready")
    );
}

#[test]
fn provider_list_unknown_override_exits_usage() {
    let output = run_with(
        &["provider", "list", "--provider", "missing"],
        CmdOptions::default().with_env_remove("AGENTCTL_PROVIDER"),
    );

    assert_eq!(output.code, 64);
    assert!(output.stderr_text().contains("unknown provider"));
}

#[test]
fn provider_list_text_respects_environment_override_and_prints_maturity() {
    let lock = GlobalStateLock::new();
    let _provider = EnvGuard::set(&lock, "AGENTCTL_PROVIDER", "claude");

    let output = run_with(&["provider", "list"], CmdOptions::default());
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let stdout = output.stdout_text();
    assert!(stdout.contains("selected_provider: claude (environment)"));
    assert!(stdout.contains("maturity: stable"));
}

#[test]
fn provider_healthcheck_text_reports_selected_source_and_status() {
    let output = run_with(
        &["provider", "healthcheck", "--provider", "gemini"],
        CmdOptions::default().with_env_remove("AGENTCTL_PROVIDER"),
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let stdout = output.stdout_text();
    assert!(stdout.contains("provider: gemini"));
    assert!(stdout.contains("selected_source: cli-argument"));
    assert!(stdout.contains("status: degraded"));
}

#[test]
fn provider_healthcheck_unknown_override_exits_usage() {
    let output = run_with(
        &["provider", "healthcheck", "--provider", "missing"],
        CmdOptions::default().with_env_remove("AGENTCTL_PROVIDER"),
    );

    assert_eq!(output.code, 64);
    assert!(output.stderr_text().contains("unknown provider"));
    assert!(output.stderr_text().contains("available providers:"));
}
