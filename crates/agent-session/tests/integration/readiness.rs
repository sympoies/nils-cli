use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use nils_test_support::cmd::CmdOutput;
use pretty_assertions::assert_eq;
use serde_json::json;

use super::coordination::{
    capability, data, digest, run_with_env, seed_brokers, write_private_json,
};

#[test]
fn managed_session_readiness_authenticates_without_orchestration() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[(
            "ready",
            "ready-incarnation",
            "ready-private-capability-material-0001",
        )],
    );
    let capability_file = capability(&state_dir, "ready");
    let checkpoint_file = state_dir.join(format!(
        "sessions/ready/coordination/main-agent-checkpoint-{}.json",
        digest("ready-incarnation")
    ));
    let before = fs::read(&checkpoint_file).expect("checkpoint");
    let ready = run_with_env(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "readiness",
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_CAPABILITY_FILE", &capability_file),
            (
                "AGENT_SESSION_CHECKPOINT_FILE",
                checkpoint_file.to_str().expect("checkpoint"),
            ),
        ],
    );
    assert_eq!(
        ready.code,
        0,
        "stdout={} stderr={}",
        ready.stdout_text(),
        ready.stderr_text()
    );
    assert_eq!(
        ready.stdout_json()["schema_version"],
        "cli.agent-session.readiness.v1"
    );
    assert_eq!(
        data(&ready),
        json!({
            "schema_version": "agent-session.runtime-readiness.v1",
            "ready": true,
            "session_id": "ready",
            "session_incarnation": "ready-incarnation",
            "checkpoint_file": checkpoint_file.to_str().expect("checkpoint"),
        })
    );
    assert!(
        !ready
            .stdout_text()
            .contains("ready-private-capability-material-0001")
    );
    assert_eq!(
        fs::read(&checkpoint_file).expect("checkpoint after readiness"),
        before
    );
    assert!(
        !state_dir.join("orchestration").exists(),
        "readiness must not create mode state"
    );
}

fn assert_readiness_rejected(output: &CmdOutput, fixture_root: &Path, code: &str, case: &str) {
    assert_eq!(output.code, 65, "{case}: {}", output.stdout_text());
    let envelope = output.stdout_json();
    assert_eq!(
        envelope["schema_version"], "cli.agent-session.readiness.v1",
        "{case}"
    );
    assert_eq!(envelope["ok"], false, "{case}");
    assert_eq!(envelope["error"]["code"], code, "{case}");
    assert_eq!(envelope["error"]["details"]["retryable"], false, "{case}");
    assert_eq!(
        envelope["error"]["details"]["next_action"], "resume-or-restart-managed-session",
        "{case}"
    );
    assert_eq!(
        envelope["error"]["details"]["recovery"]["action"], "resume-or-restart-managed-session",
        "{case}"
    );
    for text in [output.stdout_text(), output.stderr_text()] {
        assert!(
            !text.contains(fixture_root.to_str().unwrap()),
            "{case}: no supplied private path may leak"
        );
        assert!(
            !text.contains("private-capability-material"),
            "{case}: no capability material may leak"
        );
    }
}

#[test]
fn managed_session_readiness_rejects_untrusted_checkpoints() {
    for case in [
        "unset",
        "empty",
        "wrong-path",
        "missing",
        "public",
        "symlink",
        "hardlink",
        "directory",
    ] {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let state_dir = tmp.path().join("state");
        fs::create_dir(&state_dir).expect("state");
        seed_brokers(
            &state_dir,
            &[(
                "ready",
                "ready-incarnation",
                "ready-private-capability-material-0001",
            )],
        );
        let capability_file = capability(&state_dir, "ready");
        let checkpoint_file = state_dir.join(format!(
            "sessions/ready/coordination/main-agent-checkpoint-{}.json",
            digest("ready-incarnation")
        ));
        let other = tmp.path().join("other-checkpoint");
        fs::write(&other, b"preserve checkpoint content").expect("other");
        fs::set_permissions(&other, fs::Permissions::from_mode(0o600)).expect("other mode");
        match case {
            "missing" => fs::remove_file(&checkpoint_file).unwrap(),
            "public" => {
                fs::set_permissions(&checkpoint_file, fs::Permissions::from_mode(0o644)).unwrap()
            }
            "symlink" => {
                fs::remove_file(&checkpoint_file).unwrap();
                std::os::unix::fs::symlink(&other, &checkpoint_file).unwrap();
            }
            "hardlink" => {
                fs::remove_file(&checkpoint_file).unwrap();
                fs::hard_link(&other, &checkpoint_file).unwrap();
            }
            "directory" => {
                fs::remove_file(&checkpoint_file).unwrap();
                fs::create_dir(&checkpoint_file).unwrap();
            }
            _ => {}
        }
        let supplied = match case {
            "empty" => "",
            "wrong-path" => other.to_str().unwrap(),
            _ => checkpoint_file.to_str().unwrap(),
        };
        let mut env = vec![("AGENT_SESSION_CAPABILITY_FILE", capability_file.as_str())];
        if case != "unset" {
            env.push(("AGENT_SESSION_CHECKPOINT_FILE", supplied));
        }
        let rejected = run_with_env(
            tmp.path(),
            &[
                "--state-dir",
                state_dir.to_str().unwrap(),
                "readiness",
                "--format",
                "json",
            ],
            &env,
        );
        assert_readiness_rejected(
            &rejected,
            tmp.path(),
            "runtime-checkpoint-unavailable",
            case,
        );
        assert_eq!(fs::read(&other).unwrap(), b"preserve checkpoint content");
    }
}

#[test]
fn managed_session_readiness_rejects_unverified_incarnations() {
    for case in [
        "unset",
        "empty",
        "public",
        "revoked",
        "stale-capability",
        "stale-heartbeat",
        "not-ready",
        "record-incarnation",
    ] {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let state_dir = tmp.path().join("state");
        fs::create_dir(&state_dir).expect("state");
        seed_brokers(
            &state_dir,
            &[(
                "ready",
                "ready-incarnation",
                "ready-private-capability-material-0001",
            )],
        );
        let capability_file = capability(&state_dir, "ready");
        let checkpoint_file = state_dir.join(format!(
            "sessions/ready/coordination/main-agent-checkpoint-{}.json",
            digest("ready-incarnation")
        ));
        let copied = tmp.path().join("copied-capability");
        fs::copy(&capability_file, &copied).unwrap();
        fs::set_permissions(&copied, fs::Permissions::from_mode(0o600)).unwrap();
        match case {
            "public" => fs::set_permissions(&copied, fs::Permissions::from_mode(0o644)).unwrap(),
            "revoked" => fs::remove_file(&capability_file).unwrap(),
            "stale-capability" => {
                fs::write(&copied, "previous-incarnation-private-capability-material").unwrap()
            }
            "stale-heartbeat" => fs::write(
                checkpoint_file.parent().unwrap().join("heartbeat"),
                "ready-incarnation:1\n",
            )
            .unwrap(),
            "not-ready" => {
                let path = state_dir.join("coordination/registry.json");
                let mut registry: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                registry["brokers"]["ready"]["state"] = json!("stopped");
                write_private_json(&path, &registry);
            }
            "record-incarnation" => {
                let path = state_dir.join("sessions/ready/session.json");
                let mut record: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                record["runtime"]["launch_id"] = json!("replacement-incarnation");
                write_private_json(&path, &record);
            }
            _ => {}
        }
        let mut env = vec![(
            "AGENT_SESSION_CHECKPOINT_FILE",
            checkpoint_file.to_str().unwrap(),
        )];
        if case != "unset" {
            env.push((
                "AGENT_SESSION_CAPABILITY_FILE",
                if case == "empty" {
                    ""
                } else {
                    copied.to_str().unwrap()
                },
            ));
        }
        let rejected = run_with_env(
            tmp.path(),
            &[
                "--state-dir",
                state_dir.to_str().unwrap(),
                "readiness",
                "--format",
                "json",
            ],
            &env,
        );
        assert_readiness_rejected(&rejected, tmp.path(), "coordination-unauthorized", case);
    }
}
