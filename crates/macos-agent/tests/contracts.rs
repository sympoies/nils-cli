use tempfile::TempDir;

mod common;

#[test]
fn success_commands_write_stdout_only() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let cases: Vec<Vec<&str>> = vec![
        vec!["--format", "json", "preflight"],
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
