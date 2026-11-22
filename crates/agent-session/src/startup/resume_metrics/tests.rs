use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;

fn sample(tick_seconds: u64, wall: &str) -> Sample {
    Sample {
        at: wall.into(),
        tick_ns: Some(tick_seconds * 1_000_000_000),
    }
}

#[test]
fn resume_metrics_window_uses_same_boot_and_monotonic_ingress() {
    let start = sample(100, "2026-01-01T00:00:00Z");
    let just_before = sample(159, "2025-12-31T23:59:59Z");
    let deadline = sample(160, "2026-01-01T00:05:00Z");
    let mut receipt = Receipt {
        binding: "fixture".into(),
        kind: LaunchKind::Resume,
        boot_id: Some("same-boot".into()),
        start,
        finish: None,
    };
    let boot = Some("same-boot");
    assert_eq!(receipt.projection(&just_before, boot).outcome, "pending");
    assert_eq!(receipt.projection(&deadline, boot).outcome, "timed_out");
    assert_eq!(
        receipt
            .projection(&just_before, Some("foreign-boot"))
            .unknown_reason,
        Some("clock_unavailable")
    );
    assert!(!receipt.accepts(&sample(99, "2026-01-01T00:00:30Z"), boot));
    assert!(!receipt.accepts(&deadline, boot));
    assert!(receipt.accepts(&just_before, boot));
    receipt.finish = Some(Finish {
        outcome: FinishOutcome::Succeeded,
        sample: Some(just_before),
    });
    assert!(!receipt.accepts(&deadline, boot));
    let result = receipt.projection(&deadline, boot);
    assert_eq!(result.outcome, "succeeded");
    assert_eq!(result.duration_ms, Some(59_000));
    assert_eq!(result.completed_at.as_deref(), Some("2025-12-31T23:59:59Z"));
}

fn fixture(root: &Path, provider: &str) -> (CliContext, SessionRecord) {
    let context = CliContext {
        state_dir: root.join("state"),
        host: None,
    };
    let record = serde_json::from_value(json!({
        "schema_version":"agent-session.session.v1","id":"fixture","agent":provider,
        "mode":"interactive","cwd":root,"tmux_session":"fixture",
        "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z",
        "provider_resume":{"provider":provider,"session_id":"fixture-conversation",
            "captured_at":"2026-01-01T00:00:00Z","capture_method":"fixture","resume_args":[]},
        "runtime":{"kind":"tmux","tmux_session":"fixture","generation":1,
            "started_at":"2026-01-01T00:00:00Z","launch_id":"fixture-runtime",
            "agent_profile_provider_config_dir":root.join("config")}
    }))
    .unwrap();
    crate::write_session_record(&context, &record).unwrap();
    (context, record)
}

#[test]
fn resume_metrics_distinguishes_initial_and_raw_resume_and_observes_gate_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let (context, mut record) = fixture(tmp.path(), "codex");
    let dir = session_dir(&context, &record.id);
    let gate = dir.join("fixture-gate");
    release_gate(&context, &record, LaunchKind::Initial, &gate).unwrap();
    assert_eq!(fs::read(&gate).unwrap(), b"ready\n");
    assert_eq!(
        projection(&context, &record).unknown_reason,
        Some("not_applicable")
    );
    record.runtime.as_mut().unwrap().generation += 1;
    crate::write_session_record(&context, &record).unwrap();
    assert_eq!(
        projection(&context, &record).unknown_reason,
        Some("identity_changed")
    );

    release_gate(&context, &record, LaunchKind::Resume, &gate).unwrap();
    assert_eq!(
        projection(&context, &record).unknown_reason,
        Some("unsupported_runtime")
    );

    record.runtime.as_mut().unwrap().generation += 1;
    crate::write_session_record(&context, &record).unwrap();
    let gate_blocked = dir.join("gate-is-a-directory");
    fs::create_dir(&gate_blocked).unwrap();
    assert!(release_gate(&context, &record, LaunchKind::Resume, &gate_blocked).is_err());
    let failure = projection(&context, &record);
    assert_eq!(failure.outcome, "failed");
    assert_eq!(failure.duration_ms, None);
    assert_eq!(failure.unknown_reason, Some("launch_failed"));
}

#[test]
fn resume_metrics_storage_failure_does_not_prevent_gate_release() {
    let tmp = tempfile::tempdir().unwrap();
    let (context, record) = fixture(tmp.path(), "claude");
    let dir = session_dir(&context, &record.id);
    let gate = dir.join("fixture-gate");
    fs::create_dir(dir.join(RECEIPT_FILE)).unwrap();
    release_gate(&context, &record, LaunchKind::Resume, &gate).unwrap();
    assert_eq!(fs::read(&gate).unwrap(), b"ready\n");
    assert_eq!(
        projection(&context, &record).unknown_reason,
        Some("not_recorded")
    );
}

#[test]
fn resume_metrics_ignores_stale_configuration_and_oversized_receipts() {
    let tmp = tempfile::tempdir().unwrap();
    let (context, mut record) = fixture(tmp.path(), "claude");
    let dir = session_dir(&context, &record.id);
    release_gate(
        &context,
        &record,
        LaunchKind::Resume,
        &dir.join("fixture-gate"),
    )
    .unwrap();
    assert_eq!(projection(&context, &record).outcome, "pending");

    let previous = record.clone();
    record.runtime.as_mut().unwrap().extra.insert(
        "agent_profile_provider_config_dir".into(),
        json!(tmp.path().join("replacement-config")),
    );
    crate::write_session_record(&context, &record).unwrap();
    finish(&context, &previous, FinishOutcome::Succeeded, Sample::now());
    assert_eq!(
        projection(&context, &record).unknown_reason,
        Some("identity_changed")
    );

    let receipt = dir.join(RECEIPT_FILE);
    OpenOptions::new()
        .write(true)
        .open(receipt)
        .unwrap()
        .set_len(MAX_RECEIPT_BYTES + 1)
        .unwrap();
    assert_eq!(
        projection(&context, &record).unknown_reason,
        Some("not_recorded")
    );
}

#[test]
fn resume_metrics_leaves_other_providers_uninstrumented() {
    let tmp = tempfile::tempdir().unwrap();
    let (context, record) = fixture(tmp.path(), "dsh");
    let dir = session_dir(&context, &record.id);
    let gate = dir.join("fixture-gate");
    release_gate(&context, &record, LaunchKind::Resume, &gate).unwrap();
    assert_eq!(fs::read(&gate).unwrap(), b"ready\n");
    assert!(!dir.join(RECEIPT_FILE).exists());
    assert_eq!(
        projection(&context, &record).unknown_reason,
        Some("not_recorded")
    );
}
