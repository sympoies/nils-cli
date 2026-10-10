//! Same-session resume mailbox continuity (`sympoies/nils-cli#2302`).
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

use super::cli::{
    data, fake_agent, fake_tmux, run, sha256_hex, write_resumable_session_record_with_agent_bin,
};

const RECIPIENT: &str = "recipient";

/// One resumable managed session driven through the real CLI with fake tmux.
struct Fixture {
    tmp: tempfile::TempDir,
    state_dir: PathBuf,
    cwd: PathBuf,
    session: PathBuf,
    codex_bin: PathBuf,
    tmux_arg: String,
    tmux_log_arg: String,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let state_dir = tmp.path().join("state");
        let cwd = tmp.path().join("repo");
        fs::create_dir_all(&cwd).expect("repo dir");
        let (tmux_bin, tmux_log) = fake_tmux(tmp.path());
        let codex_bin = fake_agent(tmp.path(), "codex");
        let mut fixture = Self {
            session: PathBuf::new(),
            tmux_arg: tmux_bin.to_string_lossy().to_string(),
            tmux_log_arg: tmux_log.to_string_lossy().to_string(),
            tmp,
            state_dir,
            cwd,
            codex_bin,
        };
        fixture.session = fixture.write_record();
        fixture
    }

    fn write_record(&self) -> PathBuf {
        write_resumable_session_record_with_agent_bin(
            &self.state_dir,
            RECIPIENT,
            "codex",
            "hs-codex-recipient",
            &self.cwd,
            &[
                "resume",
                "resume-session-id",
                "--cd",
                self.cwd.to_str().unwrap(),
                "--no-alt-screen",
            ],
            Some(&self.codex_bin),
        )
    }

    fn state_arg(&self) -> String {
        self.state_dir.to_string_lossy().to_string()
    }

    fn cli(&self, args: &[&str], envs: &[(&str, &str)]) -> nils_test_support::cmd::CmdOutput {
        let state_arg = self.state_arg();
        let mut full = vec!["--state-dir", state_arg.as_str()];
        full.extend_from_slice(args);
        run(self.tmp.path(), &full, envs)
    }

    /// Resume and return the new record, incarnation and capability path.
    fn resume(&self) -> (Value, String, PathBuf) {
        let output = self.cli(
            &[
                "resume",
                RECIPIENT,
                "--tmux-bin",
                &self.tmux_arg,
                "--format",
                "json",
            ],
            &[
                ("AGENT_SESSION_FAKE_TMUX_LOG", &self.tmux_log_arg),
                ("AGENT_SESSION_FAKE_TMUX_HAS_SESSION", "0"),
                ("NILS_TEST_PANE_LIFETIME_MS", "500"),
            ],
        );
        assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
        let record: Value =
            serde_json::from_str(&fs::read_to_string(self.session.join("session.json")).unwrap())
                .unwrap();
        let incarnation = record["runtime"]["launch_id"].as_str().unwrap().to_string();
        let capability = self.session.join(format!(
            "coordination/capability-{}",
            sha256_hex(&incarnation)
        ));
        (record, incarnation, capability)
    }

    /// The runtime exits and its wrapper retires the broker, as after a host OOM.
    fn stop(&self, record: &Value, capability: &Path) {
        let stopped = self.cli(
            &[
                "broker",
                "stop",
                "--session",
                RECIPIENT,
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
        fs::remove_file(self.session.join("coordination/heartbeat")).expect("expire heartbeat");
    }

    fn registry_path(&self) -> PathBuf {
        self.state_dir.join("coordination/registry.json")
    }

    fn registry(&self) -> Value {
        serde_json::from_slice(&fs::read(self.registry_path()).expect("registry")).unwrap()
    }

    fn stored(&self, id: &str) -> Value {
        self.registry()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["message_id"] == id)
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn seed(&self, messages: Vec<Value>) {
        let mut registry = self.registry();
        registry["messages"] = Value::Array(messages);
        let path = self.registry_path();
        fs::write(&path, serde_json::to_vec_pretty(&registry).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn inbox(&self, capability: &Path) -> Value {
        let output = self.cli(
            &[
                "message",
                "inbox",
                "--session",
                RECIPIENT,
                "--capability-file",
                capability.to_str().unwrap(),
                "--format",
                "json",
            ],
            &[],
        );
        assert_eq!(output.code, 0, "{}", output.stderr_text());
        data(&output.stdout_json())["messages"].clone()
    }

    /// Write `<state>/orchestration/registry.json` privately.
    fn write_orchestration_registry(&self, bytes: &[u8]) {
        let root = self.state_dir.join("orchestration");
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.join("registry.json");
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn message(id: &str, sender: &str, recipient: &str, incarnation: &str, state: &str) -> Value {
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
}

fn message_ids(rows: &Value) -> Vec<String> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|row| row["message_id"].as_str().unwrap().to_string())
        .collect()
}

/// Persisted unread peer mail must follow the same session across `resume`
/// (`sympoies/nils-cli#2302`): only the exact predecessor incarnation's unread,
/// unexpired mail moves, sender and creation time are preserved, the carry is
/// auditable, and acknowledgement stays single-use.
#[test]
fn resume_carries_unread_peer_mail_from_the_exact_predecessor_incarnation() {
    let fixture = Fixture::new();
    let (record_a, incarnation_a, capability_a) = fixture.resume();
    fixture.stop(&record_a, &capability_a);
    let local = "00000000-0000-4000-8000-000000000001";
    let remote = "00000000-0000-4000-8000-000000000002";
    let already_read = "00000000-0000-4000-8000-000000000003";
    let expired = "00000000-0000-4000-8000-000000000004";
    let other_session = "00000000-0000-4000-8000-000000000005";
    let remote_sender = format!("remote:{}", json!(["peer-machine", "remote-peer"]));
    let mut expired_message = message(expired, "peer", RECIPIENT, &incarnation_a, "unread");
    expired_message["expires_at_epoch"] = json!(jiff::Timestamp::now().as_second() - 1);
    fixture.seed(vec![
        message(local, "peer", RECIPIENT, &incarnation_a, "unread"),
        message(remote, &remote_sender, RECIPIENT, &incarnation_a, "unread"),
        message(already_read, "peer", RECIPIENT, &incarnation_a, "read"),
        expired_message,
        message(other_session, "peer", "bystander", &incarnation_a, "unread"),
    ]);

    let (record_b, incarnation_b, capability_b) = fixture.resume();
    assert_ne!(incarnation_a, incarnation_b);
    let rows = fixture.inbox(&capability_b);
    assert_eq!(
        message_ids(&rows),
        vec![local.to_string(), remote.to_string()]
    );
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
        fixture.cli(
            &[
                "message",
                "ack",
                "--session",
                RECIPIENT,
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

    let bystander = fixture.cli(
        &[
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

    for (id, recipient) in [(already_read, RECIPIENT), (other_session, "bystander")] {
        let retained = fixture.stored(id);
        assert_eq!(retained["recipient_session_id"], recipient, "{id}");
        assert_eq!(
            retained["recipient_incarnation"],
            incarnation_a.as_str(),
            "{id}"
        );
        assert!(retained.get("resume_carry").is_none(), "{id}");
    }
    let expired_row = fixture.stored(expired);
    assert!(
        expired_row.is_null() || expired_row["recipient_incarnation"] == incarnation_a.as_str(),
        "expired mail is never carried: {expired_row}"
    );
    let carried = fixture.stored(remote);
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
    let audit = fixture.cli(
        &["message", "audit", "--include-healthy", "--format", "json"],
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
    fixture.stop(&record_b, &capability_b);
    let (_, incarnation_c, capability_c) = fixture.resume();
    let rows = fixture.inbox(&capability_c);
    assert_eq!(message_ids(&rows), vec![remote.to_string()]);
    assert_eq!(rows[0]["resume_carry"]["carry_count"], 2);
    let carried = fixture.stored(remote);
    assert_eq!(carried["recipient_incarnation"], incarnation_c.as_str());
    assert_eq!(
        carried["resume_carry"]["original_recipient_incarnation"],
        incarnation_a.as_str()
    );
    assert_eq!(
        carried["resume_carry"]["from_incarnation"],
        incarnation_b.as_str()
    );
    let acknowledged = fixture.stored(local);
    assert_eq!(acknowledged["state"], "acknowledged");
    assert_eq!(
        acknowledged["recipient_incarnation"],
        incarnation_b.as_str(),
        "terminal mail stays with the incarnation that handled it"
    );
}

/// A deleted session's stopped broker stays registered; a new session that
/// reuses its ID is a different lineage and must not inherit its unread mail.
#[test]
fn resume_does_not_carry_a_deleted_sessions_mail_into_a_recreated_session() {
    let fixture = Fixture::new();
    let (record_a, incarnation_a, capability_a) = fixture.resume();
    fixture.stop(&record_a, &capability_a);
    let stale = "00000000-0000-4000-8000-000000000011";
    fixture.seed(vec![message(
        stale,
        "peer",
        RECIPIENT,
        &incarnation_a,
        "unread",
    )]);
    let deleted = fixture.cli(
        &[
            "delete",
            RECIPIENT,
            "--tmux-bin",
            &fixture.tmux_arg,
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_FAKE_TMUX_LOG", &fixture.tmux_log_arg),
            ("AGENT_SESSION_FAKE_TMUX_HAS_SESSION", "0"),
        ],
    );
    assert_eq!(deleted.code, 0, "stderr={}", deleted.stderr_text());
    assert_eq!(data(&deleted.stdout_json())["deleted"], true);
    assert_eq!(
        fixture.registry()["brokers"][RECIPIENT]["incarnation"],
        incarnation_a.as_str(),
        "the deleted session's stopped broker stays registered"
    );

    // Recreate the ID as a new session, created now.
    let session = fixture.write_record();
    let record_path = session.join("session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    record["created_at"] = json!(jiff::Timestamp::now().to_string());
    fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();

    let (_, incarnation_b, capability_b) = fixture.resume();
    assert_ne!(incarnation_a, incarnation_b);
    let rows = fixture.inbox(&capability_b);
    assert_eq!(message_ids(&rows), Vec::<String>::new());
    let retained = fixture.stored(stale);
    assert_eq!(retained["recipient_incarnation"], incarnation_a.as_str());
    assert!(retained.get("resume_carry").is_none(), "{retained}");
}

/// Guidance from a Main Agent primary manager of this worker stays with the
/// predecessor for Main Agent's own reconcile/quarantine; other peer mail moves.
#[test]
fn resume_leaves_main_agent_controller_guidance_with_the_predecessor() {
    let fixture = Fixture::new();
    let template = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/orchestration/registry-v2-populated.json"),
    )
    .unwrap();
    let registry = template
        .replace("\"main-v2\"", "\"controller\"")
        .replace("\"worker-v2\"", &format!("\"{RECIPIENT}\""));
    let parsed: Value = serde_json::from_str(&registry).unwrap();
    assert_eq!(
        parsed["assignments"]["assignment-v2"]["primary_manager"]["session_id"],
        "controller"
    );
    assert_eq!(
        parsed["assignments"]["assignment-v2"]["worker"]["session_id"],
        RECIPIENT
    );
    fixture.write_orchestration_registry(registry.as_bytes());

    let (record_a, incarnation_a, capability_a) = fixture.resume();
    fixture.stop(&record_a, &capability_a);
    let guidance = "00000000-0000-4000-8000-000000000021";
    let peer = "00000000-0000-4000-8000-000000000022";
    fixture.seed(vec![
        message(guidance, "controller", RECIPIENT, &incarnation_a, "unread"),
        message(peer, "peer", RECIPIENT, &incarnation_a, "unread"),
    ]);

    let (_, incarnation_b, capability_b) = fixture.resume();
    let rows = fixture.inbox(&capability_b);
    assert_eq!(message_ids(&rows), vec![peer.to_string()]);
    let retained = fixture.stored(guidance);
    assert_eq!(retained["state"], "unread");
    assert_eq!(retained["recipient_incarnation"], incarnation_a.as_str());
    assert!(retained.get("resume_carry").is_none(), "{retained}");
    assert_eq!(
        fixture.stored(peer)["recipient_incarnation"],
        incarnation_b.as_str()
    );
}

/// When the Main Agent relationship cannot be read, nothing is carried.
#[test]
fn resume_carries_nothing_when_the_orchestration_registry_is_unreadable() {
    let fixture = Fixture::new();
    fixture.write_orchestration_registry(b"{ not an orchestration registry");

    let (record_a, incarnation_a, capability_a) = fixture.resume();
    fixture.stop(&record_a, &capability_a);
    let peer = "00000000-0000-4000-8000-000000000031";
    fixture.seed(vec![message(
        peer,
        "peer",
        RECIPIENT,
        &incarnation_a,
        "unread",
    )]);

    let (_, _, capability_b) = fixture.resume();
    assert_eq!(
        message_ids(&fixture.inbox(&capability_b)),
        Vec::<String>::new()
    );
    let retained = fixture.stored(peer);
    assert_eq!(retained["state"], "unread");
    assert_eq!(retained["recipient_incarnation"], incarnation_a.as_str());
}
