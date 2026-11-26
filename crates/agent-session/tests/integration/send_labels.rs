use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

use nils_test_support::cmd::{CmdOutput, run_resolved};
use nils_test_support::tempdir::ScopedTempDir;
use pretty_assertions::assert_eq;
use serde_json::Value;

use super::coordination::{capability, seed_brokers};

struct Fixture {
    root: ScopedTempDir,
    state: PathBuf,
    body: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = ScopedTempDir::new();
        let state = root.path().join("state");
        fs::create_dir(&state).unwrap();
        seed_brokers(
            &state,
            &[
                (
                    "sender",
                    "sender-incarnation",
                    "synthetic-sender-capability-material",
                ),
                (
                    "recipient",
                    "recipient-incarnation",
                    "synthetic-recipient-capability-material",
                ),
            ],
        );
        let body = root.path().join("body.txt");
        fs::write(&body, "synthetic message").unwrap();
        Self { root, state, body }
    }

    fn run(&self, args: &[&str]) -> CmdOutput {
        let mut cmd = vec!["--state-dir", self.state.to_str().unwrap()];
        cmd.extend_from_slice(args);
        cmd.extend(["--format", "json"]);
        run_resolved(
            "agent-session",
            &cmd,
            &super::hermetic_provider_options(self.root.path())
                .without_ambient_managed_session_env()
                .with_env("AGENT_SESSION_MACHINE", "local"),
        )
    }

    fn send(&self, key: &str, target_args: &[&str]) -> CmdOutput {
        let sender_cap = capability(&self.state, "sender");
        let key = format!("send-label-{key}");
        let mut args = vec![
            "message",
            "send",
            "--from",
            "sender",
            "--body-file",
            self.body.to_str().unwrap(),
            "--capability-file",
            &sender_cap,
            "--idempotency-key",
            &key,
        ];
        args.extend_from_slice(target_args);
        self.run(&args)
    }

    fn inbox(&self) -> Vec<Value> {
        let output = self.run(&[
            "message",
            "inbox",
            "--session",
            "recipient",
            "--capability-file",
            &capability(&self.state, "recipient"),
        ]);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        output.stdout_json()["data"]["messages"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn record_path(&self) -> PathBuf {
        self.state.join("sessions/recipient/session.json")
    }

    fn record(&self) -> Value {
        serde_json::from_slice(&fs::read(self.record_path()).unwrap()).unwrap()
    }

    fn set_label(&self, value: &str) {
        let output = self.run(&[
            "metadata",
            "set-label",
            "recipient",
            &format!("assignment={value}"),
        ]);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
    }
}

fn assert_error(output: CmdOutput, status: i32, code: &str) {
    assert_eq!(
        output.code,
        status,
        "stdout={} stderr={}",
        output.stdout_text(),
        output.stderr_text()
    );
    assert_eq!(output.stdout_json()["error"]["code"], code);
}

#[test]
fn send_labels_stale_mailbox_expectation_does_not_enqueue() {
    let fixture = Fixture::new();
    let absent = fixture.send(
        "absent",
        &["--to", "recipient", "--expect-label", "assignment=before"],
    );
    assert_error(absent, 75, "send-label-mismatch");
    fixture.set_label("before");
    let snapshot = fixture.record();
    let old = snapshot["send_labels"]["assignment"].as_str().unwrap();
    fixture.set_label("after");

    let guarded = fixture.send(
        "stale",
        &[
            "--to",
            "recipient",
            "--expect-label",
            &format!("assignment={old}"),
        ],
    );
    assert!(fixture.inbox().is_empty());
    assert_error(guarded, 75, "send-label-mismatch");
}

#[test]
fn send_labels_matching_local_and_unguarded_enqueue_and_replay() {
    let fixture = Fixture::new();
    fixture.set_label("current");
    let guarded = [
        "--to",
        "recipient",
        "--to-machine",
        "local",
        "--expect-label",
        "assignment=current",
    ];
    let sent = fixture.send("guarded", &guarded);
    assert_eq!(sent.code, 0, "{}", sent.stdout_text());
    let unguarded = fixture.send("unguarded", &["--to", "recipient"]);
    assert_eq!(unguarded.code, 0, "{}", unguarded.stdout_text());
    fixture.set_label("next");
    let replay = fixture.send("guarded", &guarded);
    assert_eq!(replay.code, 0, "{}", replay.stdout_text());
    assert_eq!(replay.stdout_json(), sent.stdout_json());
    assert_error(
        fixture.send(
            "guarded",
            &["--to", "recipient", "--expect-label", "assignment=next"],
        ),
        65,
        "idempotency-key-reused",
    );
    assert_eq!(fixture.inbox().len(), 2);
}

#[test]
fn send_labels_remote_expectation_refuses_before_outbox() {
    let fixture = Fixture::new();
    assert_error(
        fixture.send(
            "remote",
            &[
                "--to",
                "recipient",
                "--to-machine",
                "other",
                "--expect-label",
                "assignment=current",
            ],
        ),
        69,
        "send-label-unsupported",
    );
    assert!(
        !fixture
            .state
            .join("coordination/federation-journal.json")
            .exists()
    );
    assert!(fixture.inbox().is_empty());
}

#[test]
fn send_labels_terminal_refuses_before_writing_and_delivers_on_match() {
    let fixture = Fixture::new();
    fixture.set_label("current");
    let fake_tmux = fixture.root.path().join("fake-tmux");
    fs::write(
        &fake_tmux,
        "#!/bin/sh\ncase \"$1\" in\n  has-session) exit 0 ;;\n  send-keys) printf '%s\\n' \"$*\" >> tmux-input ;;\nesac\n",
    )
    .unwrap();
    fs::set_permissions(&fake_tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let send = |expectation: &str| {
        fixture.run(&[
            "send",
            "recipient",
            "--tmux-bin",
            fake_tmux.to_str().unwrap(),
            "--key",
            "escape",
            "--expect-label",
            expectation,
        ])
    };
    let input = fixture.root.path().join("tmux-input");
    assert_error(send("assignment=stale"), 75, "send-label-mismatch");
    assert!(!input.exists());
    let matched = send("assignment=current");
    assert_eq!(matched.code, 0, "{}", matched.stdout_text());
    assert!(fs::read_to_string(input).unwrap().ends_with(" Escape\n"));
}
