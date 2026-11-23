//! Keyed terminal read-back uses only evidence from the original attempt.
use super::{CmdOutput, Fixture, Value, fs, json, run_fixture};
use pretty_assertions::assert_eq;

const GUARDS: &[&str] = &[
    "--require-idle",
    "--require-empty-composer",
    "--idempotency-key",
    "synthetic-attempt",
];

pub(super) fn query(fixture: &Fixture, action: &str, key: &str) -> CmdOutput {
    run_fixture(
        fixture.root(),
        &[
            "--state-dir",
            fixture.root().join("state").to_str().unwrap(),
            "attempt",
            action,
            "guarded",
            "--expect-runtime",
            "runtime-one",
            "--idempotency-key",
            key,
            "--tmux-bin",
            fixture.root().join("tmux").to_str().unwrap(),
            "--format",
            "json",
        ],
    )
}

pub(super) fn status(fixture: &Fixture, key: &str) -> Value {
    let output = query(fixture, "status", key);
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    let result = output.stdout_json();
    assert!(!output.stdout_text().contains("continue-fixture"));
    assert!(!output.stdout_text().contains("synthetic-attempt"));
    result["data"].clone()
}

#[test]
fn attempt_terminal_refusal_and_replacement() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("start-after-paste"), "").unwrap();
    fixture.refused(&fixture.send(GUARDS), "send-session-not-idle");
    let status_before = status(&fixture, "synthetic-attempt");
    assert_eq!(status_before["outcome"], "not_submitted");
    assert_eq!(status_before["composer"], "absent");

    let path = fixture.root().join("state/sessions/guarded/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!("runtime-two");
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert_eq!(status(&fixture, "synthetic-attempt")["outcome"], "unknown");
    assert_ne!(query(&fixture, "recover", "synthetic-attempt").code, 0);
    assert!(!fixture.root().join("submissions").exists());
}

#[test]
fn attempt_retained_input_recovery_uses_one_enter_without_paste() {
    let fixture = Fixture::new("codex");
    let activity = fixture.root().join("state/sessions/guarded/activity.json");
    let idle = fs::read(&activity).unwrap();
    for marker in ["start-after-paste", "backspace-ignored"] {
        fs::write(fixture.root().join(marker), "").unwrap();
    }
    fixture.refused(&fixture.send(GUARDS), "send-session-not-idle");
    fs::write(activity, idle).unwrap();
    assert_eq!(status(&fixture, "synthetic-attempt")["composer"], "exact");

    let composer = fixture.root().join("composer");
    fs::write(&composer, "owner draft").unwrap();
    assert_ne!(query(&fixture, "recover", "synthetic-attempt").code, 0);
    assert_eq!(fs::read_to_string(&composer).unwrap(), "owner draft");
    assert!(!fixture.root().join("submissions").exists());

    // A fresh attempt provides the independent input used for successful recovery.
    let fixture = Fixture::new("codex");
    let activity = fixture.root().join("state/sessions/guarded/activity.json");
    let idle = fs::read(&activity).unwrap();
    for marker in ["start-after-paste", "backspace-ignored"] {
        fs::write(fixture.root().join(marker), "").unwrap();
    }
    fixture.refused(&fixture.send(GUARDS), "send-session-not-idle");
    fs::write(activity, idle).unwrap();
    let before = fixture.calls().len();
    let output = query(&fixture, "recover", "synthetic-attempt");
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_ne!(query(&fixture, "recover", "synthetic-attempt").code, 0);
    assert_eq!(
        fixture.calls()[before..]
            .iter()
            .filter(|call| call[0] == "send-keys" && call.last().unwrap() == "Enter")
            .count(),
        1,
    );
    assert!(
        fixture.calls()[before..]
            .iter()
            .all(|call| call[0] != "paste-buffer")
    );
    assert_eq!(
        fs::read_to_string(fixture.root().join("submissions"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn attempt_unverified_enter_never_becomes_retryable() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("unverified-after-enter"), "").unwrap();
    fs::write(fixture.root().join("backspace-ignored"), "").unwrap();
    fixture.refused(&fixture.send(GUARDS), "send-submit-unconfirmed");
    assert_eq!(status(&fixture, "synthetic-attempt")["outcome"], "unknown");
    let before = fixture.calls().len();
    assert_ne!(query(&fixture, "recover", "synthetic-attempt").code, 0);
    assert!(
        fixture.calls()[before..]
            .iter()
            .all(|call| call[0] != "send-keys")
    );
}
