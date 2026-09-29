//! A package-manager upgrade replaces or removes the installed `agent-session`
//! under a running `serve` (sympoies/nils-cli#1817). The daemon must notice,
//! drain, and exit with the documented restart code so its supervisor starts
//! the new binary, without touching the tmux sessions it already launched.

use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;

/// `EX_TEMPFAIL`: the documented "restart me on the new binary" exit code.
const BINARY_REPLACED_EXIT: i32 = 75;
const OPT_OUT_ENV: &str = "AGENT_SESSION_SERVE_EXIT_ON_BINARY_CHANGE";

/// A versioned install prefix holding a private copy of the built binary, so
/// the test can replace or remove it without disturbing the Cargo artifact.
struct Install {
    version_dir: PathBuf,
    binary: PathBuf,
}

impl Install {
    fn new(root: &Path) -> Self {
        let version_dir = root.join("Cellar/nils-cli/1.0.0");
        let bin = version_dir.join("bin");
        fs::create_dir_all(&bin).expect("install bin");
        let binary = bin.join("agent-session");
        copy_executable(&binary);
        Self {
            version_dir,
            binary,
        }
    }

    /// Atomic rename over the installed path, as package managers upgrade.
    fn replace(&self) {
        let staged = self.binary.with_extension("new");
        copy_executable(&staged);
        fs::rename(&staged, &self.binary).expect("replace installed binary");
    }

    /// The old versioned release disappears, as after a Homebrew cleanup.
    fn remove(&self) {
        fs::remove_dir_all(&self.version_dir).expect("remove installed release");
    }
}

fn copy_executable(to: &Path) {
    fs::copy(nils_test_support::bin::resolve("agent-session"), to).expect("copy agent-session");
    fs::set_permissions(to, fs::Permissions::from_mode(0o755)).expect("binary mode");
}

/// A tmux stand-in with no server that records every invocation, so the test
/// can prove the draining daemon never issued a destructive tmux command.
fn fake_tmux(root: &Path) -> (PathBuf, PathBuf) {
    let bin = root.join("tmux");
    let log = root.join("tmux.log");
    fs::write(
        &bin,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nprintf '%s\\n' 'no server running on /tmp/tmux-test/default' >&2\nexit 1\n",
            log.display()
        ),
    )
    .expect("fake tmux");
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).expect("fake tmux mode");
    (bin, log)
}

fn unused_loopback_addr() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("loopback port")
}

/// Kills its serve on drop, so a failed assertion cannot orphan a daemon.
struct Serve {
    child: Child,
    addr: SocketAddr,
    stderr_path: PathBuf,
    tmux_log: PathBuf,
}

impl Serve {
    fn spawn(root: &Path, install: &Install, opt_out: Option<&str>) -> Self {
        let home = root.join("home");
        fs::create_dir_all(&home).expect("serve home");
        let (tmux, tmux_log) = fake_tmux(root);
        let addr = unused_loopback_addr();
        let stderr_path = root.join("serve.stderr");
        let mut command = Command::new(&install.binary);
        command
            .arg("serve")
            .arg("--bind")
            .arg(addr.to_string())
            .arg("--state-dir")
            .arg(root.join("state"))
            .env("HOME", &home)
            .env_remove("XDG_STATE_HOME")
            .env("AGENT_SESSION_TMUX_BIN", &tmux)
            .env_remove(OPT_OUT_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(&stderr_path).expect("serve stderr"),
            ));
        for key in nils_test_support::cmd::MANAGED_SESSION_ENV {
            command.env_remove(key);
        }
        if let Some(value) = opt_out {
            command.env(OPT_OUT_ENV, value);
        }
        let child = command.spawn().expect("spawn serve");
        let mut serve = Self {
            child,
            addr,
            stderr_path,
            tmux_log,
        };
        serve.wait_listening();
        serve
    }

    fn wait_listening(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while TcpStream::connect(self.addr).is_err() {
            if let Some(status) = self.child.try_wait().expect("poll serve") {
                panic!("serve exited before listening: {status}; {}", self.stderr());
            }
            assert!(
                Instant::now() < deadline,
                "serve did not listen: {}",
                self.stderr()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn wait_exit(&mut self, timeout: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("poll serve") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn stderr(&self) -> String {
        fs::read_to_string(&self.stderr_path).unwrap_or_default()
    }

    fn destructive_tmux_calls(&self) -> Vec<String> {
        fs::read_to_string(&self.tmux_log)
            .unwrap_or_default()
            .lines()
            .filter(|call| call.split_whitespace().any(|arg| arg.starts_with("kill-")))
            .map(str::to_owned)
            .collect()
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn assert_exits_for_restart(change: impl FnOnce(&Install)) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let install = Install::new(&tmp.path().join("prefix"));
    let mut serve = Serve::spawn(tmp.path(), &install, None);

    change(&install);

    let status = serve
        .wait_exit(Duration::from_secs(20))
        .unwrap_or_else(|| panic!("serve kept running: {}", serve.stderr()));
    assert_eq!(
        status.code(),
        Some(BINARY_REPLACED_EXIT),
        "{}",
        serve.stderr()
    );
    assert!(
        serve.stderr().contains("serve-binary-replaced"),
        "{}",
        serve.stderr()
    );
    assert_eq!(serve.destructive_tmux_calls(), Vec::<String>::new());
}

#[test]
fn serve_exits_for_restart_when_its_binary_is_replaced() {
    assert_exits_for_restart(Install::replace);
}

#[test]
fn serve_exits_for_restart_when_its_release_is_removed() {
    assert_exits_for_restart(Install::remove);
}

#[test]
fn serve_keeps_running_after_replacement_when_opted_out() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let install = Install::new(&tmp.path().join("prefix"));
    let mut serve = Serve::spawn(tmp.path(), &install, Some("0"));

    install.replace();

    assert!(
        serve.wait_exit(Duration::from_secs(4)).is_none(),
        "opted-out serve exited: {}",
        serve.stderr()
    );
    assert!(TcpStream::connect(serve.addr).is_ok());
}
