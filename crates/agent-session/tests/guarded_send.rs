//! Admission regressions use a modeled tmux executable and synthetic records.
#[path = "guarded_send/attempt.rs"]
mod attempt;
#[path = "guarded_send/cases.rs"]
mod cases;
#[path = "guarded_send/claude_notice.rs"]
mod claude_notice;
#[path = "guarded_send/completion_evidence.rs"]
mod completion_evidence;
#[path = "guarded_send/exit.rs"]
mod exit;
#[path = "guarded_send/history_command_bytes.rs"]
mod history_command_bytes;
#[path = "guarded_send/live_composer.rs"]
mod live_composer;
#[path = "guarded_send/long_paste.rs"]
mod long_paste;
#[path = "guarded_send/observation_deadline.rs"]
mod observation_deadline;
#[path = "guarded_send/permission.rs"]
mod permission;
#[path = "guarded_send/placeholder.rs"]
mod placeholder;
#[path = "guarded_send/repair.rs"]
mod repair;
use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;
include!("guarded_send/fixture.rs");

fn run_fixture(root: &Path, args: &[&str]) -> CmdOutput {
    run_resolved(
        "agent-session",
        args,
        &CmdOptions::new()
            .with_cwd(root)
            .with_env_remove_prefix("AGENT_SESSION_"),
    )
}

#[test]
fn guarded_send_draft_present_refuses_without_pasting() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("composer"), "user draft").unwrap();
        let output = fixture.send(&["--require-empty-composer"]);
        let value = fixture.refused(&output, "send-composer-not-empty");
        assert_eq!(value["error"]["details"]["guard"], "empty_composer");
        fixture.no_delivery();
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            "user draft"
        );
    }
}

#[test]
fn guarded_send_turn_started_after_caller_check_refuses_without_pasting() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        // A caller observed waiting. The modeled liveness probe now starts a
        // turn before send's final admission; pane adapters cannot fence it.
        fs::write(fixture.root().join("start-turn"), "").unwrap();
        let output = fixture.send(&["--require-idle"]);
        let value = fixture.refused(&output, "send-session-not-idle");
        assert_eq!(value["error"]["details"]["guard"], "idle");
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_identity_and_key_happy_path_submits_once() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        let output = fixture.send(&[
            "--require-idle",
            "--require-empty-composer",
            "--expect-runtime",
            "runtime-one",
            "--expect-provider-session",
            "provider-one",
            "--idempotency-key",
            "continuation-one",
        ]);
        assert_eq!(output.code, 0, "{}", output.stderr_text());
        assert_eq!(
            output.stdout_json()["data"]["submission"]["outcome"],
            "submitted"
        );
        assert_eq!(output.stdout_json()["data"]["guards"]["atomic"], false);
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(|call| call[0] == "paste-buffer")
                .count(),
            1
        );
        assert_eq!(
            fs::read_to_string(fixture.root().join("submissions"))
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}

#[test]
fn transient_post_paste_composer_capture_retries_without_reposting() {
    for (provider, failed_capture) in [("claude", 3), ("codex", 2)] {
        let fixture = Fixture::new(provider);
        fs::write(
            fixture.root().join("fail-post-paste-capture-once"),
            failed_capture.to_string(),
        )
        .unwrap();

        let output = fixture.send_text(
            Some("continue-fixture"),
            &[
                "--require-idle",
                "--require-empty-composer",
                "--idempotency-key",
                "transient-observation",
            ],
        );

        assert_eq!(output.code, 0, "{provider}: {}", output.stdout_text());
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(|call| call[0] == "paste-buffer")
                .count(),
            1,
            "{provider} paste count"
        );
        let submissions = fs::read_to_string(fixture.root().join("submissions")).unwrap();
        assert_eq!(
            submissions.lines().count(),
            1,
            "{provider} submission count"
        );
        assert_eq!(
            serde_json::from_str::<String>(submissions.trim()).unwrap(),
            "continue-fixture"
        );
    }
}

#[test]
fn guarded_send_repeated_key_survives_process_restart_and_runtime_change() {
    let fixture = Fixture::new("claude");
    let first = fixture.send(&["--idempotency-key", "continuation-one"]);
    assert_eq!(first.code, 0, "{}", first.stderr_text());
    let path = fixture.root().join("state/sessions/guarded/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!("runtime-two");
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    let repeated = fixture.send(&["--idempotency-key", "continuation-one"]);
    fixture.refused(&repeated, "send-idempotency-key-used");
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
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
fn guarded_send_identity_mismatch_refuses_without_delivery() {
    for guard in ["--expect-runtime", "--expect-provider-session"] {
        let fixture = Fixture::new("claude");
        let output = fixture.send(&[guard, "replaced-identity"]);
        fixture.refused(&output, "send-identity-mismatch");
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_unguarded_output_remains_compatible() {
    let fixture = Fixture::new("codex");
    let output = fixture.send(&[]);
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    let value = output.stdout_json();
    assert_eq!(value["schema_version"], "cli.agent-session.send.v1");
    assert_eq!(value["data"]["sent_text"], true);
    assert!(value["data"].get("guards").is_none());
    assert!(value["data"].get("idempotency_key").is_none());
}

#[test]
fn guarded_send_post_paste_mismatch_preserves_user_input_without_submit() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("inject-draft"), "").unwrap();
        let output = fixture.send(&[
            "--require-idle",
            "--require-empty-composer",
            "--idempotency-key",
            "mismatch",
        ]);
        fixture.refused(&output, "send-composer-mismatch");
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(|call| call[0] == "paste-buffer")
                .count(),
            1
        );
        assert!(!fixture.root().join("submissions").exists());
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            "continue-fixture user addition"
        );
        let repeated = fixture.send(&["--idempotency-key", "mismatch"]);
        fixture.refused(&repeated, "send-idempotency-key-used");
    }
}

#[test]
fn guarded_send_turn_start_after_paste_refuses_without_submit() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("start-after-paste"), "").unwrap();
    let output = fixture.send(&["--require-idle", "--require-empty-composer"]);
    fixture.refused(&output, "send-session-not-idle");
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
    );
    assert!(!fixture.root().join("submissions").exists());
}

#[test]
fn guarded_send_unobservable_capture_and_activity_fail_closed() {
    let fixture = Fixture::new("claude");
    let observation = fixture
        .root()
        .join("state/sessions/guarded/drafts/observation.json");
    fs::create_dir_all(observation.parent().unwrap()).unwrap();
    fs::write(&observation, "{}").unwrap();
    fs::write(fixture.root().join("capture-fails"), "").unwrap();
    let refusal = fixture.refused(
        &fixture.send(&["--require-empty-composer"]),
        "send-composer-unobservable",
    );
    assert_eq!(refusal["error"]["details"]["stage"], "admission");
    assert_eq!(refusal["error"]["details"]["observation"], "capture-failed");
    assert!(
        !observation.exists(),
        "unknown input resets the stale timer"
    );
    fixture.no_delivery();
    fs::remove_file(fixture.root().join("capture-fails")).unwrap();
    fs::remove_file(fixture.root().join("state/sessions/guarded/activity.json")).unwrap();
    fixture.refused(&fixture.send(&["--require-idle"]), "send-idle-unobservable");
    fixture.no_delivery();
}

#[test]
fn guarded_send_unsupported_provider_checks_fail_closed() {
    let fixture = Fixture::new("dsh");
    for flag in ["--require-idle", "--require-empty-composer"] {
        fixture.refused(&fixture.send(&[flag]), "send-guard-unsupported");
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_pending_provider_event_refuses_without_pasting() {
    use std::os::fd::AsRawFd;
    let fixture = Fixture::new("claude");
    let intent = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(
            fixture
                .root()
                .join("state/sessions/guarded/.activity-ingress.lock"),
        )
        .unwrap();
    // SAFETY: intent owns this valid descriptor; this is synthetic state only.
    assert_eq!(unsafe { libc::flock(intent.as_raw_fd(), libc::LOCK_SH) }, 0);
    let output = fixture.send(&["--require-idle"]);
    fixture.refused(&output, "send-provider-event-pending");
    fixture.no_delivery();
}

#[test]
fn guarded_send_concurrent_same_key_delivers_once() {
    let fixture = Fixture::new("claude");
    let mut codes = std::thread::scope(|scope| {
        let first = scope.spawn(|| fixture.send(&["--idempotency-key", "concurrent"]));
        let second = scope.spawn(|| fixture.send(&["--idempotency-key", "concurrent"]));
        let outputs = [first.join().unwrap(), second.join().unwrap()];
        for output in &outputs {
            if output.code != 0 {
                fixture.refused(output, "send-idempotency-key-used");
            }
        }
        outputs.map(|output| output.code)
    });
    codes.sort();
    assert_eq!(codes, [0, 1]);
    assert_eq!(
        fixture
            .calls()
            .iter()
            .filter(|call| call[0] == "paste-buffer")
            .count(),
        1
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
fn guarded_send_ambiguous_composer_fails_closed() {
    for (provider, draft) in [
        ("codex", "Ask Codex to do anything"),
        ("claude", "[Pasted text #1 +2 lines]"),
        ("claude", "Try \"edit <filepath> to...\""),
    ] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("composer"), draft).unwrap();
        fixture.refused(
            &fixture.send(&["--require-empty-composer"]),
            "send-composer-unobservable",
        );
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_turn_start_during_buffer_load_refuses_without_pasting() {
    let fixture = Fixture::new("claude");
    fs::write(fixture.root().join("start-on-load"), "").unwrap();
    fixture.refused(
        &fixture.send(&["--require-idle", "--idempotency-key", "load-race"]),
        "send-session-not-idle",
    );
    assert!(fixture.calls().iter().any(|call| call[0] == "load-buffer"));
    fixture.no_delivery();
}

#[test]
fn guarded_send_truncated_capture_is_unobservable() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("capture-oversize"), "").unwrap();
        fixture.refused(
            &fixture.send(&["--require-empty-composer"]),
            "send-composer-unobservable",
        );
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_observed_provider_hook_idle_is_supported() {
    let fixture = Fixture::new("claude");
    let path = fixture.root().join("state/sessions/guarded/activity.json");
    let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    document["state"]["source"]["confidence"] = json!("observed");
    fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    let output = fixture.send(&["--require-idle", "--require-empty-composer"]);
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    assert_eq!(
        output.stdout_json()["data"]["submission"]["enter_presses"],
        1
    );
}

#[test]
fn guarded_send_inferred_or_console_idle_is_unobservable() {
    for (kind, confidence) in [
        ("provider_hook", "inferred"),
        ("console_observation", "observed"),
    ] {
        let fixture = Fixture::new("claude");
        let path = fixture.root().join("state/sessions/guarded/activity.json");
        let mut document: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        document["state"]["source"] = json!({"kind": kind, "confidence": confidence});
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        fixture.refused(&fixture.send(&["--require-idle"]), "send-idle-unobservable");
        fixture.no_delivery();
    }
}

#[test]
fn guarded_send_unverified_submission_fails_cleans_and_retains_claim() {
    for provider in ["claude", "codex"] {
        let fixture = Fixture::new(provider);
        fs::write(fixture.root().join("unverified-after-enter"), "").unwrap();
        let guards = [
            "--require-empty-composer",
            "--idempotency-key",
            "uncertain-attempt",
        ];
        let output = fixture.send(&guards);
        let result = fixture.refused(&output, "send-submit-unconfirmed");
        let details = &result["error"]["details"];
        assert_eq!(details["outcome"], "unverified");
        assert_eq!(details["enter_presses"], 1);
        assert_eq!(details["pasted"], true);
        assert_eq!(details["cleared"], true);
        assert_eq!(details["remaining_input"], "empty");
        assert_eq!(
            fs::read_to_string(fixture.root().join("composer")).unwrap(),
            ""
        );
        assert!(!fixture.root().join("submissions").exists());
        assert_eq!(
            fixture
                .calls()
                .iter()
                .filter(
                    |call| call[0] == "send-keys" && call.last().is_some_and(|key| key == "Enter")
                )
                .count(),
            1
        );
        fixture.refused(&fixture.send(&guards), "send-idempotency-key-used");
    }
}
