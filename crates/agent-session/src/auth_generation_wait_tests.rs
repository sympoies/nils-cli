//! Elapsed-time publication wait; controller cadence must not consume it early.
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn credential_wait_delayed_publication_survives_rapid_ticks_and_restarts_once() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    publish(tmp.path(), 3);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, true);
    crate::coordination::auth_effects::test_support::seed(&context, &record);
    let before = log(tmp.path());
    for age in 0..6 {
        // Equal, older and another account's publication are never recovery.
        let mut publication =
            crate::claude_account::generation::tests::envelope(if age % 3 == 1 { 2 } else { 3 });
        if age % 3 == 2 {
            publication = crate::claude_account::generation::tests::envelope(4);
            publication["account"] = json!("beta");
        }
        fs::write(
            tmp.path().join("credential-state.json"),
            publication.to_string(),
        )
        .unwrap();
        for _ in 0..2 {
            tick(&context, &record, &tmux, 100 + age);
            crate::auth_incident::failover::tick(&context, &record, &tmux, 100 + age).unwrap();
            assert_eq!(recovery_receipt(&context)["state"], "waiting_generation");
            assert!(failover_receipt(&context).is_null());
            // A pending wait retains neither lock between controller ticks.
            let _record_lock =
                crate::acquire_session_record_lock_timed(&context, ID, Duration::ZERO).unwrap();
            let _operation_lock = crate::coordination::lock_private_store(
                &context,
                &format!("auth-recovery-{ID}.lock"),
            )
            .unwrap();
        }
    }
    assert_eq!(log(tmp.path()), before);
    publish(tmp.path(), 4);
    fs::write(tmp.path().join("exit-on-probe"), "").unwrap();
    let watcher = on_relaunched_prompt(&context, tmp.path(), &launch_id(&record).unwrap());
    tick(&context, &record, &tmux, 106);
    watcher.join().unwrap();
    let current = load_session_record(&context, ID).unwrap();
    let receipt = recovery_receipt(&context);
    assert_eq!(receipt["state"], "runtime_resumed");
    assert_eq!(receipt["replayed"], false);
    assert_eq!(receipt["unknown_operation_count"], 2);
    assert!(!receipt.to_string().contains("sensitive-canary"));
    assert_eq!(
        current.runtime.as_ref().unwrap().generation,
        record.runtime.as_ref().unwrap().generation + 1
    );
    assert_eq!(
        crate::claude_account::view_for_record(&current)
            .unwrap()
            .selected_account
            .as_deref(),
        Some("alpha")
    );
    let launches = log(tmp.path()).matches("new-session").count();
    assert_eq!(launches, 2);
    tick(&context, &current, &tmux, 107);
    assert_eq!(log(tmp.path()).matches("new-session").count(), launches);
    let effects = &log(tmp.path())[before.len()..];
    assert!(!effects.contains("load-buffer"));
    assert!(!effects.contains("send-keys"));
}

#[test]
fn credential_wait_expires_at_thirty_seconds_without_new_publication() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, true);
    let before = log(tmp.path());
    tick(&context, &record, &tmux, 100);
    tick(&context, &record, &tmux, 129);
    assert_eq!(recovery_receipt(&context)["state"], "waiting_generation");
    tick(&context, &record, &tmux, 130);
    let receipt = recovery_receipt(&context);
    assert_eq!(receipt["state"], "blocked");
    assert_eq!(receipt["deadline_epoch"], 130);
    assert_eq!(receipt["last_reason"], "publication_deadline");
    crate::auth_incident::failover::tick(&context, &record, &tmux, 130).unwrap();
    assert_eq!(failover_receipt(&context)["state"], "blocked_no_credential");
    tick(&context, &record, &tmux, 131);
    assert_eq!(recovery_receipt(&context)["state"], "blocked");
    assert_eq!(log(tmp.path()), before);
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
}

fn failover_receipt(context: &CliContext) -> Value {
    let store: Value = serde_json::from_slice(
        &fs::read(crate::session_dir(context, ID).join("auth-incidents.json")).unwrap(),
    )
    .unwrap();
    store["incidents"][0]["failover"].clone()
}
