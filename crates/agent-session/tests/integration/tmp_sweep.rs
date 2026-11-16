//! `agent-session tmp sweep`: reclaim per-session `TMPDIR` directories under
//! `AGENT_HOME/tmp` (agent write-location contract, class `tmp`).

use std::fs;
use std::path::Path;

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const OWNER_FILE: &str = ".agent-session-tmp";

fn sweep(home: &Path, state_dir: &Path, args: &[&str]) -> CmdOutput {
    let mut full = vec![
        "--state-dir",
        state_dir.to_str().expect("state dir UTF-8"),
        "tmp",
        "sweep",
    ];
    full.extend_from_slice(args);
    let options = CmdOptions::new()
        .with_env("AGENT_HOME", home.to_str().expect("home UTF-8"))
        .with_env_remove_many(&["AGENT_SESSION_TMP_GRACE_SECS", "XDG_STATE_HOME"]);
    run_resolved("agent-session", &full, &options)
}

/// A tmp directory as `agent-session` creates it for a session of `state_dir`.
fn owned_dir(home: &Path, state_dir: &Path, id: &str) {
    let dir = home.join("tmp").join(id);
    fs::create_dir_all(&dir).expect("session tmp dir");
    let owner = json!({
        "schema_version": "agent-session.session-tmp.v1",
        "session_id": id,
        "state_dir": state_dir.to_str().expect("state dir UTF-8"),
        "stopped_at_epoch": null,
    });
    fs::write(dir.join(OWNER_FILE), owner.to_string()).expect("owner file");
    fs::write(dir.join("scratch.txt"), "data").expect("scratch file");
}

#[test]
fn sweep_reports_and_removes_orphans_but_not_foreign_dirs() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let home = temp.path().join("home");
    let state_dir = temp.path().join("state");
    fs::create_dir_all(&state_dir).expect("state dir");
    owned_dir(&home, &state_dir, "orphan-1");
    let other_state = temp.path().join("other-state");
    fs::create_dir_all(&other_state).expect("other state dir");
    owned_dir(&home, &other_state, "other-1");
    owned_dir(&home, &temp.path().join("vanished-state"), "vanished-1");
    fs::create_dir_all(home.join("tmp/not-ours")).expect("foreign dir");

    let output = sweep(
        &home,
        &state_dir,
        &["--grace-secs", "0", "--format", "json"],
    );

    assert!(output.success(), "{}", output.stderr_text());
    let envelope = output.stdout_json();
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["schema_version"], "cli.agent-session.tmp-sweep.v1");
    assert!(!home.join("tmp/orphan-1").exists());
    assert!(home.join("tmp/vanished-1").is_dir());
    assert!(home.join("tmp/other-1").is_dir());
    assert!(home.join("tmp/not-ours").is_dir());
}

fn record(state: &Path, id: &str) {
    let dir = state.join("sessions").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("session.json"),
        json!({
            "schema_version": agent_session::SESSION_DOCUMENT_VERSION,
            "id": id, "agent": "codex", "mode": "interactive", "title": null,
            "cwd": state, "tmux_session": id, "prompt_file": null, "log_file": null,
            "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
            "runtime": {"kind":"tmux", "tmux_session":id, "generation":1,
                "started_at":"2030-01-01T00:00:00Z", "launch_id":"fixture-runtime"}
        })
        .to_string(),
    )
    .unwrap();
}

fn mixed_root(home: &Path, state: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(state).unwrap();
    for id in [
        "live-canary",
        "dead-canary",
        "unknown-canary",
        "fresh-canary",
    ] {
        owned_dir(home, state, id);
        record(state, id);
        if id != "fresh-canary" {
            let path = home.join("tmp").join(id).join(OWNER_FILE);
            let mut owner: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            owner["stopped_at_epoch"] = json!(1);
            fs::write(path, owner.to_string()).unwrap();
        }
    }
    owned_dir(home, &state.join("vanished-foreign"), "foreign-canary");
    let tmux = home.join("fake-tmux");
    fs::write(&tmux, "#!/bin/sh\ncase \"$3\" in\n=live-canary) exit 0;;\n=unknown-canary) exit 2;;\n*) echo \"can't find session: fixture\" >&2; exit 1;;\nesac\n").unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    tmux
}

#[test]
fn sweep_mixed_root_reclaims_only_expired_dead_and_reports_counts_without_values() {
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let tmux = mixed_root(&home, &state);
    let output = sweep(
        &home,
        &state,
        &[
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--grace-secs",
            "3600",
            "--format",
            "json",
        ],
    );
    assert!(output.success(), "{}", output.stderr_text());
    assert!(!home.join("tmp/dead-canary").exists());
    for id in [
        "live-canary",
        "unknown-canary",
        "fresh-canary",
        "foreign-canary",
    ] {
        assert!(
            home.join("tmp").join(id).join("scratch.txt").is_file(),
            "{id}"
        );
    }
    let report = output.stdout_json()["data"].clone();
    assert_eq!(report["removed"], 1);
    let text = output.stdout_text();
    assert!(!text.contains("canary"), "{text}");
    assert!(!text.contains(home.to_str().unwrap()), "{text}");
}

#[test]
fn serve_sweeps_expired_dead_without_a_session_launch() {
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let tmux = mixed_root(&home, &state);
    let mut command = super::hermetic_provider_command(temp.path());
    command
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "serve",
            "--bind",
            "127.0.0.1:0",
            "--machine",
            "fixture",
            "--tmux-bin",
            tmux.to_str().unwrap(),
        ])
        .env("AGENT_HOME", &home)
        .env("AGENT_SESSION_TMP_GRACE_SECS", "3600")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().unwrap();
    let started = Instant::now();
    while home.join("tmp/dead-canary").exists() && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        !home.join("tmp/dead-canary").exists(),
        "serve never triggered retention"
    );
    for id in [
        "live-canary",
        "unknown-canary",
        "fresh-canary",
        "foreign-canary",
    ] {
        assert!(
            home.join("tmp").join(id).join("scratch.txt").is_file(),
            "{id}"
        );
    }
}

#[test]
fn sweep_keeps_a_fresh_orphan_inside_the_grace_period_and_renders_text() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let home = temp.path().join("home");
    let state_dir = temp.path().join("state");
    fs::create_dir_all(&state_dir).expect("state dir");
    owned_dir(&home, &state_dir, "orphan-2");

    let output = sweep(&home, &state_dir, &["--grace-secs", "3600"]);

    assert!(output.success(), "{}", output.stderr_text());
    let text = output.stdout_text();
    assert!(text.contains("removed: 0"), "{text}");
    assert!(text.contains("retained: 1"), "{text}");
    assert!(!text.contains("orphan-2"), "{text}");
    assert!(home.join("tmp/orphan-2/scratch.txt").is_file());
}
