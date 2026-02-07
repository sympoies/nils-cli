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
        "observe",
        "wait",
    ] {
        assert!(text.contains(token), "missing token in help: {token}");
    }
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
