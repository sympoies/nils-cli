use tempfile::TempDir;

mod common;

#[test]
fn success_commands_write_stdout_only() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let cases: Vec<Vec<&str>> = vec![
        vec!["--format", "json", "preflight"],
        vec!["--format", "json", "ax", "list"],
        vec![
            "--format",
            "json",
            "--dry-run",
            "ax",
            "click",
            "--node-id",
            "1.1",
        ],
        vec![
            "--format",
            "json",
            "--dry-run",
            "ax",
            "type",
            "--node-id",
            "1.1",
            "--text",
            "hello",
        ],
        vec![
            "--format", "json", "window", "activate", "--app", "Terminal",
        ],
        vec![
            "--format",
            "json",
            "input",
            "click",
            "--x",
            "10",
            "--y",
            "10",
            "--dry-run",
        ],
        vec![
            "--format",
            "json",
            "input",
            "type",
            "--text",
            "hello",
            "--dry-run",
        ],
        vec![
            "--format",
            "json",
            "input",
            "hotkey",
            "--mods",
            "cmd",
            "--key",
            "4",
            "--dry-run",
        ],
    ];

    for args in cases {
        let out = harness.run(cwd.path(), &args);
        assert_eq!(out.code, 0, "args={args:?}, stderr={}", out.stderr_text());
        assert!(!out.stdout_text().trim().is_empty(), "args={args:?}");
        assert_eq!(out.stderr_text(), "", "args={args:?}");
    }
}

#[test]
fn error_commands_write_stderr_only_with_error_prefix() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let cases: Vec<Vec<&str>> = vec![
        vec!["input", "hotkey", "--mods", "invalid", "--key", "4"],
        vec!["observe", "screenshot", "--window-id", "999"],
        vec!["input", "type", "--text", ""],
    ];

    for args in cases {
        let out = harness.run(cwd.path(), &args);
        assert!(out.code != 0, "args={args:?}");
        assert_eq!(out.stdout_text(), "", "args={args:?}");
        assert!(out.stderr_text().starts_with("error:"), "args={args:?}");
    }
}

#[test]
fn error_format_json_emits_machine_parseable_payload() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let out = harness.run(
        cwd.path(),
        &[
            "--error-format",
            "json",
            "input",
            "hotkey",
            "--mods",
            "invalid",
            "--key",
            "4",
        ],
    );

    assert_eq!(out.code, 2);
    assert_eq!(out.stdout_text(), "");
    let payload: serde_json::Value =
        serde_json::from_str(&out.stderr_text()).expect("stderr should be json");
    assert_eq!(payload["schema_version"], serde_json::json!(1));
    assert_eq!(payload["ok"], serde_json::json!(false));
    assert_eq!(payload["error"]["category"], serde_json::json!("usage"));
    assert!(payload["error"]["message"]
        .as_str()
        .unwrap_or("")
        .contains("invalid modifier"));
    assert!(
        payload["error"]["hints"].is_array() || payload["error"]["hints"].is_null(),
        "hints should be an array when present"
    );
}

#[test]
fn trace_writes_artifacts_for_success_and_failure() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let trace_dir = cwd.path().join("trace-out");
    let trace_dir_text = trace_dir.to_string_lossy().to_string();

    let ok = harness.run(
        cwd.path(),
        &[
            "--trace",
            "--trace-dir",
            &trace_dir_text,
            "--format",
            "json",
            "input",
            "click",
            "--x",
            "1",
            "--y",
            "2",
            "--dry-run",
        ],
    );
    assert_eq!(ok.code, 0, "stderr: {}", ok.stderr_text());

    let err = harness.run(
        cwd.path(),
        &[
            "--trace",
            "--trace-dir",
            &trace_dir_text,
            "--error-format",
            "json",
            "input",
            "hotkey",
            "--mods",
            "invalid",
            "--key",
            "4",
        ],
    );
    assert_eq!(err.code, 2);

    let entries = std::fs::read_dir(&trace_dir)
        .expect("trace dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .map(|ext| ext == "json")
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    assert!(
        entries.len() >= 2,
        "expected at least two trace files, got {}",
        entries.len()
    );
}

#[test]
fn trace_dir_not_writable_is_actionable_runtime_error() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let trace_file = cwd.path().join("trace-file");
    std::fs::write(&trace_file, b"blocked").expect("write trace blocker");

    let out = harness.run(
        cwd.path(),
        &[
            "--trace",
            "--trace-dir",
            &trace_file.to_string_lossy(),
            "--error-format",
            "json",
            "--format",
            "json",
            "preflight",
        ],
    );

    assert_eq!(out.code, 1, "stderr: {}", out.stderr_text());
    assert_eq!(out.stdout_text(), "");

    let payload: serde_json::Value =
        serde_json::from_str(&out.stderr_text()).expect("stderr should be json");
    assert_eq!(payload["schema_version"], serde_json::json!(1));
    assert_eq!(payload["ok"], serde_json::json!(false));
    assert_eq!(payload["error"]["category"], serde_json::json!("runtime"));
    assert_eq!(
        payload["error"]["operation"],
        serde_json::json!("trace.write")
    );
    assert!(payload["error"]["message"]
        .as_str()
        .unwrap_or("")
        .contains("not writable"));
    let has_hint = payload["error"]["hints"]
        .as_array()
        .map(|hints| {
            hints
                .iter()
                .any(|hint| hint.as_str().unwrap_or("").contains("writable directory"))
        })
        .unwrap_or(false);
    assert!(
        has_hint,
        "expected writable-directory hint in error payload"
    );
}

#[test]
fn trace_command_labels_include_ax_commands() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let trace_dir = cwd.path().join("trace-ax");
    let trace_dir_text = trace_dir.to_string_lossy().to_string();

    let list = harness.run(
        cwd.path(),
        &[
            "--trace",
            "--trace-dir",
            &trace_dir_text,
            "--format",
            "json",
            "ax",
            "list",
        ],
    );
    assert_eq!(list.code, 0, "stderr: {}", list.stderr_text());

    let click = harness.run(
        cwd.path(),
        &[
            "--trace",
            "--trace-dir",
            &trace_dir_text,
            "--format",
            "json",
            "--dry-run",
            "ax",
            "click",
            "--node-id",
            "1.1",
        ],
    );
    assert_eq!(click.code, 0, "stderr: {}", click.stderr_text());

    let typ = harness.run(
        cwd.path(),
        &[
            "--trace",
            "--trace-dir",
            &trace_dir_text,
            "--format",
            "json",
            "--dry-run",
            "ax",
            "type",
            "--node-id",
            "1.1",
            "--text",
            "hello",
        ],
    );
    assert_eq!(typ.code, 0, "stderr: {}", typ.stderr_text());

    let mut commands = std::fs::read_dir(&trace_dir)
        .expect("trace dir should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().map(|ext| ext == "json").unwrap_or(false))
        .map(|path| {
            let raw = std::fs::read_to_string(path).expect("trace file should be readable");
            let payload: serde_json::Value =
                serde_json::from_str(&raw).expect("trace payload should be json");
            payload["command"].as_str().unwrap_or_default().to_string()
        })
        .collect::<Vec<_>>();

    commands.sort();
    assert!(commands.iter().any(|command| command == "ax.list"));
    assert!(commands.iter().any(|command| command == "ax.click"));
    assert!(commands.iter().any(|command| command == "ax.type"));
}
