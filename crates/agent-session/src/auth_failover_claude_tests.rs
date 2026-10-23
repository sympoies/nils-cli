//! Lifecycle acceptance for the second provider, using the real broker runner.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

fn failover_broker(root: &Path, lock: &GlobalStateLock, candidate: Value) -> EnvGuard {
    let guard = broker(root, lock);
    let beta = root.join("beta");
    fs::create_dir(&beta).unwrap();
    fs::set_permissions(&beta, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(beta.join(".credentials.json"), "{}").unwrap();
    fs::write(
        root.join("inventory.json"),
        json!({
            "schema_version":"agent-session.account-broker.v2", "provider":"claude",
            "capabilities":["credential_state_v1","auth_failover_v1"],
            "selection_strategies":["next_with_capacity"],
            "accounts":[{"account":"alpha"},{"account":"beta"}]
        })
        .to_string(),
    )
    .unwrap();
    let now = jiff::Timestamp::now().as_second();
    fs::write(root.join("selection.json"), json!({
        "schema_version":"agent-session.account-broker.v2","provider":"claude","account":candidate,
        "auth_failover":{"policy_allowed":true,"capacity_confirmed":true,
            "checked_at_epoch":now,"expires_at_epoch":now+10}
    }).to_string()).unwrap();
    for account in ["alpha", "beta"] {
        let mut state = crate::claude_account::generation::tests::envelope(1);
        state["account"] = json!(account);
        fs::write(
            root.join(format!("{account}-state.json")),
            state.to_string(),
        )
        .unwrap();
        fs::write(
            root.join(format!("{account}-materialized.json")),
            json!({
                "schema_version":"agent-session.account-broker.v2","provider":"claude",
                "account":account,"config_dir":root.join(account)
            })
            .to_string(),
        )
        .unwrap();
    }
    let script = format!(
        r#"#!/bin/sh
root={root}
printf '%s\n' "$*" >> "$root/broker.log"
account=alpha
for arg in "$@"; do
  [ "$arg" = beta ] && account=beta
done
case "$1" in
list) cat "$root/inventory.json" ;;
select)
  now=$(date +%s)
  sed -e "s/\"checked_at_epoch\":[0-9]*/\"checked_at_epoch\":$now/" \
      -e "s/\"expires_at_epoch\":[0-9]*/\"expires_at_epoch\":$((now+10))/" "$root/selection.json"
  ;;
credential-state) cat "$root/$account-state.json" ;;
materialize) cat "$root/$account-materialized.json" ;;
*) exit 1 ;;
esac
"#,
        root = shell_words::quote(&root.to_string_lossy())
    );
    executable(&root.join("broker"), &script);
    guard
}

fn exhausted(context: &CliContext, record: &SessionRecord, tmux: &Path) {
    tick(context, record, tmux, 100);
    tick(context, record, tmux, 220);
    assert_eq!(
        recovery_receipt(context)["last_reason"],
        "publication_deadline"
    );
}

fn failover_receipt(context: &CliContext) -> Value {
    let store: Value = serde_json::from_slice(
        &fs::read(crate::session_dir(context, ID).join("auth-incidents.json")).unwrap(),
    )
    .unwrap();
    store["incidents"][0]["failover"].clone()
}

#[test]
fn auth_failover_claude_preserves_conversation_retires_claims_and_never_replays() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = failover_broker(tmp.path(), &lock, json!("beta"));
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, true);
    crate::coordination::auth_effects::test_support::seed(&context, &record);
    exhausted(&context, &record, &tmux);
    let before = log(tmp.path());
    fs::write(tmp.path().join("exit-on-probe"), "").unwrap();
    let watcher = on_relaunched_prompt(&context, tmp.path(), &launch_id(&record).unwrap());
    crate::auth_incident::failover::tick(
        &context,
        &record,
        &tmux,
        jiff::Timestamp::now().as_second(),
    )
    .unwrap();
    watcher.join().unwrap();
    let receipt = failover_receipt(&context);
    assert_eq!(receipt["state"], "account_switched", "{receipt}");
    assert_eq!(receipt["replayed"], false);
    assert_eq!(receipt["unknown_operation_count"], 2);
    assert_eq!(receipt["unknown_in_flight_effects"][0]["state"], "unknown");
    assert!(!receipt.to_string().contains("sensitive-canary"));
    let current = load_session_record(&context, ID).unwrap();
    assert_eq!(current.id, record.id);
    assert_eq!(
        current.provider_resume.as_ref().unwrap().session_id,
        CONVERSATION
    );
    assert_eq!(
        current.runtime.as_ref().unwrap().generation,
        record.runtime.as_ref().unwrap().generation + 1
    );
    assert_ne!(launch_id(&current), launch_id(&record));
    assert_eq!(
        crate::claude_account::view_for_record(&current)
            .unwrap()
            .selected_account
            .as_deref(),
        Some("beta")
    );
    crate::coordination::auth_effects::test_support::assert_retired(&context, &record);
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &current).is_ok());
    let effects = &log(tmp.path())[before.len()..];
    assert!(!effects.contains("load-buffer"));
    assert!(!effects.contains("send-keys"));
    let launches = log(tmp.path()).matches("new-session").count();
    crate::auth_incident::failover::tick(
        &context,
        &current,
        &tmux,
        jiff::Timestamp::now().as_second(),
    )
    .unwrap();
    assert_eq!(log(tmp.path()).matches("new-session").count(), launches);
    crate::auth_incident::observe(
        &context,
        &current,
        "consecutive-auth-fault",
        crate::auth_incident::AuthSource::ClaudeStopFailure,
        crate::activity::Confidence::Authoritative,
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    // Runtime readiness is insufficient to reset the failure chain.
    tick(&context, &current, &tmux, 100);
    tick(&context, &current, &tmux, 220);
    let broker_calls = fs::read_to_string(tmp.path().join("broker.log")).unwrap();
    crate::auth_incident::failover::tick(
        &context,
        &current,
        &tmux,
        jiff::Timestamp::now().as_second(),
    )
    .unwrap();
    let store: Value = serde_json::from_slice(
        &fs::read(crate::session_dir(&context, ID).join("auth-incidents.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store["incidents"][1]["failover"]["reason"],
        "consecutive_fault"
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("broker.log")).unwrap(),
        broker_calls
    );
    assert_eq!(log(tmp.path()).matches("new-session").count(), launches);
    crate::auth_incident::recover(
        &context,
        &current,
        "healthy",
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    assert_eq!(failover_receipt(&context)["state"], "healthy");
}

#[test]
fn auth_failover_claude_no_candidate_keeps_the_failed_runtime_quarantined() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = failover_broker(tmp.path(), &lock, Value::Null);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, true);
    exhausted(&context, &record, &tmux);
    let before = log(tmp.path());
    crate::auth_incident::failover::tick(
        &context,
        &record,
        &tmux,
        jiff::Timestamp::now().as_second(),
    )
    .unwrap();
    assert_eq!(failover_receipt(&context)["reason"], "no_candidate");
    assert_eq!(log(tmp.path()), before);
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
}
