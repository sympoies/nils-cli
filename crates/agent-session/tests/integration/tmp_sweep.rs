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

fn entries(list: &Value) -> Vec<(String, String)> {
    list.as_array()
        .expect("entry list")
        .iter()
        .map(|entry| {
            (
                entry["id"].as_str().expect("id").to_string(),
                entry["reason"].as_str().expect("reason").to_string(),
            )
        })
        .collect()
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
    assert_eq!(
        entries(&envelope["data"]["removed"]),
        vec![
            ("orphan-1".to_string(), "orphan".to_string()),
            ("vanished-1".to_string(), "orphan".to_string())
        ]
    );
    assert_eq!(envelope["data"]["retained"], json!([]));
    assert!(!home.join("tmp/orphan-1").exists());
    assert!(!home.join("tmp/vanished-1").exists());
    assert!(home.join("tmp/other-1").is_dir());
    assert!(home.join("tmp/not-ours").is_dir());
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
    assert!(text.contains("orphan-2 (grace)"), "{text}");
    assert!(home.join("tmp/orphan-2/scratch.txt").is_file());
}
