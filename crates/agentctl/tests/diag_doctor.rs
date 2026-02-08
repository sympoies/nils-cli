use agentctl::diag::{classify_hint_category, FailureHintCategory};
use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::{prepend_path, EnvGuard, GlobalStateLock, StubBinDir};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::path::PathBuf;

fn agentctl_bin() -> PathBuf {
    bin::resolve("agentctl")
}

fn run(args: &[&str]) -> CmdOutput {
    let bin = agentctl_bin();
    cmd::run_with(&bin, args, &CmdOptions::default())
}

fn install_stub_tools(stub: &StubBinDir) {
    stub.write_exe("codex", "#!/bin/sh\nexit 0\n");
    stub.write_exe(
        "macos-agent",
        "#!/bin/sh\necho '{\"ok\":true,\"result\":{\"checks\":[{\"id\":\"accessibility\",\"status\":\"ok\",\"blocking\":true}]}}'\n",
    );
    stub.write_exe("screen-record", "#!/bin/sh\necho 'preflight ok'\n");
    stub.write_exe(
        "image-processing",
        "#!/bin/sh\necho 'image-processing help'\n",
    );
    stub.write_exe("fzf-cli", "#!/bin/sh\necho 'fzf-cli help'\n");
}

fn has_check(checks: &[Value], component: &str, subject: &str) -> bool {
    checks.iter().any(|check| {
        check.get("component").and_then(Value::as_str) == Some(component)
            && check.get("subject").and_then(Value::as_str) == Some(subject)
    })
}

#[test]
fn diag_doctor_json_includes_provider_and_automation_readiness() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    install_stub_tools(&stub);
    let auth_file = stub.path().join("auth.json");
    std::fs::write(&auth_file, "{}").expect("write auth file");
    let auth_file_str = auth_file.to_string_lossy().to_string();

    let _path = prepend_path(&lock, stub.path());
    let _dangerous = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");
    let _auth_file = EnvGuard::set(&lock, "CODEX_AUTH_FILE", &auth_file_str);
    let _macos_test_mode = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
    let _screen_record_test_mode = EnvGuard::set(&lock, "CODEX_SCREEN_RECORD_TEST_MODE", "1");

    let output = run(&["diag", "doctor", "--format", "json"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let parsed: Value = serde_json::from_str(&output.stdout_text()).expect("doctor json");
    assert_eq!(parsed["schema_version"], "agentctl.diag.v1");
    assert_eq!(parsed["command"], "doctor");
    assert_eq!(parsed["probe_mode"], "test");
    let checks = parsed
        .pointer("/readiness/checks")
        .and_then(Value::as_array)
        .expect("checks array");
    assert!(has_check(checks, "provider", "codex"));
    for tool in [
        "macos-agent",
        "screen-record",
        "image-processing",
        "fzf-cli",
    ] {
        assert!(
            has_check(checks, "automation", tool),
            "missing check for {tool}"
        );
    }
}

#[test]
fn diag_doctor_unknown_provider_exits_usage() {
    let output = run(&["diag", "doctor", "--provider", "missing-provider"]);
    assert_eq!(output.code, 64);
    assert!(output.stderr_text().contains("unknown provider"));
}

#[test]
fn diag_hint_classification_distinguishes_failure_categories() {
    assert_eq!(
        classify_hint_category("permission denied by tcc"),
        FailureHintCategory::Permission
    );
    assert_eq!(
        classify_hint_category("unsupported platform: only supported on macos"),
        FailureHintCategory::PlatformLimitation
    );
    assert_eq!(
        classify_hint_category("command not found in PATH"),
        FailureHintCategory::MissingDependency
    );
}
