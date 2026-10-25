use crate::coordination::{
    broker, capability_path, digest_bytes, heartbeat_path, lock_registry, now_epoch,
    revalidate_capability_file, verify_capability_token,
};
use crate::{CliContext, SessionRecord};
use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn locked_revalidation_checks_rotated_capability_before_runtime_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let context = CliContext {
        state_dir: dir.path().to_path_buf(),
        host: None,
    };
    let mut record: SessionRecord = serde_json::from_value(json!({
        "schema_version": crate::SESSION_DOCUMENT_VERSION,
        "id":"recipient", "agent":"codex", "mode":"interactive",
        "cwd":"/fixture/repository", "tmux_session":"fixture-session",
        "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z",
        "runtime":{
            "kind":"tmux", "tmux_session":"fixture-session", "generation":1,
            "started_at":"2030-01-01T00:00:00Z", "launch_id":"original"
        }
    }))
    .unwrap();
    crate::write_session_record(&context, &record).unwrap();
    broker::test_support::seed_live_broker(&context, "recipient", "original", json!({}));
    let original_token = "synthetic-original-recipient-capability";
    let supplied = capability_path(&context, "recipient", "original");
    write_atomic(&supplied, original_token.as_bytes(), SECRET_FILE_MODE).unwrap();
    let mut locked = lock_registry(&context).unwrap();
    locked
        .registry
        .brokers
        .get_mut("recipient")
        .unwrap()
        .capability_digest = digest_bytes(original_token.as_bytes());
    verify_capability_token(
        &context,
        &locked.registry,
        &record,
        "original",
        original_token,
    )
    .unwrap();

    // `reply` can reload this new record after authenticating the old runtime.
    // Model the broker/capability rotation under its locked revalidation boundary.
    record.runtime.as_mut().unwrap().launch_id = "replacement".into();
    let current_token = "synthetic-replacement-recipient-capability";
    let current = capability_path(&context, "recipient", "replacement");
    write_atomic(&current, current_token.as_bytes(), SECRET_FILE_MODE).unwrap();
    let broker = locked.registry.brokers.get_mut("recipient").unwrap();
    broker.incarnation = "replacement".into();
    broker.capability_digest = digest_bytes(current_token.as_bytes());
    write_atomic(
        &heartbeat_path(&context.state_dir, "recipient"),
        format!("replacement:{}\n", now_epoch()).as_bytes(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    verify_capability_token(
        &context,
        &locked.registry,
        &record,
        "replacement",
        current_token,
    )
    .unwrap();

    let error =
        revalidate_capability_file(&context, &locked.registry, &record, "original", &supplied)
            .unwrap_err();
    assert_eq!(error.code(), "coordination-unauthorized");
    assert_eq!(
        error.details().unwrap()["reason_code"],
        "capability-mismatch"
    );
}
