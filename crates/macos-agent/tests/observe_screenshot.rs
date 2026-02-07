use tempfile::TempDir;

mod common;

#[test]
fn observe_screenshot_writes_file_and_prints_path_line() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let output_path = cwd.path().join("artifact.png");

    let out = harness.run(
        cwd.path(),
        &[
            "observe",
            "screenshot",
            "--active-window",
            "--path",
            output_path.to_str().unwrap(),
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());
    assert_eq!(out.stderr_text(), "");
    assert_eq!(out.stdout_text().trim(), output_path.display().to_string());
    assert!(output_path.exists(), "screenshot output should exist");
    assert!(
        std::fs::metadata(&output_path).expect("metadata").len() > 0,
        "screenshot should be non-empty"
    );
}

#[test]
fn observe_screenshot_json_contract() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(
        cwd.path(),
        &[
            "--format",
            "json",
            "observe",
            "screenshot",
            "--active-window",
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());
    assert_eq!(out.stderr_text(), "");

    let payload: serde_json::Value =
        serde_json::from_str(&out.stdout_text()).expect("stdout should be valid json");
    assert_eq!(payload["schema_version"], serde_json::json!(1));
    assert_eq!(payload["ok"], serde_json::json!(true));
    assert_eq!(payload["command"], serde_json::json!("observe.screenshot"));
    assert!(payload["result"]["path"]
        .as_str()
        .unwrap()
        .contains("window-100"));
}

#[test]
fn observe_screenshot_errors_keep_stdout_empty() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(cwd.path(), &["observe", "screenshot", "--window-id", "999"]);

    assert_eq!(out.code, 2);
    assert_eq!(out.stdout_text(), "");
    assert!(out.stderr_text().starts_with("error:"));
}
