use std::fs as std_fs;
use std::sync::Arc;

use nils_test_support::fs;
use serde_json::json;

#[test]
fn write_text_creates_parents_and_writes_contents() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let path = temp.path().join("nested/dir/file.txt");
    let written = fs::write_text(&path, "hello\n");
    assert_eq!(std_fs::read_to_string(written).expect("read"), "hello\n");
}

#[test]
fn write_text_in_dir_joins_relative_path_and_writes_contents() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let written = fs::write_text_in_dir(temp.path(), "nested/dir/file.txt", "hello\n");
    assert_eq!(std_fs::read_to_string(written).expect("read"), "hello\n");
}

#[test]
fn write_bytes_preserves_raw_bytes() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let path = temp.path().join("bin/data.bin");
    let data = [0u8, 159, 146, 150, 255];
    let written = fs::write_bytes(&path, &data);
    assert_eq!(std_fs::read(written).expect("read"), data);
}

#[test]
fn write_json_writes_pretty_json() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let path = temp.path().join("config/settings.json");
    let value = json!({"ok": true, "count": 2});
    let written = fs::write_json(&path, &value);
    let actual = std_fs::read_to_string(written).expect("read");
    let expected = serde_json::to_string_pretty(&value).expect("json");
    assert_eq!(actual, expected);
}

#[test]
fn write_executable_writes_contents() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let path = temp.path().join("bin/run.sh");
    let written = fs::write_executable(&path, "#!/bin/sh\necho ok\n");
    assert_eq!(
        std_fs::read_to_string(written).expect("read"),
        "#!/bin/sh\necho ok\n"
    );
}

#[test]
fn write_executable_in_dir_joins_relative_path() {
    let temp = tempfile::TempDir::new().expect("tempdir");
    let written = fs::write_executable_in_dir(temp.path(), "bin/run.sh", "#!/bin/sh\necho ok\n");
    assert_eq!(
        std_fs::read_to_string(written).expect("read"),
        "#!/bin/sh\necho ok\n"
    );
}

#[cfg(unix)]
#[test]
fn write_executable_sets_unix_mode() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::TempDir::new().expect("tempdir");
    let path = temp.path().join("bin/tool");
    let written = fs::write_executable(&path, "echo ok\n");
    let mode = std_fs::metadata(written)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111);
}

/// On Linux the kernel keeps a script's inode text-busy while any process
/// holds it open for write, so `execve` of it fails with ETXTBSY; a sibling
/// test thread that forks while a fixture writer's descriptor is still open
/// inherits that descriptor, which is how a freshly written fixture ends up
/// busy at pre-warm time (sympoies/nils-cli#2170). Hold the descriptor across
/// the probe so the failure is reproducible on demand.
///
/// This ETXTBSY-on-exec behavior is Linux-specific: on macOS `execve` of a
/// write-open shebang script succeeds instead of failing, so reproducing the
/// busy state (and the #2170 flake itself) is a Linux phenomenon. The other
/// regressions in this file exercise the atomic install itself and run on
/// every unix target.
#[cfg(target_os = "linux")]
#[test]
fn exec_reports_executable_file_busy_while_the_script_is_open_for_write() {
    use std::io::ErrorKind;
    use std::process::Command;
    use std::thread;
    use std::time::{Duration, Instant};

    let temp = tempfile::TempDir::new().expect("tempdir");
    let path = temp.path().join("bin/tool");
    fs::write_executable(&path, "#!/bin/sh\nexit 0\n");

    let writer = std_fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open writer");
    let probe = Command::new(&path).output();
    match probe {
        Err(error) => assert_eq!(error.kind(), ErrorKind::ExecutableFileBusy),
        Ok(_) => panic!("exec of a write-open script must fail with ETXTBSY"),
    }

    drop(writer);
    // The kernel's text-busy clear can race an immediate exec (the last
    // write descriptor was just closed), so retry the post-close exec briefly
    // before asserting the fixture is no longer busy.
    let deadline = Instant::now() + Duration::from_millis(200);
    loop {
        match Command::new(&path).output() {
            Ok(probe) => {
                assert!(probe.status.success());
                return;
            }
            Err(error)
                if error.kind() == ErrorKind::ExecutableFileBusy && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("exec after close failed: {error}"),
        }
    }
}

/// The install must leave the fixture directory clean and the target
/// immediately executable: the write-then-exec ordering a pre-warm relies on.
#[cfg(unix)]
#[test]
fn write_executable_installs_cleanly_and_is_immediately_executable() {
    use std::process::Command;

    let temp = tempfile::TempDir::new().expect("tempdir");
    let path = temp.path().join("tool");
    let script = "#!/bin/sh\nexit 0\n";
    let written = fs::write_executable(&path, script);
    assert_eq!(written, path);
    assert_eq!(std_fs::read_to_string(&written).expect("read"), script);

    let entries = std_fs::read_dir(temp.path())
        .expect("read dir")
        .map(|entry| entry.expect("entry").file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec!["tool"]);

    let output = Command::new(&written).output().expect("exec");
    assert!(output.status.success());
}

#[cfg(unix)]
#[test]
fn write_executable_with_mode_applies_the_requested_mode() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let temp = tempfile::TempDir::new().expect("tempdir");
    let path = temp.path().join("tool");
    let written = fs::write_executable_with_mode(&path, "#!/bin/sh\nexit 0\n", 0o700);
    let mode = std_fs::metadata(&written)
        .expect("metadata")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);

    let output = Command::new(&written).output().expect("exec");
    assert!(output.status.success());
}

/// Model the pre-warm's bounded ETXTBSY retry while a sibling installs the
/// fixture repeatedly. A writer that leaves the target open-for-write during
/// the install (the old in-place `fs::write`) keeps the target busy for the
/// whole write, so the bounded retry exhausts and the pre-warm fails — exactly
/// the failure in sympoies/nils-cli#2170. An atomic install (sibling temp,
/// close, rename) never leaves the target busy by the writer, so the retry
/// never exhausts.
#[cfg(unix)]
#[test]
fn write_executable_prewarm_retry_never_exhausts_while_installing() {
    use std::io::ErrorKind;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    let temp = tempfile::TempDir::new().expect("tempdir");
    let path = temp.path().join("bin/tool");
    // A large body makes an in-place writer's open-for-write window long
    // enough to outlast the pre-warm's bounded retry.
    let body = format!("#!/bin/sh\n# {}\nexit 0\n", "x".repeat(16 * 1024 * 1024));

    // Install once before the probe starts so the target exists and the probe
    // never observes ENOENT (the rename keeps the target present thereafter).
    fs::write_executable(&path, &body);

    let exhausted = Arc::new(AtomicUsize::new(0));
    let running = Arc::new(AtomicBool::new(true));

    let probe_path = path.clone();
    let probe_exhausted = Arc::clone(&exhausted);
    let probe_running = Arc::clone(&running);
    let probe = thread::spawn(move || {
        while probe_running.load(Ordering::Relaxed) {
            // The pre-warm's contract: a bounded ETXTBSY retry.
            let deadline = Instant::now() + Duration::from_millis(10);
            let mut success = false;
            loop {
                match Command::new(&probe_path).output() {
                    Ok(_) => {
                        success = true;
                        break;
                    }
                    Err(error)
                        if error.kind() == ErrorKind::ExecutableFileBusy
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                    // Busy past the deadline: this is the pre-warm failure the
                    // regression guards against.
                    Err(error) if error.kind() == ErrorKind::ExecutableFileBusy => break,
                    Err(error) => panic!("probe exec failed with a non-ETXTBSY error: {error}"),
                }
            }
            if !success {
                probe_exhausted.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    for _ in 0..8 {
        fs::write_executable(&path, &body);
    }
    running.store(false, Ordering::Relaxed);
    probe.join().expect("probe thread");

    assert_eq!(
        exhausted.load(Ordering::Relaxed),
        0,
        "the pre-warm's bounded ETXTBSY retry was exhausted while installing"
    );
}
