use super::*;

#[test]
fn metadata_equality_includes_ctime() {
    let root = tempfile::TempDir::new().expect("metadata root");
    let path = root.path().join("file");
    fs::write(&path, b"content").expect("write fixture");
    let before = fs::metadata(&path).expect("metadata before");
    std::thread::sleep(Duration::from_millis(10));
    fs::set_permissions(&path, before.permissions()).expect("touch ctime with chmod");
    let after = fs::metadata(&path).expect("metadata after");

    assert_eq!(before.mode(), after.mode());
    assert_eq!(before.len(), after.len());
    assert_eq!(before.mtime(), after.mtime());
    assert!(
        !same_metadata(&before, &after),
        "ctime-only drift must invalidate metadata equality"
    );
}

/// A `/proc` entry whose process departs after the open must read as
/// absent, not as a resource failure.
///
/// The descendant-cleanup scan walks short-lived grandchildren, so this
/// race is routine. `openat` reports a departed process as `ENOENT`, but a
/// process that exits between the open and the read reports `ESRCH` on the
/// read, which Rust maps to `Uncategorized` rather than `NotFound`. That
/// turned a benign liveness race into `dirty-checkout-resource-unavailable`
/// and failed the whole snapshot, with the diagnostic only on stdout.
#[cfg(target_os = "linux")]
#[test]
fn a_proc_entry_read_after_its_process_exits_is_absent_not_a_resource_failure() {
    use std::process::{Command, Stdio};

    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("exec sleep 30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn a short-lived process to scan");
    let path = PathBuf::from(format!("/proc/{}/stat", child.id()));

    // Open while the process is alive, so the open cannot be the failure.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&path)
        .expect("open the live process entry");

    // Retire and reap it, which is what makes the held descriptor report
    // ESRCH deterministically instead of racing the exit.
    child.kill().expect("retire the scanned process");
    child.wait().expect("reap the scanned process");

    let read = read_open_linux_metadata(file, 4096);
    assert!(
        matches!(read, Ok(None)),
        "a departed process must read as absent, got {read:?}"
    );

    // The same departure seen at open time keeps its existing verdict.
    assert!(
        matches!(read_linux_metadata(&path, 4096), Ok(None)),
        "a process gone before the open must also read as absent"
    );
}

/// Only a departure is excused. A permission or descriptor-exhaustion
/// failure must still surface as a resource failure.
#[cfg(target_os = "linux")]
#[test]
fn only_a_departed_process_errno_is_excused_by_the_proc_scan() {
    for excused in [libc::ESRCH, libc::ENOENT] {
        assert!(
            is_departed_process_error(&io::Error::from_raw_os_error(excused)),
            "errno {excused} means the process is gone"
        );
    }
    for refused in [libc::EACCES, libc::EMFILE, libc::ENFILE, libc::EIO] {
        assert!(
            !is_departed_process_error(&io::Error::from_raw_os_error(refused)),
            "errno {refused} is not a departure and must stay a failure"
        );
    }
}
