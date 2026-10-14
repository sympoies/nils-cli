use super::cli::{fake_tmux, run, tmux_calls, write_resumable_session_record_with_agent_bin};
use std::fs::{self, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;
use serde_json::Value;

struct Readers {
    release: Option<Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for Readers {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn block_provisioning(state: &Path, launched: &Path, ready: &Path) -> Readers {
    let state = state.to_owned();
    let launched = launched.to_owned();
    let ready = ready.to_owned();
    let (release, wait) = mpsc::channel();
    let worker = thread::spawn(move || {
        let started = Instant::now();
        while !launched.exists() {
            assert!(started.elapsed() < Duration::from_secs(10));
            thread::sleep(Duration::from_millis(10));
        }
        let locks: Vec<_> = (0..4)
            .map(|_| {
                let lock = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .mode(0o600)
                    .open(state.join("coordination/registry.lock"))
                    .unwrap();
                // SAFETY: the fixture owns each live lock descriptor.
                assert_eq!(
                    unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) },
                    0
                );
                lock
            })
            .collect();
        fs::write(ready, b"ready\n").unwrap();
        // Keep contention present through the returned resume outcome, rather
        // than allowing a second rollback wait to succeed by timing accident.
        let _ = wait.recv_timeout(Duration::from_secs(30));
        drop(locks);
    });
    Readers {
        release: Some(release),
        worker: Some(worker),
    }
}

#[test]
fn resume_provision_contention_restores_prior_state_and_retry_action() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    let (tmux, log) = fake_tmux(tmp.path());
    let backend = tmp.path().join("tmux-backend");
    fs::rename(&tmux, &backend).unwrap();
    let launched = tmp.path().join("runtime-launched");
    let ready = tmp.path().join("readers-ready");
    fs::write(
        &tmux,
        format!(
            "#!/bin/sh\n{} \"$@\"\nstatus=$?\nif [ \"$1\" = new-session ] && [ \"$status\" = 0 ]; then\n  : > {}\n  attempts=0\n  while [ ! -f {} ]; do\n    attempts=$((attempts + 1))\n    [ \"$attempts\" -lt 500 ] || exit 42\n    sleep 0.01\n  done\nfi\nexit \"$status\"\n",
            shell_words::quote(&backend.to_string_lossy()),
            shell_words::quote(&launched.to_string_lossy()),
            shell_words::quote(&ready.to_string_lossy()),
        ),
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let provider = tmp.path().join("fixture-provider");
    let provider_calls = tmp.path().join("provider-calls");
    fs::write(
        &provider,
        format!(
            "#!/bin/sh\nprintf 'called\\n' >> {}\n",
            shell_words::quote(&provider_calls.to_string_lossy())
        ),
    )
    .unwrap();
    fs::set_permissions(&provider, fs::Permissions::from_mode(0o700)).unwrap();
    let session = write_resumable_session_record_with_agent_bin(
        &state,
        "resume-contention",
        "codex",
        "fixture-resume-contention",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
        Some(&provider),
    );
    let artifacts = [
        ".startup-stage",
        ".startup-failure",
        ".startup-diagnostic.log",
        ".runtime-exit-status",
    ];
    for name in artifacts {
        fs::write(session.join(name), b"prior fixture state\n").unwrap();
    }
    let record_path = session.join("session.json");
    let before: agent_session::SessionRecord =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    let _readers = block_provisioning(&state, &launched, &ready);
    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "resume",
            "resume-contention",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap()),
            ("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1"),
            ("AGENT_SESSION_FAKE_TMUX_ABSENT_AFTER_KILL", "1"),
        ],
    );
    assert_eq!(output.code, 1);
    let error = output.stdout_json();
    let after: agent_session::SessionRecord =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
    for name in artifacts {
        assert_eq!(
            fs::read(session.join(name)).unwrap(),
            b"prior fixture state\n"
        );
        assert!(!session.join(format!("{name}.resume-backup")).exists());
    }
    assert!(!session.join("activity.json").exists());
    assert!(!session.join("activity.replay.bin").exists());
    assert!(!agent_session::launch_gate_path(&state, &before).exists());
    assert!(!provider_calls.exists());
    let calls = tmux_calls(&log);
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.first().is_some_and(|arg| arg == "new-session"))
            .count(),
        1
    );
    let registry_path = state.join("coordination/registry.json");
    if registry_path.exists() {
        let registry: Value = serde_json::from_slice(&fs::read(registry_path).unwrap()).unwrap();
        assert!(registry["brokers"].get("resume-contention").is_none());
    }
    assert_eq!(error["error"]["code"], "coordination-lock-timeout");
    assert_eq!(error["error"]["details"]["retryable"], true);
    assert_eq!(error["error"]["details"]["next_action"], "retry-resume");
    assert_eq!(
        error["error"]["details"]["recovery"]["outcome"],
        "prior_restored"
    );
}
