//! Review regressions: native capture fidelity and zero-input refusal state.
use std::fs;
use std::os::fd::AsRawFd;

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use super::Fixture;

impl Fixture {
    fn document(&self, name: &str) -> Value {
        serde_json::from_slice(
            &fs::read(self.root().join("state/sessions/guarded").join(name)).unwrap(),
        )
        .unwrap()
    }

    fn arm_recovery(&self) -> Vec<u8> {
        let bytes = serde_json::to_vec(&json!({
            "schema_version": "agent-session.auto-resume.v1", "enabled": true,
            "state": "armed", "updated_at": "2000-01-01T00:00:00Z"
        }))
        .unwrap();
        fs::write(
            self.root().join("state/sessions/guarded/auto-resume.json"),
            &bytes,
        )
        .unwrap();
        bytes
    }

    fn bind_codex(&self) -> fs::File {
        let dir = self.root().join("state/sessions/guarded");
        let mut record = self.document("session.json");
        record["runtime"]["kind"] = json!("codex_app_server");
        record["runtime"]["codex_app_server_protocol"] = json!("v2");
        for (key, file) in [
            ("codex_app_server_socket", "cx.sock"),
            ("codex_app_server_proxy", "cx.proxy"),
            ("codex_app_server_thread_handoff", "cx.thread"),
            ("codex_app_server_thread_attached", "cx.attached"),
        ] {
            record["runtime"][key] = json!(self.root().join(file));
        }
        record["codex_account_binding"] = json!({
            "schema_version": "agent-session.codex-account-binding.v1",
            "selected_account": "fixture-account", "revision": 1, "state": "bound",
            "applied_runtime_id": "runtime-one", "updated_at": "2000-01-01T00:00:00Z"
        });
        fs::write(
            dir.join("session.json"),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let path = dir.join(".codex-app-server-proxy-capability");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema_version": "agent-session.codex-manual-input-proxy.v1",
                "launch_id": "runtime-one", "token": "00000000-0000-4000-8000-000000000001",
                "owner_pid": std::process::id(), "expires_at_epoch_ms": now + 60_000
            }))
            .unwrap(),
        )
        .unwrap();
        let lease = fs::File::open(path).unwrap();
        // SAFETY: the synthetic fixture owns the descriptor and holds it through send.
        assert_eq!(unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_SH) }, 0);
        lease
    }

    fn unchanged_control_state(&self, recovery: &[u8]) {
        assert!(
            self.document("session.json")
                .get("codex_account_input_fence")
                .is_none()
        );
        assert_eq!(
            fs::read(self.root().join("state/sessions/guarded/auto-resume.json")).unwrap(),
            recovery
        );
        self.no_delivery();
    }

    fn input_control_state(&self) {
        let record = self.document("session.json");
        assert_eq!(
            record["codex_account_input_fence"]["launch_id"],
            "runtime-one"
        );
        assert_eq!(record["codex_account_input_fence"]["activity_revision"], 2);
        let recovery = self.document("auto-resume.json");
        assert_eq!(recovery["enabled"], false);
        assert_eq!(recovery["state"], "cancelled");
        assert_eq!(recovery["failure_reason"], "manual_input");
    }
}

#[test]
fn guarded_send_whitespace_draft_refuses_before_input() {
    for provider in ["claude", "codex"] {
        for text in [Some("continue-fixture"), None] {
            let fixture = Fixture::new(provider);
            fs::write(fixture.root().join("composer"), " ").unwrap();
            let output = fixture.send_text(text, &["--require-empty-composer"]);
            fixture.refused(&output, "send-composer-not-empty");
            fixture.no_delivery();
            assert_eq!(
                fs::read_to_string(fixture.root().join("composer")).unwrap(),
                " "
            );
        }
    }
}

#[test]
fn guarded_send_trailing_space_payload_submits_once() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        let output = fixture.send_text(
            Some("continue-fixture  "),
            &[
                "--require-empty-composer",
                "--idempotency-key",
                "trailing-space",
            ],
        );
        assert_eq!(output.code, 0, "{}", output.stdout_json());
        assert_eq!(
            output.stdout_json()["data"]["submission"]["enter_presses"],
            1
        );
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(|call| call[0] == "paste-buffer")
                .count(),
            1
        );
        let submissions = fs::read_to_string(fixture.root().join("submissions")).unwrap();
        let prompts: Vec<String> = submissions
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(prompts, ["continue-fixture  "]);
        fixture.refused(
            &fixture.send(&["--idempotency-key", "trailing-space"]),
            "send-idempotency-key-used",
        );
        assert_eq!(
            fs::read_to_string(fixture.root().join("submissions")).unwrap(),
            submissions
        );
    }
}

#[test]
fn guarded_send_second_admission_refusal_preserves_control_state() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    let recovery = fixture.arm_recovery();
    fs::write(fixture.root().join("flip-second-runtime-read"), "").unwrap();
    fixture.refused(
        &fixture.send_text(None, &["--expect-runtime", "runtime-one"]),
        "send-identity-mismatch",
    );
    fixture.unchanged_control_state(&recovery);
}

#[test]
fn guarded_send_load_time_refusal_preserves_control_state() {
    for provider in ["codex", "claude"] {
        for (marker, reason) in [
            ("draft-on-load", "send-composer-not-empty"),
            ("capture-fails-on-load", "send-composer-unobservable"),
        ] {
            let fixture = Fixture::new(provider);
            let recovery = fixture.arm_recovery();
            fs::write(fixture.root().join(marker), "").unwrap();
            fixture.refused(
                &fixture.send(&[
                    "--require-empty-composer",
                    "--idempotency-key",
                    "load-conflict",
                ]),
                reason,
            );
            assert!(fixture.calls().iter().any(|call| call[0] == "load-buffer"));
            fixture.unchanged_control_state(&recovery);
        }
    }
}

#[test]
fn guarded_send_admitted_input_updates_control_state() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    fixture.arm_recovery();
    // App-server supports identity-only key input, not composer observations.
    let output = fixture.send_text(
        None,
        &[
            "--expect-runtime",
            "runtime-one",
            "--idempotency-key",
            "admitted",
        ],
    );
    assert_eq!(output.code, 0, "{}", output.stdout_json());
    fixture.input_control_state();
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "send-keys")
            .count(),
        1
    );
    assert!(fixture.calls().iter().all(|call| call[0] != "paste-buffer"));
    assert_eq!(
        fs::read_to_string(fixture.root().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn guarded_send_post_input_refusal_retains_control_state() {
    let fixture = Fixture::new("codex");
    let _capability = fixture.bind_codex();
    fixture.arm_recovery();
    fs::write(fixture.root().join("replace-after-enter"), "").unwrap();
    // The first key is delivered; runtime identity changes before the second.
    fixture.refused(
        &fixture.send_text(
            None,
            &[
                "--expect-runtime",
                "runtime-one",
                "--idempotency-key",
                "post-input",
                "--key",
                "enter",
            ],
        ),
        "send-identity-mismatch",
    );
    fixture.input_control_state();
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "send-keys")
            .count(),
        1
    );
    assert!(fixture.calls().iter().all(|call| call[0] != "paste-buffer"));
    let submissions = fs::read_to_string(fixture.root().join("submissions")).unwrap();
    assert_eq!(submissions.lines().count(), 1);
    fixture.refused(
        &fixture.send_text(
            None,
            &[
                "--expect-runtime",
                "runtime-one",
                "--idempotency-key",
                "post-input",
            ],
        ),
        "send-idempotency-key-used",
    );
    assert_eq!(
        fs::read_to_string(fixture.root().join("submissions")).unwrap(),
        submissions
    );
}
