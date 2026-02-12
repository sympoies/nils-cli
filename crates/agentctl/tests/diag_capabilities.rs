use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
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

#[test]
fn diag_capabilities_json_reports_inventory_and_readiness() {
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

    let output = run(&[
        "diag",
        "capabilities",
        "--format",
        "json",
        "--include-experimental",
    ]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());

    let parsed: Value = serde_json::from_str(&output.stdout_text()).expect("capabilities json");
    assert_eq!(parsed["schema_version"], "agentctl.diag.v1");
    assert_eq!(parsed["command"], "capabilities");
    assert_eq!(parsed["probe_mode"], "test");
    assert!(parsed.pointer("/readiness/checks").is_some());

    let providers = parsed
        .get("providers")
        .and_then(Value::as_array)
        .expect("providers");
    assert!(!providers.is_empty());
    let codex_provider = providers
        .iter()
        .find(|provider| provider.get("id").and_then(Value::as_str) == Some("codex"))
        .expect("codex provider");
    let capabilities = codex_provider
        .get("capabilities")
        .and_then(Value::as_array)
        .expect("capabilities");
    assert!(
        capabilities
            .iter()
            .any(|capability| capability.get("name").and_then(Value::as_str) == Some("execute"))
    );
    assert!(capabilities.iter().any(
        |capability| capability.get("name").and_then(Value::as_str) == Some("diag.rate-limits")
    ));
    assert!(
        capabilities
            .iter()
            .any(|capability| capability.get("name").and_then(Value::as_str)
                == Some("auth.commands"))
    );

    let tools = parsed
        .get("automation_tools")
        .and_then(Value::as_array)
        .expect("automation tools");
    for id in [
        "macos-agent",
        "screen-record",
        "image-processing",
        "fzf-cli",
    ] {
        assert!(
            tools
                .iter()
                .any(|tool| tool.get("id").and_then(Value::as_str) == Some(id)),
            "missing automation tool {id}"
        );
    }

    let image_processing = tools
        .iter()
        .find(|tool| tool.get("id").and_then(Value::as_str) == Some("image-processing"))
        .expect("image-processing automation tool");
    let image_processing_capabilities = image_processing
        .get("capabilities")
        .and_then(Value::as_array)
        .expect("image-processing capabilities");
    assert!(
        image_processing_capabilities
            .iter()
            .any(|capability| capability.as_str() == Some("svg-validate"))
    );
    assert!(
        image_processing_capabilities
            .iter()
            .any(|capability| capability.as_str() == Some("convert.from-svg"))
    );
}

#[test]
fn diag_capabilities_unknown_provider_exits_usage() {
    let output = run(&["diag", "capabilities", "--provider", "missing-provider"]);
    assert_eq!(output.code, 64);
    assert!(output.stderr_text().contains("unknown provider"));
}

#[test]
fn diag_capabilities_text_output_includes_provider_and_automation_sections() {
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

    let output = run(&["diag", "capabilities", "--format", "text"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let stdout = output.stdout_text();
    assert!(stdout.contains("probe_mode: test"));
    assert!(stdout.contains("providers:"));
    assert!(stdout.contains("automation_tools:"));
}
