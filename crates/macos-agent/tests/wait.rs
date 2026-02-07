use tempfile::TempDir;

mod common;

#[test]
fn wait_sleep_returns_single_result_line() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(cwd.path(), &["wait", "sleep", "--ms", "1"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());
    assert_eq!(out.stderr_text(), "");
    assert!(out.stdout_text().starts_with("wait.sleep\t"));
}

#[test]
fn wait_app_active_succeeds_for_terminal_fixture() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(
        cwd.path(),
        &[
            "wait",
            "app-active",
            "--app",
            "Terminal",
            "--timeout-ms",
            "50",
            "--poll-ms",
            "5",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());
    assert_eq!(out.stderr_text(), "");
}

#[test]
fn wait_app_active_timeout_is_runtime_error() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(
        cwd.path(),
        &[
            "wait",
            "app-active",
            "--app",
            "Finder",
            "--timeout-ms",
            "20",
            "--poll-ms",
            "5",
        ],
    );

    assert_eq!(out.code, 1);
    assert_eq!(out.stdout_text(), "");
    assert!(out
        .stderr_text()
        .contains("timed out waiting for app-active"));
}

#[test]
fn wait_window_present_succeeds_for_terminal() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(
        cwd.path(),
        &[
            "wait",
            "window-present",
            "--app",
            "Terminal",
            "--window-name",
            "Docs",
            "--timeout-ms",
            "50",
            "--poll-ms",
            "5",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());
    assert_eq!(out.stderr_text(), "");
}

#[test]
fn wait_json_and_tsv_contracts() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let json_out = harness.run(
        cwd.path(),
        &[
            "--format",
            "json",
            "wait",
            "app-active",
            "--app",
            "Terminal",
            "--timeout-ms",
            "50",
            "--poll-ms",
            "5",
        ],
    );
    assert_eq!(json_out.code, 0, "stderr: {}", json_out.stderr_text());
    let payload: serde_json::Value =
        serde_json::from_str(&json_out.stdout_text()).expect("json payload");
    assert_eq!(payload["command"], serde_json::json!("wait.app-active"));

    let tsv_out = harness.run(
        cwd.path(),
        &["--format", "tsv", "wait", "app-active", "--app", "Terminal"],
    );
    assert_eq!(tsv_out.code, 2);
    assert_eq!(tsv_out.stdout_text(), "");
    assert!(tsv_out
        .stderr_text()
        .contains("only supported for `windows list` and `apps list`"));
}
