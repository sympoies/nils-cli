use tempfile::TempDir;

mod common;

#[test]
fn input_click_double_click_succeeds() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(
        cwd.path(),
        &[
            "input",
            "click",
            "--x",
            "200",
            "--y",
            "160",
            "--count",
            "2",
            "--pre-wait-ms",
            "1",
            "--post-wait-ms",
            "1",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());
    assert_eq!(out.stderr_text(), "");
    assert!(out.stdout_text().contains("input.click"));
}

#[test]
fn input_click_invalid_count_is_usage_error() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(
        cwd.path(),
        &["input", "click", "--x", "1", "--y", "2", "--count", "0"],
    );

    assert_eq!(out.code, 2);
    assert_eq!(out.stdout_text(), "");
    assert!(out.stderr_text().starts_with("error:"));
}

#[test]
fn input_click_runtime_error_from_cliclick_is_concise() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let options = harness
        .cmd_options(cwd.path())
        .with_env("CODEX_MACOS_AGENT_STUB_CLICLICK_MODE", "fail");
    let out = harness.run_with_options(
        cwd.path(),
        &["input", "click", "--x", "10", "--y", "10"],
        options,
    );

    assert_eq!(out.code, 1);
    assert_eq!(out.stdout_text(), "");
    assert!(out
        .stderr_text()
        .contains("input.click failed via `cliclick`"));
}

#[test]
fn input_click_dry_run_does_not_execute_backend() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let options = harness
        .cmd_options(cwd.path())
        .with_env("CODEX_MACOS_AGENT_STUB_CLICLICK_MODE", "fail");
    let out = harness.run_with_options(
        cwd.path(),
        &[
            "--dry-run",
            "input",
            "click",
            "--x",
            "10",
            "--y",
            "10",
            "--format",
            "json",
        ],
        options,
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());
    let payload: serde_json::Value =
        serde_json::from_str(&out.stdout_text()).expect("stdout should be json");
    assert_eq!(
        payload["result"]["policy"]["dry_run"],
        serde_json::json!(true)
    );
    assert_eq!(payload["result"]["policy"]["retries"], serde_json::json!(0));
    assert_eq!(
        payload["result"]["policy"]["retry_delay_ms"],
        serde_json::json!(150)
    );
    assert_eq!(
        payload["result"]["policy"]["timeout_ms"],
        serde_json::json!(4000)
    );
    assert_eq!(
        payload["result"]["meta"]["dry_run"],
        serde_json::json!(true)
    );
}
