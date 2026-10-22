use std::path::Path;
use std::process::Command;

use nils_test_support::bin;
use pretty_assertions::assert_eq;
use serde_json::Value;

fn service_send_error(body_file: &Path) -> Value {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut command = Command::new(bin::resolve("agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let output = command
        .arg("--state-dir")
        .arg(tmp.path().join("state"))
        .args([
            "message",
            "service-send",
            "--service",
            "fixture-service",
            "--service-generation",
            "fixture-generation",
            "--credential-file",
        ])
        .arg(tmp.path().join("missing-credential"))
        .args(["--to", "fixture-recipient", "--body-file"])
        .arg(body_file)
        .args(["--idempotency-key", "body-source-test", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(65));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        envelope["schema_version"],
        "cli.agent-session.message-service-send.v1"
    );
    assert_eq!(envelope["error"]["details"]["retryable"], false);
    envelope["error"].clone()
}

#[cfg(target_os = "linux")]
#[test]
fn service_send_memfd_body_reports_source_rejection() {
    use std::io::{Seek, Write};
    use std::os::fd::{AsRawFd, FromRawFd};

    // No CLOEXEC: the CLI must see the same readable memfd in /proc/self/fd.
    let fd = unsafe { libc::memfd_create(c"mailbox-body-fixture".as_ptr(), 0) };
    assert!(fd >= 0);
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(b"valid message body\n").unwrap();
    file.rewind().unwrap();
    let path = format!("/proc/self/fd/{}", file.as_raw_fd());
    assert_eq!(std::fs::read(&path).unwrap(), b"valid message body\n");
    let error = service_send_error(Path::new(&path));
    assert_eq!(error["code"], "mailbox-body-source-invalid");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("symbolic links")
    );
    assert!(!error.to_string().contains(&path));
}

#[test]
fn service_send_body_source_errors_are_distinct_from_content_errors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let body = tmp.path().join("body");
    std::fs::write(&body, b"valid body").unwrap();
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&body, &link).unwrap();
    for path in [link, tmp.path().to_owned(), tmp.path().join("missing")] {
        let error = service_send_error(&path);
        assert_eq!(error["code"], "mailbox-body-source-invalid");
        assert!(!error.to_string().contains(path.to_str().unwrap()));
    }
    for content in [b"".as_slice(), b"\xff", b"body\0", b"body\x01"] {
        std::fs::write(&body, content).unwrap();
        assert_eq!(service_send_error(&body)["code"], "mailbox-body-invalid");
    }
    std::fs::write(&body, vec![b'x'; 16 * 1024 + 1]).unwrap();
    assert_eq!(service_send_error(&body)["code"], "mailbox-body-too-large");
    std::fs::write(&body, b"valid body\n").unwrap();
    assert_eq!(
        service_send_error(&body)["code"],
        "mailbox-service-unauthorized"
    );
}
