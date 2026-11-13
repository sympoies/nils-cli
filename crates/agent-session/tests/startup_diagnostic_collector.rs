//! The app-server collector must finish after the source FIFO writer closes.
use std::ffi::CString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;

struct Collector(Option<Child>);

impl Drop for Collector {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            // SAFETY: the fixture owns this child and its dedicated process group.
            unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
            let _ = child.wait();
        }
    }
}

#[test]
fn fifo_diagnostic_is_retained_when_the_writer_closes_before_collection() {
    let root = nils_test_support::tempdir::ScopedTempDir::new();
    let fifo = root.path().join("stderr");
    let fifo_name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: fifo_name is a valid NUL-terminated path inside the fixture.
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
    let release = root.path().join("collect");
    let diagnostic = root.path().join("diagnostic");
    let stage = root.path().join("stage");
    let exit_status = root.path().join("exit");
    fs::write(&stage, "app_server\n").unwrap();
    fs::write(&exit_status, "23\n").unwrap();
    let script = format!(
        "{}\n{}",
        include_str!("../src/startup/codex-diagnostic.sh"),
        r#"exec 3< "$1"
while [ ! -f "$2" ]; do sleep 0.01; done
collect_startup_diagnostic <&3
"#
    );
    let mut collector = Collector(Some(
        Command::new("/bin/sh")
            .args(["-c", &script, "fixture"])
            .arg(&fifo)
            .arg(&release)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("startup_stage", &stage)
            .env("runtime_exit_status", &exit_status)
            .env("startup_diagnostic_buffer", root.path().join("buffer"))
            .env("startup_diagnostic", &diagnostic)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap(),
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut writer = loop {
        match OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo)
        {
            Ok(writer) => break writer,
            Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                assert!(Instant::now() < deadline, "collector never opened its FIFO");
                assert!(collector.0.as_mut().unwrap().try_wait().unwrap().is_none());
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("open diagnostic FIFO: {error}"),
        }
    };
    let payload = b"FINAL_STARTUP_REASON\n";
    writer.write_all(payload).unwrap();
    drop(writer);
    // The reader already owns the FIFO, but collection starts only after EOF.
    fs::write(&release, "").unwrap();
    loop {
        if collector.0.as_mut().unwrap().try_wait().unwrap().is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "collector stalled after its writer closed"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = collector.0.take().unwrap().wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&diagnostic).unwrap(), payload);
}
