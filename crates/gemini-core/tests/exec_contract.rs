use gemini_core::exec;
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
use pretty_assertions::assert_eq;

#[test]
fn exec_contract_missing_prompt_is_rejected() {
    let mut stderr = Vec::new();
    let code = exec::exec_dangerous("", "caller", &mut stderr);

    assert_eq!(code, 1);
    assert!(String::from_utf8_lossy(&stderr).contains("_gemini_exec_dangerous: missing prompt"));
}

#[test]
fn exec_contract_disabled_policy_reports_validation_message() {
    let lock = GlobalStateLock::new();
    let _danger = EnvGuard::set(&lock, "GEMINI_ALLOW_DANGEROUS_ENABLED", "false");

    let (enabled, message) = exec::allow_dangerous_status(Some("adapter"));
    assert!(!enabled);
    assert!(
        message
            .expect("message")
            .contains("adapter: disabled (set GEMINI_ALLOW_DANGEROUS_ENABLED=true)")
    );

    let check = exec::check_allow_dangerous(Some("adapter")).expect_err("disabled policy error");
    assert_eq!(check.code, "disabled-policy");
    assert_eq!(check.retryable, false);
}

#[test]
fn exec_contract_missing_binary_fails_without_panicking() {
    let lock = GlobalStateLock::new();
    let _danger = EnvGuard::set(&lock, "GEMINI_ALLOW_DANGEROUS_ENABLED", "true");
    let _path = EnvGuard::set(&lock, "PATH", "");

    let mut stderr = Vec::new();
    let code = exec::exec_dangerous("ping", "caller", &mut stderr);

    assert_eq!(code, 1);
    assert!(String::from_utf8_lossy(&stderr).contains("failed to run gemini exec"));
}

#[test]
fn exec_contract_success_path_uses_expected_command_shape() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    let args_log = tempfile::NamedTempFile::new().expect("args log");
    let args_log_path = args_log.path().to_string_lossy().to_string();

    stub.write_exe(
        "gemini",
        r#"#!/bin/bash
set -euo pipefail
out="${GEMINI_TEST_ARGV_LOG:?missing GEMINI_TEST_ARGV_LOG}"
: > "$out"
for a in "$@"; do
  echo "$a" >> "$out"
done
"#,
    );

    let _path = prepend_path(&lock, stub.path());
    let _danger = EnvGuard::set(&lock, "GEMINI_ALLOW_DANGEROUS_ENABLED", "true");
    let _model = EnvGuard::set(&lock, "GEMINI_CLI_MODEL", "gemini-test");
    let _reason = EnvGuard::set(&lock, "GEMINI_CLI_REASONING", "high");
    let _argv_log = EnvGuard::set(&lock, "GEMINI_TEST_ARGV_LOG", &args_log_path);

    let mut stderr = Vec::new();
    let code = exec::exec_dangerous("hello world", "caller", &mut stderr);

    assert_eq!(code, 0);
    assert!(stderr.is_empty());

    let args = std::fs::read_to_string(args_log.path())
        .expect("read args")
        .lines()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        args,
        vec![
            "exec",
            "--dangerously-bypass-approvals-and-sandbox",
            "-s",
            "workspace-write",
            "-m",
            "gemini-test",
            "-c",
            "model_reasoning_effort=\"high\"",
            "--",
            "hello world",
        ]
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
    );
}
