//! Mailbox read events: the recording boundary of `mailbox-read-events-v1.md`.
use super::super::{capability_path, run_message};
use super::*;
use crate::cli::{Cli, Command};
use clap::Parser;
use nils_common::cli_contract::OutputFormat;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;

const PROVIDER_SESSION: &str = "0a1b2c3d-0000-4000-8000-000000000001";
const TRANSCRIPT: &str = "{\"type\":\"user\",\"sessionId\":\"0a1b2c3d-0000-4000-8000-000000000001\",\"cwd\":\"/work/example\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n";

struct Rig {
    _temp: tempfile::TempDir,
    _config: tempfile::TempDir,
    _env: EnvGuard,
    context: CliContext,
    transcript: std::path::PathBuf,
}

impl Rig {
    /// A recipient bound to a Claude conversation; `with_transcript` controls
    /// whether the exact provider transcript exists for the producer to find.
    fn new(lock: &GlobalStateLock, with_transcript: bool) -> Self {
        let (temp, context) = tests::fixture();
        let config = tempfile::TempDir::new().unwrap();
        let project = config.path().join("projects/example");
        std::fs::create_dir_all(&project).unwrap();
        let transcript = project.join(format!("{PROVIDER_SESSION}.jsonl"));
        if with_transcript {
            std::fs::write(&transcript, TRANSCRIPT).unwrap();
        }
        let path = context.state_dir.join("sessions/recipient/session.json");
        let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        record["agent"] = json!("claude");
        record["provider_resume"] = json!({
            "provider": "claude", "session_id": PROVIDER_SESSION,
            "captured_at": "2030-01-01T00:00:00Z", "capture_method": "claude-explicit-session-id",
            "resume_args": ["--resume", PROVIDER_SESSION],
        });
        write_atomic(
            &path,
            &serde_json::to_vec(&record).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        let env = EnvGuard::set(lock, "CLAUDE_CONFIG_DIR", config.path().to_str().unwrap());
        Self {
            _temp: temp,
            _config: config,
            _env: env,
            context,
            transcript,
        }
    }

    fn deliver(&self, body: &str) -> String {
        let mut envelope = tests::envelope();
        envelope.body = body.into();
        envelope.body_sha256 = digest_bytes(body.as_bytes());
        receive(&self.context, "destination", envelope).unwrap()["message_id"]
            .as_str()
            .unwrap()
            .into()
    }

    fn capability(&self) -> String {
        capability_path(&self.context, "recipient", "recipient-incarnation")
            .to_str()
            .unwrap()
            .into()
    }

    fn run(&self, args: &[&str]) -> i32 {
        let argv = ["agent-session", "message"]
            .into_iter()
            .chain(args.iter().copied());
        let Command::Message(message) = Cli::try_parse_from(argv).unwrap().command else {
            panic!("not a message command")
        };
        run_message(&self.context, message)
    }

    fn show(&self, id: &str) -> i32 {
        let capability = self.capability();
        self.run(&[
            "show",
            "--session",
            "recipient",
            "--message",
            id,
            "--capability-file",
            &capability,
        ])
    }

    fn ack(&self, id: &str, revision: &str, key: &str) -> i32 {
        let capability = self.capability();
        self.run(&[
            "ack",
            "--session",
            "recipient",
            "--message",
            id,
            "--if-revision",
            revision,
            "--idempotency-key",
            key,
            "--capability-file",
            &capability,
        ])
    }

    fn ledger(&self) -> Vec<Value> {
        self.ledger_for(None, PROVIDER_SESSION)
    }

    fn ledger_path_for(&self, profile: Option<&str>, session: &str) -> std::path::PathBuf {
        let history = crate::provider_history::stable_history_id("claude", profile, session);
        self.context
            .state_dir
            .join(format!("history/mailbox-reads/{history}.jsonl"))
    }

    fn ledger_for(&self, profile: Option<&str>, session: &str) -> Vec<Value> {
        let history = crate::provider_history::stable_history_id("claude", profile, session);
        let path = self
            .context
            .state_dir
            .join(format!("history/mailbox-reads/{history}.jsonl"));
        std::fs::read_to_string(path)
            .map(|text| {
                text.lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn edit_record(&self, edit: impl FnOnce(&mut Value)) {
        let path = self
            .context
            .state_dir
            .join("sessions/recipient/session.json");
        let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        edit(&mut record);
        write_atomic(
            &path,
            &serde_json::to_vec(&record).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
    }

    fn transcript_in(config: &std::path::Path, session: &str, copies: usize) {
        let project = config.join("projects/example");
        std::fs::create_dir_all(&project).unwrap();
        let row = TRANSCRIPT.replace(PROVIDER_SESSION, session);
        std::fs::write(project.join(format!("{session}.jsonl")), row.repeat(copies)).unwrap();
    }

    fn notifications(&self) -> Value {
        serde_json::to_value(&lock_registry(&self.context).unwrap().registry.notifications).unwrap()
    }
}

#[test]
fn first_show_records_one_event_at_the_transcript_length_and_repeats_change_nothing() {
    let lock = GlobalStateLock::new();
    let rig = Rig::new(&lock, true);
    let id = rig.deliver("Please review the synthetic change.\n");
    assert_eq!(rig.show(&id), 0);
    let ledger = rig.ledger();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0]["op"], "create");
    assert_eq!(ledger[0]["message_id"], id);
    assert_eq!(ledger[0]["anchor"], TRANSCRIPT.len());
    assert_eq!(ledger[0]["body"], "Please review the synthetic change.\n");
    std::fs::write(&rig.transcript, format!("{TRANSCRIPT}{TRANSCRIPT}")).unwrap();
    assert_eq!(rig.show(&id), 0);
    assert_eq!(
        rig.ledger(),
        ledger,
        "a repeated show must not move or duplicate the event"
    );
}

#[test]
fn ack_updates_the_same_event_once_and_an_unread_ack_creates_it() {
    let lock = GlobalStateLock::new();
    let rig = Rig::new(&lock, true);
    let read = rig.deliver("shown first");
    assert_eq!(rig.show(&read), 0);
    assert_eq!(rig.ack(&read, "2", "ack-read"), 0);
    assert_eq!(rig.ack(&read, "2", "ack-read"), 0, "same key replays");
    let ledger = rig.ledger();
    assert_eq!(ledger.len(), 2);
    assert_eq!(ledger[1]["op"], "update");
    assert_eq!(ledger[1]["message_id"], read);
    assert!(ledger[1]["acknowledged_at_ms"].is_i64());

    let unread = rig.deliver("acknowledged unseen");
    assert_eq!(rig.ack(&unread, "1", "ack-unread"), 0);
    let ledger = rig.ledger();
    assert_eq!(ledger.len(), 3);
    assert_eq!(ledger[2]["op"], "create");
    assert_eq!(ledger[2]["message_id"], unread);
    assert_eq!(ledger[2]["acknowledged"], true);
}

#[test]
fn an_unresolvable_transcript_or_an_unauthorized_reader_records_nothing() {
    let lock = GlobalStateLock::new();
    let rig = Rig::new(&lock, false);
    let id = rig.deliver("no transcript yet");
    assert_eq!(rig.show(&id), 0, "mail is readable without a transcript");
    assert_eq!(rig.ledger(), Vec::<Value>::new());

    let rig = Rig::new(&lock, true);
    let id = rig.deliver("wrong capability");
    let code = rig.run(&[
        "show",
        "--session",
        "recipient",
        "--message",
        &id,
        "--capability-file",
        "/nonexistent/capability",
    ]);
    assert_ne!(code, 0);
    assert_eq!(rig.ledger(), Vec::<Value>::new());
}

#[test]
fn recording_never_changes_notification_state() {
    let lock = GlobalStateLock::new();
    let rig = Rig::new(&lock, true);
    let id = rig.deliver("quiet read");
    let before = rig.notifications();
    assert_eq!(rig.show(&id), 0);
    assert_eq!(rig.ack(&id, "2", "quiet-ack"), 0);
    assert_eq!(rig.ledger().len(), 2);
    assert_eq!(rig.notifications(), before);
}

#[test]
fn a_profiled_recipient_records_under_its_own_profile_history_and_root() {
    let lock = GlobalStateLock::new();
    let rig = Rig::new(&lock, true);
    let profile_root = tempfile::TempDir::new().unwrap();
    Rig::transcript_in(profile_root.path(), PROVIDER_SESSION, 2);
    rig.edit_record(|record| {
        record["runtime"]["agent_profile"] = json!("profile-a");
        record["runtime"]["agent_profile_provider_config_dir"] = json!(profile_root.path());
    });
    let id = rig.deliver("profiled read");
    assert_eq!(rig.show(&id), 0);
    let profiled = rig.ledger_for(Some("profile-a"), PROVIDER_SESSION);
    assert_eq!(
        profiled.len(),
        1,
        "the event belongs to the profile's history"
    );
    assert_eq!(
        profiled[0]["anchor"],
        TRANSCRIPT.len() * 2,
        "anchored in the profile's own root"
    );
    assert_eq!(
        rig.ledger(),
        Vec::<Value>::new(),
        "nothing under the default history"
    );
}

#[test]
fn a_rebind_between_commit_and_record_attributes_the_read_to_no_conversation() {
    use crate::cli::MessageShowArgs;
    let lock = GlobalStateLock::new();
    let rig = Rig::new(&lock, true);
    let other = "0a1b2c3d-0000-4000-8000-000000000002";
    let config = std::path::PathBuf::from(std::env::var("CLAUDE_CONFIG_DIR").unwrap());
    Rig::transcript_in(&config, other, 1);
    let id = rig.deliver("read before the rebind");
    let args = MessageShowArgs {
        session: "recipient".into(),
        message: id,
        capability_file: Some(capability_path(
            &rig.context,
            "recipient",
            "recipient-incarnation",
        )),
        format: OutputFormat::Json,
    };
    let rebind =
        || rig.edit_record(|record| record["provider_resume"]["session_id"] = json!(other));
    super::super::read_events::show_with(&rig.context, args, rebind).unwrap();
    assert_eq!(
        rig.ledger_for(None, other),
        Vec::<Value>::new(),
        "not the new conversation"
    );
    assert_eq!(
        rig.ledger(),
        Vec::<Value>::new(),
        "and no half-attributed event"
    );
}

#[test]
fn a_locked_ledger_never_blocks_a_committed_show_or_ack() {
    use crate::cli::{MessageAckArgs, MessageShowArgs};
    use std::os::fd::AsRawFd;
    let lock = GlobalStateLock::new();
    let rig = Rig::new(&lock, true);
    let id = rig.deliver("read while the ledger is busy");
    let path = rig.ledger_path_for(None, PROVIDER_SESSION);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let held = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap();
    // SAFETY: the descriptor stays open for the whole test.
    assert_eq!(unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX) }, 0);
    let capability = capability_path(&rig.context, "recipient", "recipient-incarnation");
    let (state_dir, message) = (rig.context.state_dir.clone(), id.clone());
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let context = CliContext {
            state_dir,
            host: None,
        };
        let shown = super::super::read_events::show(
            &context,
            MessageShowArgs {
                session: "recipient".into(),
                message: message.clone(),
                capability_file: Some(capability.clone()),
                format: OutputFormat::Json,
            },
        );
        let acked = super::super::read_events::ack(
            &context,
            MessageAckArgs {
                session: "recipient".into(),
                message,
                if_revision: 2,
                capability_file: Some(capability),
                idempotency_key: "locked-ack".into(),
                format: OutputFormat::Json,
            },
        );
        let _ = done.send((shown.is_ok(), acked.is_ok()));
    });
    let outcome = finished.recv_timeout(std::time::Duration::from_secs(10));
    assert_eq!(
        outcome,
        Ok((true, true)),
        "show and ack must complete while the ledger is locked"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "",
        "the busy ledger is left untouched"
    );
}
