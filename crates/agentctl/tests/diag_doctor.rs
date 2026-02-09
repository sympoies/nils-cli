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

fn check_by_subject<'a>(checks: &'a [Value], subject: &str) -> &'a Value {
    checks
        .iter()
        .find(|check| check.get("subject").and_then(Value::as_str) == Some(subject))
        .expect("check for subject should exist")
}

#[test]
fn diag_doctor_json_includes_provider_and_automation_readiness() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    install_stub_tools(&stub);
    let auth_file = stub.path().join("auth.json");
    std::fs::write(&auth_file, "{}").expect("write auth file");
    let auth_file_str = auth_file.to_string_lossy().to_string();

    let path_only_stub = stub.path().to_string_lossy().to_string();
    let _path = EnvGuard::set(&lock, "PATH", &path_only_stub);
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

#[test]
fn diag_doctor_json_covers_probe_error_missing_binary_and_permission_hint_paths() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("codex", "#!/bin/sh\nexit 0\n");
    stub.write_exe("macos-agent", "#!/definitely/missing/interpreter\n");
    stub.write_exe(
        "screen-record",
        "#!/bin/sh\necho 'permission denied by system policy' 1>&2\nexit 7\n",
    );
    stub.write_exe("fzf-cli", "#!/bin/sh\necho 'fzf-cli help'\n");
    let auth_file = stub.path().join("auth.json");
    std::fs::write(&auth_file, "{}").expect("write auth file");
    let auth_file_str = auth_file.to_string_lossy().to_string();

    let path_only_stub = stub.path().to_string_lossy().to_string();
    let _path = EnvGuard::set(&lock, "PATH", &path_only_stub);
    let _dangerous = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");
    let _auth_file = EnvGuard::set(&lock, "CODEX_AUTH_FILE", &auth_file_str);

    let output = run(&["diag", "doctor", "--format", "json", "--probe-mode", "test"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let parsed: Value = serde_json::from_str(&output.stdout_text()).expect("doctor json");
    let checks = parsed
        .pointer("/readiness/checks")
        .and_then(Value::as_array)
        .expect("checks array");

    let screen_record = check_by_subject(checks, "screen-record");
    assert_eq!(screen_record["status"], "not-ready");
    assert_eq!(
        screen_record
            .pointer("/hint/category")
            .and_then(Value::as_str),
        Some("permission")
    );

    let image_processing = check_by_subject(checks, "image-processing");
    assert_eq!(image_processing["status"], "not-ready");
    assert!(image_processing["summary"]
        .as_str()
        .unwrap_or_default()
        .contains("binary is missing from PATH"));

    let macos_agent = check_by_subject(checks, "macos-agent");
    assert_eq!(macos_agent["status"], "not-ready");
    assert!(macos_agent["summary"]
        .as_str()
        .unwrap_or_default()
        .contains("failed to execute"));
}

#[test]
fn diag_doctor_json_parses_macos_agent_non_ok_preflight_payload() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("codex", "#!/bin/sh\nexit 0\n");
    stub.write_exe(
        "macos-agent",
        "#!/bin/sh\necho '{\"ok\":false,\"result\":{\"checks\":[{\"id\":\"accessibility\",\"status\":\"fail\",\"blocking\":true,\"message\":\"Accessibility permission denied\",\"hint\":\"Enable Accessibility\"}]}}'\n",
    );
    stub.write_exe("screen-record", "#!/bin/sh\necho 'preflight ok'\n");
    stub.write_exe(
        "image-processing",
        "#!/bin/sh\necho 'image-processing help'\n",
    );
    stub.write_exe("fzf-cli", "#!/bin/sh\necho 'fzf-cli help'\n");
    let auth_file = stub.path().join("auth.json");
    std::fs::write(&auth_file, "{}").expect("write auth file");
    let auth_file_str = auth_file.to_string_lossy().to_string();

    let _path = prepend_path(&lock, stub.path());
    let _dangerous = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");
    let _auth_file = EnvGuard::set(&lock, "CODEX_AUTH_FILE", &auth_file_str);

    let output = run(&["diag", "doctor", "--format", "json", "--probe-mode", "test"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let parsed: Value = serde_json::from_str(&output.stdout_text()).expect("doctor json");
    let checks = parsed
        .pointer("/readiness/checks")
        .and_then(Value::as_array)
        .expect("checks array");
    let macos_agent = check_by_subject(checks, "macos-agent");
    assert_eq!(macos_agent["status"], "not-ready");
    assert!(macos_agent["summary"]
        .as_str()
        .unwrap_or_default()
        .contains("Accessibility"));
    assert_eq!(
        macos_agent
            .pointer("/hint/category")
            .and_then(Value::as_str),
        Some("permission")
    );
}

#[test]
fn diag_doctor_live_mode_reports_platform_limitations_on_unsupported_hosts() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    install_stub_tools(&stub);
    let auth_file = stub.path().join("auth.json");
    std::fs::write(&auth_file, "{}").expect("write auth file");
    let auth_file_str = auth_file.to_string_lossy().to_string();

    let _path = prepend_path(&lock, stub.path());
    let _dangerous = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");
    let _auth_file = EnvGuard::set(&lock, "CODEX_AUTH_FILE", &auth_file_str);
    let _macos_test_mode = EnvGuard::remove(&lock, "CODEX_MACOS_AGENT_TEST_MODE");
    let _screen_record_test_mode = EnvGuard::remove(&lock, "CODEX_SCREEN_RECORD_TEST_MODE");

    let output = run(&["diag", "doctor", "--format", "json", "--probe-mode", "live"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let parsed: Value = serde_json::from_str(&output.stdout_text()).expect("doctor json");
    let checks = parsed
        .pointer("/readiness/checks")
        .and_then(Value::as_array)
        .expect("checks array");
    let macos_agent = check_by_subject(checks, "macos-agent");

    if std::env::consts::OS == "macos" {
        assert_eq!(macos_agent["status"], "ready");
    } else {
        assert_eq!(macos_agent["status"], "not-ready");
        assert!(macos_agent["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("not supported on"));
    }
}

#[test]
fn diag_doctor_text_mode_prints_summary_and_checks() {
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

    let output = run(&["diag", "doctor"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let stdout = output.stdout_text();
    assert!(stdout.contains("overall_status:"));
    assert!(stdout.contains("summary: total="));
    assert!(stdout.contains("checks:"));
}
