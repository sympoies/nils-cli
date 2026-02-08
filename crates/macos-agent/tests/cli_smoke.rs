use tempfile::TempDir;

mod common;

#[test]
fn help_lists_command_groups() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(cwd.path(), &["--help"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());

    let text = format!("{}{}", out.stdout_text(), out.stderr_text());
    for token in [
        "preflight",
        "windows",
        "apps",
        "window",
        "input",
        "input-source",
        "ax",
        "observe",
        "wait",
        "scenario",
        "profile",
    ] {
        assert!(text.contains(token), "missing token in help: {token}");
    }
}

#[test]
fn ax_help_lists_subcommands() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(cwd.path(), &["ax", "--help"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());

    let text = format!("{}{}", out.stdout_text(), out.stderr_text());
    for token in ["list", "click", "type"] {
        assert!(text.contains(token), "missing token in ax help: {token}");
    }
}

#[test]
fn ax_click_rejects_mixed_selectors() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(
        cwd.path(),
        &[
            "ax",
            "click",
            "--node-id",
            "node-17",
            "--role",
            "AXButton",
            "--title-contains",
            "Save",
        ],
    );

    assert_eq!(out.code, 2);
    assert_eq!(out.stdout_text(), "");
    assert!(out.stderr_text().contains("cannot be used with"));
}

#[test]
fn input_source_current_and_switch_emit_json_payloads() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let options = harness.cmd_options(cwd.path()).with_env(
        "CODEX_MACOS_AGENT_TEST_INPUT_SOURCE_CURRENT",
        "com.apple.keylayout.US",
    );

    let current = harness.run_with_options(
        cwd.path(),
        &["--format", "json", "input-source", "current"],
        options.clone(),
    );
    assert_eq!(current.code, 0, "stderr: {}", current.stderr_text());
    let current_payload: serde_json::Value =
        serde_json::from_str(&current.stdout_text()).expect("current should emit json");
    assert_eq!(
        current_payload["command"],
        serde_json::json!("input-source.current")
    );
    assert_eq!(
        current_payload["result"]["current"],
        serde_json::json!("com.apple.keylayout.US")
    );

    let switched = harness.run_with_options(
        cwd.path(),
        &["--format", "json", "input-source", "switch", "--id", "abc"],
        options,
    );
    assert_eq!(switched.code, 0, "stderr: {}", switched.stderr_text());
    let switch_payload: serde_json::Value =
        serde_json::from_str(&switched.stdout_text()).expect("switch should emit json");
    assert_eq!(
        switch_payload["command"],
        serde_json::json!("input-source.switch")
    );
    assert_eq!(
        switch_payload["result"]["previous"],
        serde_json::json!("com.apple.keylayout.US")
    );
    assert_eq!(
        switch_payload["result"]["current"],
        serde_json::json!("com.apple.keylayout.ABC")
    );
    assert_eq!(
        switch_payload["result"]["switched"],
        serde_json::json!(true)
    );
}

#[test]
fn preflight_json_smoke() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(cwd.path(), &["--format", "json", "preflight"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr_text());
    assert_eq!(out.stderr_text(), "");

    let payload: serde_json::Value =
        serde_json::from_str(&out.stdout_text()).expect("stdout should be json");
    assert_eq!(payload["schema_version"], serde_json::json!(1));
    assert_eq!(payload["command"], serde_json::json!("preflight"));
}

#[test]
fn tsv_mode_is_rejected_for_input_commands() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(
        cwd.path(),
        &["--format", "tsv", "input", "click", "--x", "1", "--y", "2"],
    );

    assert_eq!(out.code, 2);
    assert_eq!(out.stdout_text(), "");
    assert!(out
        .stderr_text()
        .contains("only supported for `windows list` and `apps list`"));
}
