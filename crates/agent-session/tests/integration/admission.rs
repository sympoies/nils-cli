//! Admission contracts exercised through independent CLI processes and HTTP.
use super::cli::{
    fake_agent, fake_tmux, spawn_scoped_test_process_group, tmux_calls,
    write_resumable_session_record_with_agent_bin,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Barrier},
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    dir: tempfile::TempDir,
    tmux: PathBuf,
    log: PathBuf,
    provider: PathBuf,
}
impl Fixture {
    fn new(limit: Option<u64>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (tmux, log) = fake_tmux(dir.path());
        let provider = fake_agent(dir.path(), "provider");
        fs::create_dir(dir.path().join("state")).unwrap();
        fs::create_dir(dir.path().join("home")).unwrap();
        if let Some(limit) = limit {
            fs::write(
                dir.path().join("state/admission.json"),
                json!({"max_active_sessions":limit}).to_string(),
            )
            .unwrap();
        }
        Self {
            dir,
            tmux,
            log,
            provider,
        }
    }
    fn command(&self) -> Command {
        let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
        nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("AGENT_SESSION_") {
                command.env_remove(key);
            }
        }
        command
            .current_dir(self.dir.path())
            .args(["--host", "example-host"])
            .arg("--state-dir")
            .arg(self.dir.path().join("state"))
            .env("HOME", self.dir.path().join("home"))
            .env("AGENT_HOME", self.dir.path().join("agent-home"))
            .env("CODEX_HOME", self.dir.path().join("home"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .env("AGENT_SESSION_MACHINE", "example-host")
            .env("AGENT_SESSION_CODEX_BIN", &self.provider)
            .env("AGENT_SESSION_CLAUDE_BIN", &self.provider)
            .env("AGENT_SESSION_TMUX_BIN", &self.tmux)
            .env("AGENT_SESSION_FAKE_TMUX_LOG", &self.log)
            .env("AGENT_SESSION_FAKE_TMUX_ABSENT_AFTER_KILL", "1")
            .env("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1")
            .stdin(Stdio::null());
        command
    }
    fn start(&self, id: &str) -> Command {
        self.start_for(id, "codex")
    }
    fn start_for(&self, id: &str, agent: &str) -> Command {
        let mut command = self.command();
        command
            .args(["start", "--agent", agent, "--id", id, "--format", "json"])
            .arg("--cwd")
            .arg(self.dir.path());
        command
    }
    fn exact_command(&self, id: &str) -> Command {
        let record: Value = serde_json::from_slice(
            &fs::read(
                self.dir
                    .path()
                    .join("state/sessions")
                    .join(id)
                    .join("session.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let identity = &record["delete_tmux_identity"];
        let mut command = self.command();
        command
            .env("AGENT_SESSION_FAKE_TMUX_AGENT_SESSION_ID", id)
            .env(
                "AGENT_SESSION_FAKE_TMUX_STATE_DIR",
                self.dir.path().join("state"),
            )
            .env(
                "AGENT_SESSION_FAKE_TMUX_RUNTIME_ID",
                record["runtime"]["launch_id"].as_str().unwrap(),
            )
            .env(
                "AGENT_SESSION_FAKE_TMUX_SESSION_ID",
                identity["session_id"].as_str().unwrap(),
            )
            .env(
                "AGENT_SESSION_FAKE_TMUX_PANE_ID",
                identity["pane_id"].as_str().unwrap(),
            )
            .env(
                "AGENT_SESSION_FAKE_TMUX_PANE_PID",
                identity["pane_pid"].as_i64().unwrap().to_string(),
            );
        command
    }
    fn starts(&self) -> usize {
        tmux_calls(&self.log)
            .iter()
            .filter(|args| args.first().is_some_and(|a| a == "new-session"))
            .count()
    }
    fn capacity(&self) -> Value {
        let output = self
            .command()
            .args(["diagnose", "--format", "json"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"]["admission"].clone()
    }
    fn serve(&self) -> (Daemon, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let child = self
            .command()
            .args([
                "serve",
                "--bind",
                &addr.to_string(),
                "--token",
                "fixture-token",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut daemon = Daemon(child);
        let deadline = Instant::now() + Duration::from_secs(15);
        while TcpStream::connect(addr).is_err() {
            assert!(daemon.0.try_wait().unwrap().is_none(), "daemon exited");
            assert!(Instant::now() < deadline, "daemon did not bind");
            thread::sleep(Duration::from_millis(20));
        }
        (daemon, format!("http://{addr}/sessions"))
    }
}
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn envelope(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"))
}
fn refused(value: &Value) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], "session-capacity-exceeded");
    assert_eq!(value["error"]["details"]["max_active_sessions"], 1);
    assert_eq!(value["error"]["details"]["active_sessions"], 1);
    assert_eq!(value["error"]["details"]["retryable"], true);
    assert_eq!(
        value["error"]["details"]["next_action"],
        "wait-for-capacity"
    );
    assert_eq!(
        value["error"]["details"]["recovery"],
        json!({"kind":"wait", "condition":"active_sessions_below_limit","safe_to_retry":true})
    );
}
#[test]
fn admission_simultaneous_cli_creates_are_atomic() {
    let f = Arc::new(Fixture::new(Some(1)));
    let barrier = Arc::new(Barrier::new(2));
    let workers = ["first", "second"].map(|id| {
        let f = f.clone();
        let barrier = barrier.clone();
        thread::spawn(move || {
            barrier.wait();
            (id, f.start(id).output().unwrap())
        })
    });
    let outputs = workers.map(|worker| worker.join().unwrap());
    assert_eq!(
        outputs.iter().filter(|(_, o)| o.status.success()).count(),
        1,
        "{outputs:?}"
    );
    let (id, output) = outputs.iter().find(|(_, o)| !o.status.success()).unwrap();
    refused(&envelope(output));
    assert!(!f.dir.path().join("state/sessions").join(id).exists());
    assert_eq!(f.starts(), 1);
}
#[test]
fn admission_cli_and_http_share_capacity_across_daemon_restart() {
    let f = Arc::new(Fixture::new(Some(1)));
    let (daemon, url) = f.serve();
    let barrier = Arc::new(Barrier::new(2));
    let cli = {
        let f = f.clone();
        let b = barrier.clone();
        thread::spawn(move || {
            b.wait();
            f.start("cli").output().unwrap()
        })
    };
    barrier.wait();
    let response = reqwest::blocking::Client::new()
        .post(&url)
        .bearer_auth("fixture-token")
        .json(&json!({"agent":"codex", "id":"http", "cwd":f.dir.path()}))
        .send()
        .unwrap();
    let status = response.status();
    if !status.is_success() {
        assert_eq!(status.as_u16(), 409);
    }
    let http: Value = response.json().unwrap();
    let cli = cli.join().unwrap();
    assert_eq!(
        usize::from(cli.status.success()) + usize::from(status.is_success()),
        1,
        "CLI={cli:?} HTTP={http}"
    );
    let loser = if cli.status.success() {
        refused(&http);
        "http"
    } else {
        refused(&envelope(&cli));
        "cli"
    };
    assert!(!f.dir.path().join("state/sessions").join(loser).exists());
    assert_eq!(f.starts(), 1);
    drop(daemon);
    let (_restarted, url) = f.serve();
    let again: Value = reqwest::blocking::Client::new()
        .post(url)
        .bearer_auth("fixture-token")
        .json(&json!({"agent":"codex", "id":"after-restart", "cwd":f.dir.path()}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    refused(&again);
    assert_eq!(f.starts(), 1);
}
#[test]
fn admission_unlimited_and_read_only_occupancy() {
    let f = Fixture::new(None);
    let before = fs::read_dir(f.dir.path().join("state")).unwrap().count();
    assert_eq!(
        f.capacity(),
        json!({"max_active_sessions":null,"active_sessions":0})
    );
    assert_eq!(
        fs::read_dir(f.dir.path().join("state")).unwrap().count(),
        before
    );
    for id in ["first", "second"] {
        let output = f.start(id).output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    assert_eq!(
        f.capacity(),
        json!({"max_active_sessions":null,"active_sessions":2})
    );
}
#[test]
fn admission_stopped_and_deleted_sessions_release_capacity() {
    let f = Fixture::new(Some(1));
    let first = f.start("first").output().unwrap();
    assert!(first.status.success(), "{first:?}");
    let stopped = Command::new(&f.tmux)
        .args(["kill-session", "-t", "=hs-codex-first"])
        .env("AGENT_SESSION_FAKE_TMUX_LOG", &f.log)
        .output()
        .unwrap();
    assert!(stopped.status.success());
    assert_eq!(f.capacity()["active_sessions"], 0);
    let pane = spawn_scoped_test_process_group().expect("scoped deletion fixture is required");
    let pid = pane.pid().to_string();
    let second = f
        .start("second")
        .env("AGENT_SESSION_FAKE_TMUX_PANE_PID", &pid)
        .output()
        .unwrap();
    assert!(second.status.success(), "{second:?}");
    // The kernel-backed scoped fixture owns process termination. Model tmux's
    // removal of that exact pane with a shell-only receipt inside the existing
    // short teardown command budget (no interpreter startup on the kill path).
    let teardown_tmux = f.dir.path().join("teardown-tmux");
    fs::write(
        &teardown_tmux,
        format!(
            r#"#!/bin/sh
if [ "$1" = if-shell ] && [ "$6" = "kill-session -t $AGENT_SESSION_FAKE_TMUX_SESSION_ID" ]; then
  [ "$4" = "$AGENT_SESSION_FAKE_TMUX_PANE_ID" ] || exit 42
  printf '%s\n' "$AGENT_SESSION_FAKE_TMUX_SESSION_ID" >> "$AGENT_SESSION_FAKE_TMUX_LOG.killed"
  exit 0
fi
exec {} "$@"
"#,
            shell_words::quote(&f.tmux.to_string_lossy())
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&teardown_tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let deleted = f
        .exact_command("second")
        .env("AGENT_SESSION_FAKE_TMUX_PROCESS_GROUP_ID", &pid)
        .env("AGENT_SESSION_TMUX_BIN", &teardown_tmux)
        .args(["delete", "second", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        deleted.status.success(),
        "{}; last operation={:?}",
        envelope(&deleted)["error"],
        tmux_calls(&f.log).last()
    );
    let third = f.start("third").output().unwrap();
    assert!(third.status.success(), "{third:?}");
}
#[test]
fn admission_failed_and_cancelled_creates_release_capacity() {
    let f = Fixture::new(Some(1));
    let failed = f
        .start("failed")
        .env("AGENT_SESSION_FAKE_TMUX_FAIL", "new-session")
        .env("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1")
        .output()
        .unwrap();
    assert!(!failed.status.success());
    let barrier = f.dir.path().join("barrier");
    fs::create_dir(&barrier).unwrap();
    let wrapper = f.dir.path().join("paused-tmux");
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    fs::write(
        &wrapper,
        format!(
            r#"#!/bin/sh
if [ "$1" = new-session ]; then
  : > "$ADMISSION_BARRIER/ready"
  attempts=0
  while [ -d "$ADMISSION_BARRIER" ] && [ "$attempts" -lt 1000 ]; do
    attempts=$((attempts + 1))
    sleep 0.01
  done
  exit 42
fi
exec {} "$@"
"#,
            shell_words::quote(&f.tmux.to_string_lossy())
        ),
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    // A dedicated process group bounds the paused subprocess and is reaped
    // on every assertion path, including a failure before cancellation.
    struct PausedCreate(Child);
    impl Drop for PausedCreate {
        fn drop(&mut self) {
            // SAFETY: the unreaped child owns the process group created below.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.wait();
        }
    }
    let mut command = f.start("cancelled");
    command
        .env("AGENT_SESSION_TMUX_BIN", &wrapper)
        .env("ADMISSION_BARRIER", &barrier)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut cancelled = PausedCreate(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !barrier.join("ready").exists() {
        assert!(cancelled.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        f.dir
            .path()
            .join("state/sessions/cancelled/session.json")
            .is_file()
    );
    let attempts = f.starts();
    let blocked = f.start("blocked").output().unwrap();
    refused(&envelope(&blocked));
    assert_eq!(f.starts(), attempts, "refusal must precede runtime launch");
    assert!(!f.dir.path().join("state/sessions/blocked").exists());
    drop(cancelled);
    assert_eq!(f.capacity()["active_sessions"], 0);
    let output = f.start("replacement").output().unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn admission_stopped_resume_is_refused_without_changing_incarnation() {
    let f = Fixture::new(Some(1));
    let active = f.start("active").output().unwrap();
    assert!(active.status.success(), "{active:?}");
    write_resumable_session_record_with_agent_bin(
        &f.dir.path().join("state"),
        "stopped",
        "claude",
        "hs-claude-stopped",
        f.dir.path(),
        &["--resume", "resume-session-id"],
        Some(&f.provider),
    );
    fs::write(f.log.with_extension("log.killed"), "hs-claude-stopped\n").unwrap();
    // Model a stopped, never-launched recoverable runtime. A successful resume
    // replaces that stopped tmux identity, as in the shared resume fixtures.
    let tmux = f.dir.path().join("resume-tmux");
    fs::write(
        &tmux,
        format!(
            r#"#!/bin/sh
if [ "$1" = new-session ]; then : > "$AGENT_SESSION_FAKE_TMUX_LOG.killed"; fi
exec {} "$@"
"#,
            shell_words::quote(&f.tmux.to_string_lossy())
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let record = f.dir.path().join("state/sessions/stopped/session.json");
    let before = fs::read(&record).unwrap();
    let resumed = f
        .command()
        .env("AGENT_SESSION_TMUX_BIN", &tmux)
        .args(["resume", "stopped", "--format", "json"])
        .output()
        .unwrap();
    assert!(!resumed.status.success(), "resume exceeded capacity");
    refused(&envelope(&resumed));
    assert_eq!(fs::read(record).unwrap(), before);
    assert_eq!(f.starts(), 1);
    let already_active = f
        .command()
        .args(["resume", "active", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        already_active.status.success(),
        "already-running resume must not reserve another slot"
    );
    assert_eq!(f.starts(), 1);
}

#[test]
fn admission_zero_and_invalid_settings_do_not_launch() {
    let f = Fixture::new(Some(0));
    let output = f.start("zero").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let value = envelope(&output);
    assert_eq!(value["error"]["code"], "session-capacity-exceeded");
    assert_eq!(
        value["error"]["details"],
        json!({"max_active_sessions":0,"active_sessions":0,"retryable":true,"next_action":"wait-for-capacity","recovery":{"kind":"wait","condition":"active_sessions_below_limit","safe_to_retry":true}})
    );
    fs::write(
        f.dir.path().join("state/admission.json"),
        r#"{"max_active_sessions":-1}"#,
    )
    .unwrap();
    let invalid = f.start("invalid").output().unwrap();
    assert_eq!(invalid.status.code(), Some(65));
    assert_eq!(
        envelope(&invalid)["error"]["code"],
        "session-admission-config-invalid"
    );
    assert_eq!(envelope(&invalid)["error"]["details"]["retryable"], false);
    assert_eq!(
        envelope(&invalid)["error"]["details"]["next_action"],
        "repair-admission-config"
    );
    assert_eq!(f.starts(), 0);
    assert!(!f.dir.path().join("state/sessions/zero").exists());
    assert!(!f.dir.path().join("state/sessions/invalid").exists());
}

#[test]
fn admission_corrupt_occupancy_refuses_without_exposing_local_paths() {
    let f = Fixture::new(Some(1));
    let record_dir = f.dir.path().join("state/sessions/diagnostic-path-sentinel");
    fs::create_dir_all(&record_dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&record_dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(record_dir.join("session.json"), "{").unwrap();
    let diagnose = f
        .command()
        .args(["diagnose", "--format", "json"])
        .output()
        .unwrap();
    let start = f.start("refused").output().unwrap();
    for output in [diagnose, start] {
        for stream in [&output.stdout, &output.stderr] {
            let rendered = String::from_utf8_lossy(stream);
            assert!(
                !rendered.contains("diagnostic-path-sentinel"),
                "diagnostics must not expose the local record path"
            );
            assert!(
                !rendered.contains(f.dir.path().to_str().unwrap()),
                "diagnostics must not expose the local state root"
            );
        }
        assert_eq!(output.status.code(), Some(1));
        let error = envelope(&output)["error"].clone();
        assert_eq!(error["code"], "session-admission-state-unavailable");
        assert_eq!(error["details"]["retryable"], true);
        assert_eq!(error["details"]["next_action"], "retry-admission");
    }
    assert_eq!(f.starts(), 0);
    assert!(!f.dir.path().join("state/sessions/refused").exists());
}
