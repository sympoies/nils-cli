//! Same-session resume mailbox continuity (`sympoies/nils-cli#2302`).
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

use super::cli::{
    data, fake_agent, fake_tmux, run, sha256_hex, write_resumable_session_record_with_agent_bin,
};

/// Persisted unread peer mail must follow the same session across `resume`
/// (`sympoies/nils-cli#2302`): only the exact predecessor incarnation's unread,
/// unexpired mail moves, sender and creation time are preserved, the carry is
/// auditable, and acknowledgement stays single-use.
#[test]
fn resume_carries_unread_peer_mail_from_the_exact_predecessor_incarnation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).expect("repo dir");
    let (tmux_bin, tmux_log) = fake_tmux(tmp.path());
    let codex_bin = fake_agent(tmp.path(), "codex");
    let session = write_resumable_session_record_with_agent_bin(
        &state_dir,
        "recipient",
        "codex",
        "hs-codex-recipient",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
        Some(&codex_bin),
    );
    let record_path = session.join("session.json");
    let state_arg = state_dir.to_string_lossy().to_string();
    let tmux_arg = tmux_bin.to_string_lossy().to_string();
    let tmux_log_arg = tmux_log.to_string_lossy().to_string();
    let resume = || {
        let output = run(
            tmp.path(),
            &[
                "--state-dir",
                &state_arg,
                "resume",
                "recipient",
                "--tmux-bin",
                &tmux_arg,
                "--format",
                "json",
            ],
            &[
                ("AGENT_SESSION_FAKE_TMUX_LOG", &tmux_log_arg),
                ("AGENT_SESSION_FAKE_TMUX_HAS_SESSION", "0"),
                ("NILS_TEST_PANE_LIFETIME_MS", "500"),
            ],
        );
        assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
        let record: Value =
            serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
        let incarnation = record["runtime"]["launch_id"].as_str().unwrap().to_string();
        let capability = session.join(format!(
            "coordination/capability-{}",
            sha256_hex(&incarnation)
        ));
        (record, incarnation, capability)
    };
    // The runtime exits and its wrapper retires the broker, as after a host OOM.
    let stop = |record: &Value, capability: &Path| {
        let stopped = run(
            tmp.path(),
            &[
                "--state-dir",
                &state_arg,
                "broker",
                "stop",
                "--session",
                "recipient",
                "--capability-file",
                capability.to_str().unwrap(),
                "--format",
                "json",
            ],
            &[],
        );
        assert_eq!(stopped.code, 0, "{}", stopped.stderr_text());
        let group = record["delete_tmux_identity"]["process_group_id"]
            .as_i64()
            .unwrap() as libc::pid_t;
        let deadline = Instant::now() + Duration::from_secs(5);
        while unsafe { libc::kill(-group, 0) } == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert_ne!(unsafe { libc::kill(-group, 0) }, 0, "pane must stop");
        fs::remove_file(session.join("coordination/heartbeat")).expect("expire heartbeat");
    };
    let registry_path = state_dir.join("coordination/registry.json");
    let read_registry = || -> Value {
        serde_json::from_slice(&fs::read(&registry_path).expect("registry")).unwrap()
    };
    let message = |id: &str, sender: &str, recipient: &str, incarnation: &str, state: &str| {
        let now = jiff::Timestamp::now().as_second();
        json!({
            "schema_version": "agent-session.message.v1",
            "message_id": id,
            "sender_session_id": sender,
            "sender_incarnation": "sender-incarnation",
            "recipient_session_id": recipient,
            "recipient_incarnation": incarnation,
            "state": state,
            "revision": if state == "unread" { 1 } else { 2 },
            "reply_to": null,
            "reply_depth": 0,
            "created_at": "2000-01-01T00:00:07Z",
            "created_at_epoch": now - 120,
            "created_at_epoch_millis": (now - 120) * 1000,
            "expires_at": "2100-01-01T00:00:00Z",
            "expires_at_epoch": now + 3_600,
            "category": "handoff",
            "body_bytes": 4,
            "body": "body"
        })
    };

    let (record_a, incarnation_a, capability_a) = resume();
    stop(&record_a, &capability_a);
    let local = "00000000-0000-4000-8000-000000000001";
    let remote = "00000000-0000-4000-8000-000000000002";
    let already_read = "00000000-0000-4000-8000-000000000003";
    let expired = "00000000-0000-4000-8000-000000000004";
    let other_session = "00000000-0000-4000-8000-000000000005";
    let remote_sender = format!("remote:{}", json!(["peer-machine", "remote-peer"]));
    let mut registry = read_registry();
    let mut expired_message = message(expired, "peer", "recipient", &incarnation_a, "unread");
    expired_message["expires_at_epoch"] = json!(jiff::Timestamp::now().as_second() - 1);
    registry["messages"] = json!([
        message(local, "peer", "recipient", &incarnation_a, "unread"),
        message(
            remote,
            &remote_sender,
            "recipient",
            &incarnation_a,
            "unread"
        ),
        message(already_read, "peer", "recipient", &incarnation_a, "read"),
        expired_message,
        message(other_session, "peer", "bystander", &incarnation_a, "unread"),
    ]);
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&registry_path, fs::Permissions::from_mode(0o600)).unwrap();

    let (record_b, incarnation_b, capability_b) = resume();
    assert_ne!(incarnation_a, incarnation_b);
    let inbox = |capability: &Path| {
        let output = run(
            tmp.path(),
            &[
                "--state-dir",
                &state_arg,
                "message",
                "inbox",
                "--session",
                "recipient",
                "--capability-file",
                capability.to_str().unwrap(),
                "--format",
                "json",
            ],
            &[],
        );
        assert_eq!(output.code, 0, "{}", output.stderr_text());
        data(&output.stdout_json())["messages"].clone()
    };
    let rows = inbox(&capability_b);
    let ids: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["message_id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, vec![local.to_string(), remote.to_string()]);
    assert_eq!(rows[0]["sender"]["session_id"], "peer");
    assert_eq!(rows[0]["created_at"], "2000-01-01T00:00:07Z");
    assert_eq!(rows[0]["category"], "handoff");
    assert_eq!(rows[0]["state"], "unread");
    assert_eq!(rows[0]["revision"], 2);
    assert_eq!(rows[0]["resume_carry"]["carry_count"], 1);
    assert!(
        rows[0]["resume_carry"]["carried_at_epoch"]
            .as_i64()
            .is_some()
    );
    assert_eq!(rows[1]["sender"]["session_id"], "remote-peer");
    assert_eq!(rows[1]["sender"]["machine"], "peer-machine");
    assert!(
        !rows.to_string().contains(&incarnation_a),
        "inbox metadata must not disclose recipient incarnations"
    );

    let ack = |key: &str, revision: u64| {
        run(
            tmp.path(),
            &[
                "--state-dir",
                &state_arg,
                "message",
                "ack",
                "--session",
                "recipient",
                "--message",
                local,
                "--if-revision",
                &revision.to_string(),
                "--capability-file",
                capability_b.to_str().unwrap(),
                "--idempotency-key",
                key,
                "--format",
                "json",
            ],
            &[],
        )
    };
    let acked = ack("ack-carried-once", 2);
    assert_eq!(acked.code, 0, "{}", acked.stderr_text());
    assert_eq!(data(&acked.stdout_json())["state"], "acknowledged");
    let again = ack("ack-carried-twice", 2);
    assert_ne!(again.code, 0, "a carried message acknowledges once");
    assert_eq!(
        again.stdout_json()["error"]["code"],
        "message-revision-conflict"
    );

    let bystander = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "message",
            "inbox",
            "--session",
            "bystander",
            "--capability-file",
            capability_b.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[],
    );
    assert_ne!(bystander.code, 0, "another session cannot use the carry");

    let registry = read_registry();
    let stored = |id: &str| {
        registry["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["message_id"] == id)
            .cloned()
            .unwrap_or(Value::Null)
    };
    for (id, recipient, incarnation) in [
        (already_read, "recipient", &incarnation_a),
        (other_session, "bystander", &incarnation_a),
    ] {
        let retained = stored(id);
        assert_eq!(retained["recipient_session_id"], recipient, "{id}");
        assert_eq!(
            retained["recipient_incarnation"],
            incarnation.as_str(),
            "{id}"
        );
        assert!(retained.get("resume_carry").is_none(), "{id}");
    }
    let expired_row = stored(expired);
    assert!(
        expired_row.is_null() || expired_row["recipient_incarnation"] == incarnation_a.as_str(),
        "expired mail is never carried: {expired_row}"
    );
    let carried = stored(remote);
    assert_eq!(carried["recipient_incarnation"], incarnation_b.as_str());
    assert_eq!(carried["sender_session_id"], remote_sender.as_str());
    assert_eq!(carried["sender_incarnation"], "sender-incarnation");
    assert_eq!(
        carried["resume_carry"]["original_recipient_incarnation"],
        incarnation_a.as_str()
    );
    assert_eq!(
        carried["resume_carry"]["from_incarnation"],
        incarnation_a.as_str()
    );
    let audit = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "message",
            "audit",
            "--include-healthy",
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(audit.code, 0, "{}", audit.stderr_text());
    let audit = audit.stdout_json();
    let audited = data(&audit)["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["message_id"] == remote)
        .cloned()
        .expect("audited carried message");
    assert_eq!(
        audited["recipient"]["session_incarnation"],
        incarnation_b.as_str()
    );
    assert_eq!(audited["resume_carry"], carried["resume_carry"]);
    assert!(
        !audited["anomalies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|code| code == "incarnation-mismatch"),
        "{audited}"
    );

    // Repeated resume moves still-unread mail one exact step at a time.
    stop(&record_b, &capability_b);
    let (_, incarnation_c, capability_c) = resume();
    let rows = inbox(&capability_c);
    assert_eq!(rows.as_array().unwrap().len(), 1, "{rows}");
    assert_eq!(rows[0]["message_id"], remote);
    assert_eq!(rows[0]["resume_carry"]["carry_count"], 2);
    let registry = read_registry();
    let carried = registry["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["message_id"] == remote)
        .unwrap();
    assert_eq!(carried["recipient_incarnation"], incarnation_c.as_str());
    assert_eq!(
        carried["resume_carry"]["original_recipient_incarnation"],
        incarnation_a.as_str()
    );
    assert_eq!(
        carried["resume_carry"]["from_incarnation"],
        incarnation_b.as_str()
    );
    let acknowledged = registry["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["message_id"] == local)
        .unwrap();
    assert_eq!(acknowledged["state"], "acknowledged");
    assert_eq!(
        acknowledged["recipient_incarnation"],
        incarnation_b.as_str(),
        "terminal mail stays with the incarnation that handled it"
    );
}
