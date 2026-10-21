//! Lifecycle review: a generated session id is resolved after prompt input.
//!
//! Split out of `cli.rs` so the baselined file shrinks; the `#[test]` wrapper stays
//! there.

use super::*;
use pretty_assertions::{assert_eq, assert_ne};

pub(super) fn lifecycle_review_generated_id_is_resolved_after_prompt_input() {
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let state = tmp.path().join("state");
    let fifo = tmp.path().join("prompt.fifo");
    let cpath = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    let (tmux, tmux_log) = fake_tmux(tmp.path());
    let provider = fake_agent(tmp.path(), "codex");
    let first_second = jiff::Timestamp::now().as_second();
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    // Hermetic: the session tmp directory must not land in the caller's agent home.
    let _home = nils_test_support::hermetic::isolate(&mut command);
    let mut child = command
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "start",
            "--agent",
            "codex",
            "--title",
            "collision",
            "--cwd",
            tmp.path().to_str().unwrap(),
            "--prompt-file",
            fifo.to_str().unwrap(),
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--agent-bin",
            provider.to_str().unwrap(),
            "--paste-delay-ms",
            "0",
            "--format",
            "json",
        ])
        .env("AGENT_SESSION_FAKE_TMUX_LOG", &tmux_log)
        .env("NILS_TEST_PANE_LIFETIME_MS", "500")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut writer = loop {
        match fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo)
        {
            Ok(file) => break file,
            Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("prompt reader did not open");
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("{error}"),
        }
    };
    // The reader opens only after journal preallocation. Reserve every timestamp
    // it could have picked while input is held, without depending on wall-clock alignment.
    for second in first_second..=jiff::Timestamp::now().as_second() + 1 {
        let timestamp = jiff::Timestamp::from_second(second)
            .unwrap()
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%Y%m%d-%H%M%S")
            .to_string();
        fs::create_dir_all(
            state
                .join("sessions")
                .join(format!("{timestamp}-codex-collision")),
        )
        .unwrap();
    }
    writer.write_all(b"fixture prompt").unwrap();
    drop(writer);
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = value["data"]["id"].as_str().unwrap();
    assert!(id.ends_with("-codex-collision-1"), "{value}");
    let logs = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "logs",
            "--lifecycle",
            id,
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(logs.code, 0);
    let journal = logs.stdout_json();
    let rows = journal["data"]["records"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .filter(|r| r["operation"] == "start" && r["result"]["ok"] == true)
            .count(),
        1
    );
    let record: Value = serde_json::from_slice(
        &fs::read(state.join("sessions").join(id).join("session.json")).unwrap(),
    )
    .unwrap();
    let group = record["delete_tmux_identity"]["process_group_id"]
        .as_i64()
        .unwrap() as libc::pid_t;
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { libc::kill(-group, 0) } == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert_ne!(
        unsafe { libc::kill(-group, 0) },
        0,
        "bounded fixture must stop before cleanup"
    );
    let cleanup = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "delete",
            id,
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_FAKE_TMUX_LOG", tmux_log.to_str().unwrap()),
            ("AGENT_SESSION_FAKE_TMUX_ABSENT", "1"),
        ],
    );
    assert_eq!(
        cleanup.code,
        0,
        "{} {}",
        cleanup.stdout_text(),
        cleanup.stderr_text()
    );
}
