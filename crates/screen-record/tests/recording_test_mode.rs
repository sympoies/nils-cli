use tempfile::TempDir;

mod common;

#[test]
fn list_windows_outputs_fixture() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let output = harness.run(cwd.path(), &["--list-windows"]);

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    let stdout = output.stdout_text();
    let mut lines: Vec<&str> = stdout.trim().split('\n').collect();
    lines.retain(|line| !line.is_empty());

    assert_eq!(
        lines,
        vec![
            "200\tFinder\tFinder\t80\t80\t900\t600\ttrue",
            "101\tTerminal\tDocs\t40\t40\t1100\t760\ttrue",
            "100\tTerminal\tInbox\t0\t0\t1200\t800\ttrue",
        ]
    );
}

#[test]
fn list_apps_outputs_fixture() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let output = harness.run(cwd.path(), &["--list-apps"]);

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    let stdout = output.stdout_text();
    let mut lines: Vec<&str> = stdout.trim().split('\n').collect();
    lines.retain(|line| !line.is_empty());

    assert_eq!(
        lines,
        vec![
            "Finder\t222\tcom.apple.Finder",
            "Terminal\t111\tcom.apple.Terminal",
        ]
    );
}

#[test]
fn record_mov_fixture_writes_file() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let output_path = cwd.path().join("recording.mov");

    let output = harness.run(
        cwd.path(),
        &[
            "--app",
            "Terminal",
            "--duration",
            "1",
            "--audio",
            "off",
            "--path",
            output_path.to_str().unwrap(),
        ],
    );

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    let stdout = output.stdout_text();
    assert_eq!(stdout.trim(), output_path.display().to_string());
    let metadata = std::fs::metadata(&output_path).expect("output exists");
    assert!(metadata.len() > 0);
}

#[test]
fn record_mp4_fixture_writes_file() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let output_path = cwd.path().join("recording.mp4");

    let output = harness.run(
        cwd.path(),
        &[
            "--app",
            "Terminal",
            "--duration",
            "1",
            "--audio",
            "off",
            "--path",
            output_path.to_str().unwrap(),
        ],
    );

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    let stdout = output.stdout_text();
    assert_eq!(stdout.trim(), output_path.display().to_string());
    let metadata = std::fs::metadata(&output_path).expect("output exists");
    assert!(metadata.len() > 0);
}

#[test]
fn audio_both_requires_mov() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let output_path = cwd.path().join("recording.mp4");

    let output = harness.run(
        cwd.path(),
        &[
            "--app",
            "Terminal",
            "--duration",
            "1",
            "--audio",
            "both",
            "--path",
            output_path.to_str().unwrap(),
        ],
    );

    assert_eq!(output.code, 2);
    assert!(output.stderr_text().contains("--audio both requires .mov"));
}

#[test]
fn record_requires_path() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let output = harness.run(
        cwd.path(),
        &["--app", "Terminal", "--duration", "1", "--audio", "off"],
    );

    assert_eq!(output.code, 2);
    assert!(output
        .stderr_text()
        .contains("--path is required for recording"));
}

#[test]
fn screenshot_default_path_writes_png() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let output = harness.run(cwd.path(), &["--screenshot", "--app", "Terminal"]);

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    let base = std::fs::canonicalize(cwd.path()).expect("canonicalize cwd");
    let expected = base
        .join("screenshots")
        .join("screenshot-20260101-000000-win100-Terminal-Inbox.png");
    assert_eq!(output.stdout_text().trim(), expected.display().to_string());
    let metadata = std::fs::metadata(&expected).expect("output exists");
    assert!(metadata.len() > 0);
}

#[test]
fn screenshot_image_format_webp_writes_file() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let output = harness.run(
        cwd.path(),
        &[
            "--screenshot",
            "--app",
            "Terminal",
            "--image-format",
            "webp",
        ],
    );

    assert_eq!(output.code, 0, "stderr: {}", output.stderr_text());
    let base = std::fs::canonicalize(cwd.path()).expect("canonicalize cwd");
    let expected = base
        .join("screenshots")
        .join("screenshot-20260101-000000-win100-Terminal-Inbox.webp");
    assert_eq!(output.stdout_text().trim(), expected.display().to_string());
    let metadata = std::fs::metadata(&expected).expect("output exists");
    assert!(metadata.len() > 0);
}

#[test]
fn screenshot_path_and_image_format_conflict_errors() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");
    let output_path = cwd.path().join("out.png");

    let output = harness.run(
        cwd.path(),
        &[
            "--screenshot",
            "--app",
            "Terminal",
            "--path",
            output_path.to_str().unwrap(),
            "--image-format",
            "jpg",
        ],
    );

    assert_eq!(output.code, 2);
    assert!(output
        .stderr_text()
        .contains("--image-format jpg conflicts with --path extension"));
}

#[test]
fn screenshot_rejects_duration_flag() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let output = harness.run(
        cwd.path(),
        &["--screenshot", "--app", "Terminal", "--duration", "1"],
    );

    assert_eq!(output.code, 2);
    assert!(output
        .stderr_text()
        .contains("--duration is not valid with --screenshot"));
}

#[test]
fn screenshot_only_flags_require_screenshot_mode() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let output = harness.run(cwd.path(), &["--dir", "screens"]);

    assert_eq!(output.code, 2);
    assert!(output
        .stderr_text()
        .contains("screenshot flags require --screenshot"));
}

#[test]
fn list_mode_rejects_screenshot_flags() {
    let harness = common::ScreenRecordHarness::new();
    let cwd = TempDir::new().expect("tempdir");

    let output = harness.run(cwd.path(), &["--list-windows", "--image-format", "png"]);

    assert_eq!(output.code, 2);
    assert!(output
        .stderr_text()
        .contains("screenshot flags require --screenshot"));
}
