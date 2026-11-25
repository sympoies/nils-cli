//! Guards `short_socket_path` against checkout and temp-root layouts.
//!
//! A Unix-domain socket path must fit `sun_path` (104 bytes on macOS, 108 on
//! Linux), so a fixture that binds under `TMPDIR` or the checkout breaks on a
//! host whose temp root or work tree is long. The probe runs in a child process
//! so the long `TMPDIR`, a working directory containing `/repo`, and a permissive
//! umask apply without mutating this process's environment.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::Command;

use nils_test_support::short_socket_path;
use nils_test_support::tempdir::ScopedTempDir;

const PROBE_ENV: &str = "NILS_TEST_SUPPORT_SHORT_SOCKET_PROBE";
const PROBE_TEST: &str = "short_socket::short_socket_probe";
/// The strictest platform limit: macOS `sun_path` is 104 bytes including the NUL.
const SUN_PATH_LIMIT: usize = 104;

/// Runs only inside the child spawned by the guard below; otherwise a no-op.
#[test]
fn short_socket_probe() {
    if std::env::var_os(PROBE_ENV).is_none() {
        return;
    }

    let cwd = std::env::current_dir().expect("probe cwd");
    assert!(
        cwd.to_string_lossy().contains("/repo/"),
        "probe must run from a checkout-like path: {}",
        cwd.display()
    );
    let temp_root = std::env::temp_dir();
    assert!(
        temp_root.as_os_str().len() >= SUN_PATH_LIMIT,
        "probe must run under a long TMPDIR: {}",
        temp_root.display()
    );

    let (dir, socket) = short_socket_path("probe.sock");

    assert!(
        socket.as_os_str().len() < SUN_PATH_LIMIT,
        "socket path exceeds the platform limit: {}",
        socket.display()
    );
    let mode = std::fs::metadata(dir.path())
        .expect("socket dir metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "socket fixture directory must be private");
    let _listener = UnixListener::bind(&socket).expect("bind short socket");
}

#[test]
fn short_socket_path_binds_under_a_long_temp_root_checkout_and_open_umask() {
    let root = ScopedTempDir::new();
    let long_component = "l".repeat(60);
    let work_dir: PathBuf = root
        .path()
        .join(&long_component)
        .join(&long_component)
        .join("repo")
        .join("crates");
    let tmpdir = root.path().join(&long_component).join("tmp");
    std::fs::create_dir_all(&work_dir).expect("create long checkout path");
    std::fs::create_dir_all(&tmpdir).expect("create long temp root");

    let exe = std::env::current_exe().expect("current test executable");
    let output = Command::new("sh")
        .args(["-c", "umask 000; exec \"$0\" \"$@\""])
        .arg(exe)
        .args(["--exact", PROBE_TEST, "--nocapture", "--test-threads=1"])
        .env(PROBE_ENV, "1")
        .env("TMPDIR", &tmpdir)
        .current_dir(&work_dir)
        .output()
        .expect("spawn short socket probe");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "short socket probe failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
