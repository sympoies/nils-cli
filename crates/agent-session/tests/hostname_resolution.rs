use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn resolving_cli_context_does_not_execute_hostname() {
    let dir = tempfile::TempDir::new().unwrap();
    let hostname = dir.path().join("hostname");
    let called = dir.path().join("called");
    fs::write(
        &hostname,
        "#!/bin/sh\nprintf called > \"$HOSTNAME_CALLED\"\nprintf 'fixture-host\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&hostname, fs::Permissions::from_mode(0o700)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_agent-session"))
        .env_clear()
        .env("HOME", dir.path())
        .env("PATH", dir.path())
        .env("HOSTNAME_CALLED", &called)
        .arg("--state-dir")
        .arg(dir.path().join("state"))
        .args(["list", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!called.exists(), "agent-session executed hostname");
}
