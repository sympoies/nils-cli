use tempfile::TempDir;

mod common;

#[test]
fn scenario_chain_activate_wait_click_type_and_observe_succeeds() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let screenshot_path = cwd.path().join("scenario-chain.png");
    let screenshot_path_text = screenshot_path.to_string_lossy().to_string();

    let activate_out = harness.run(
        cwd.path(),
        &[
            "--format",
            "json",
            "--retries",
            "1",
            "window",
            "activate",
            "--app",
            "Terminal",
            "--wait-ms",
            "25",
        ],
    );
    assert_eq!(
        activate_out.code,
        0,
        "stderr: {}",
        activate_out.stderr_text()
    );
    let activate_payload: serde_json::Value =
        serde_json::from_str(&activate_out.stdout_text()).expect("window.activate json");
    assert_eq!(
        activate_payload["command"],
        serde_json::json!("window.activate")
    );
    assert_eq!(
        activate_payload["result"]["selected_app"],
        serde_json::json!("Terminal")
    );
    assert_eq!(
        activate_payload["result"]["policy"]["retries"],
        serde_json::json!(1)
    );

    let wait_out = harness.run(
        cwd.path(),
        &[
            "--format",
            "json",
            "wait",
            "app-active",
            "--app",
            "Terminal",
            "--timeout-ms",
            "250",
            "--poll-ms",
            "10",
        ],
    );
    assert_eq!(wait_out.code, 0, "stderr: {}", wait_out.stderr_text());
    let wait_payload: serde_json::Value =
        serde_json::from_str(&wait_out.stdout_text()).expect("wait json");
    assert_eq!(
        wait_payload["command"],
        serde_json::json!("wait.app-active")
    );

    let click_out = harness.run(
        cwd.path(),
        &[
            "--format",
            "json",
            "--retries",
            "1",
            "input",
            "click",
            "--x",
            "200",
            "--y",
            "160",
            "--count",
            "1",
            "--pre-wait-ms",
            "1",
            "--post-wait-ms",
            "1",
        ],
    );
    assert_eq!(click_out.code, 0, "stderr: {}", click_out.stderr_text());
    let click_payload: serde_json::Value =
        serde_json::from_str(&click_out.stdout_text()).expect("input.click json");
    assert_eq!(click_payload["command"], serde_json::json!("input.click"));
    assert_eq!(
        click_payload["result"]["policy"]["retries"],
        serde_json::json!(1)
    );

    let type_out = harness.run(
        cwd.path(),
        &["--format", "json", "input", "type", "--text", "hello world"],
    );
    assert_eq!(type_out.code, 0, "stderr: {}", type_out.stderr_text());
    let type_payload: serde_json::Value =
        serde_json::from_str(&type_out.stdout_text()).expect("input.type json");
    assert_eq!(type_payload["command"], serde_json::json!("input.type"));
    assert_eq!(type_payload["result"]["text_length"], serde_json::json!(11));

    let observe_out = harness.run(
        cwd.path(),
        &[
            "--format",
            "json",
            "observe",
            "screenshot",
            "--active-window",
            "--path",
            &screenshot_path_text,
        ],
    );
    assert_eq!(observe_out.code, 0, "stderr: {}", observe_out.stderr_text());
    let observe_payload: serde_json::Value =
        serde_json::from_str(&observe_out.stdout_text()).expect("observe json");
    assert_eq!(
        observe_payload["command"],
        serde_json::json!("observe.screenshot")
    );
    assert_eq!(
        observe_payload["result"]["path"],
        serde_json::json!(screenshot_path_text)
    );
    assert!(
        screenshot_path.is_file(),
        "scenario screenshot should exist"
    );
}

#[test]
fn scenario_run_executes_fixture_steps() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("scenario-basic.json");

    let out = harness.run(
        cwd.path(),
        &[
            "--format",
            "json",
            "scenario",
            "run",
            "--file",
            fixture.to_str().unwrap(),
        ],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());
    let payload: serde_json::Value =
        serde_json::from_str(&out.stdout_text()).expect("scenario run json");
    assert_eq!(payload["command"], serde_json::json!("scenario.run"));
    assert_eq!(payload["result"]["failed_steps"], serde_json::json!(0));
    assert!(payload["result"]["passed_steps"].as_u64().unwrap_or(0) >= 3);
}
