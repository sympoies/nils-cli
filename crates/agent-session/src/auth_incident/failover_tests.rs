use super::*;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

fn fixture(root: &Path) -> (CliContext, SessionRecord) {
    let context = CliContext {
        state_dir: root.join("state"),
        host: None,
    };
    let mut record = crate::codex_account::record_with_binding_value(json!({
        "schema_version": crate::codex_account::BINDING_SCHEMA_VERSION,
        "selected_account": "alpha", "revision": 7, "state": "bound",
        "applied_runtime_id": "runtime-binding-fixture", "updated_at": "2030-01-01T00:00:00Z"
    }));
    record.provider_resume = Some(
        serde_json::from_value(json!({
            "provider":"codex", "session_id":"fixture-thread", "captured_at":"2030-01-01T00:00:00Z",
            "capture_method":"fixture", "resume_args":[]
        }))
        .unwrap(),
    );
    fs::create_dir_all(session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let observed = jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() - 121)
        .unwrap()
        .to_string();
    observe(
        &context,
        &record,
        "auth-fault",
        AuthSource::CodexAppServer,
        Confidence::Authoritative,
        &observed,
    )
    .unwrap();
    (context, record)
}

fn broker(root: &Path, lock: &GlobalStateLock, account: Value, allowed: bool) -> EnvGuard {
    let now = jiff::Timestamp::now().as_second();
    fs::write(
        root.join("selection.json"),
        json!({
            "schema_version":"agent-session.codex-auth-broker.v1", "account":account,
            "auth_failover":{"policy_allowed":allowed, "capacity_confirmed":true,
                "checked_at_epoch":now,"expires_at_epoch":now+10}
        })
        .to_string(),
    )
    .unwrap();
    let script = root.join("broker");
    fs::write(&script, r#"#!/bin/sh
root=$1
shift
printf '%s\n' "$*" >> "$root/calls"
case "$1" in
list) printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","accounts":[{"account":"alpha"},{"account":"beta"}],"capabilities":["auth_failover_v1"],"selection_strategies":["next_with_capacity"]}' ;;
select) cat "$root/selection.json" ;;
*) exit 1 ;;
esac
"#).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    EnvGuard::set(
        lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &json!([script, root]).to_string(),
    )
}

fn reconcile(context: &CliContext, record: &SessionRecord) {
    tick(context, record, Path::new("unused"), "fixture", Some(false)).unwrap();
}

fn receipt(context: &CliContext, record: &SessionRecord) -> Value {
    let store = read(context, record).unwrap();
    serde_json::to_value(&store.incidents[0]).unwrap()["failover"].clone()
}

#[test]
fn auth_failover_no_candidate_is_durable_and_does_not_repeat_preflight() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, Value::Null, true);
    let (context, record) = fixture(tmp.path());
    reconcile(&context, &record);
    let first = receipt(&context, &record);
    assert_eq!(first["state"], "blocked_no_credential");
    assert_eq!(first["reason"], "no_candidate");
    let calls = fs::read_to_string(tmp.path().join("calls")).unwrap();
    assert!(calls.contains(
        "select --strategy next_with_capacity --after alpha --exclude alpha --reason authentication"
    ));
    reconcile(&context, &record);
    assert_eq!(receipt(&context, &record), first);
    assert_eq!(fs::read_to_string(tmp.path().join("calls")).unwrap(), calls);
    assert_eq!(
        crate::codex_account::selected_account(
            &crate::load_session_record(&context, &record.id).unwrap()
        )
        .as_deref(),
        Some("alpha")
    );
}

#[test]
fn auth_failover_policy_denial_cannot_queue_a_listed_account() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), false);
    let (context, record) = fixture(tmp.path());
    reconcile(&context, &record);
    assert_eq!(receipt(&context, &record)["reason"], "policy_denied");
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        crate::codex_account::selected_account(&current).as_deref(),
        Some("alpha")
    );
    assert!(
        crate::codex_account::view_for_record(&current)
            .next
            .is_none()
    );
}

#[test]
fn auth_failover_queues_once_and_keeps_the_applied_runtime_until_switch_completes() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), true);
    let (context, record) = fixture(tmp.path());
    reconcile(&context, &record);
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(receipt(&context, &record)["state"], "switching");
    let view = crate::codex_account::view_for_record(&current);
    assert_eq!(view.selected_account.as_deref(), Some("alpha"));
    assert_eq!(view.next.unwrap().account.as_deref(), Some("beta"));
    assert_eq!(
        current.runtime.as_ref().unwrap().launch_id,
        record.runtime.as_ref().unwrap().launch_id
    );
    assert_eq!(
        serde_json::to_value(&current.provider_resume).unwrap(),
        serde_json::to_value(&record.provider_resume).unwrap()
    );
    let calls = fs::read_to_string(tmp.path().join("calls")).unwrap();
    reconcile(&context, &current);
    assert_eq!(fs::read_to_string(tmp.path().join("calls")).unwrap(), calls);
}

#[test]
fn auth_failover_rechecks_policy_at_the_real_codex_apply_boundary() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), true);
    let (context, record) = fixture(tmp.path());
    reconcile(&context, &record);
    let path = tmp.path().join("selection.json");
    let mut selection: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    selection["auth_failover"]["policy_allowed"] = json!(false);
    fs::write(path, selection.to_string()).unwrap();
    let result = crate::codex_account::begin_next_apply(
        &context,
        &record.id,
        &record.runtime.as_ref().unwrap().launch_id,
    );
    assert!(
        result.is_err(),
        "a queued intent cannot outlive policy authorization"
    );
    assert_eq!(receipt(&context, &record)["reason"], "policy_denied");
    assert!(
        crate::codex_account::ensure_input_allowed(
            &crate::load_session_record(&context, &record.id).unwrap()
        )
        .is_err()
    );
}

#[test]
fn auth_failover_second_fault_cannot_flap_before_a_healthy_model_turn() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), true);
    let (context, record) = fixture(tmp.path());
    reconcile(&context, &record);
    let runtime = &record.runtime.as_ref().unwrap().launch_id;
    let next = crate::codex_account::begin_next_apply(&context, &record.id, runtime)
        .unwrap()
        .unwrap();
    crate::codex_account::finish_next_apply(
        &context,
        &record.id,
        runtime,
        &next.account,
        next.revision,
        next.intent_id.as_deref().unwrap(),
        Ok(()),
    )
    .unwrap();
    let current = crate::load_session_record(&context, &record.id).unwrap();
    reconcile(&context, &current);
    assert_eq!(receipt(&context, &current)["state"], "account_switched");
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &current).is_ok());
    let calls = fs::read_to_string(tmp.path().join("calls")).unwrap();
    let observed = jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() - 121)
        .unwrap()
        .to_string();
    observe(
        &context,
        &current,
        "second-fault",
        AuthSource::CodexAppServer,
        Confidence::Authoritative,
        &observed,
    )
    .unwrap();
    reconcile(&context, &current);
    let store = read(&context, &current).unwrap();
    assert_eq!(
        serde_json::to_value(store.incidents.last().unwrap()).unwrap()["failover"]["reason"],
        "consecutive_fault"
    );
    assert_eq!(fs::read_to_string(tmp.path().join("calls")).unwrap(), calls);
    assert_eq!(
        crate::codex_account::selected_account(
            &crate::load_session_record(&context, &record.id).unwrap()
        )
        .as_deref(),
        Some("beta")
    );
}

#[test]
fn auth_failover_crash_and_stale_runtime_do_not_repeat_the_switch() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), true);
    let (context, record) = fixture(tmp.path());
    reconcile(&context, &record);
    let mut store = read(&context, &record).unwrap();
    // A committed preflight with no durable completion must never be rerun.
    let path = session_dir(&context, &record.id).join(FILE);
    let mut value = serde_json::to_value(&store).unwrap();
    value["incidents"][0]["failover"]["state"] = json!("candidate_preflight");
    fs::write(&path, value.to_string()).unwrap();
    let calls = fs::read_to_string(tmp.path().join("calls")).unwrap();
    reconcile(&context, &record);
    assert_eq!(
        receipt(&context, &record)["reason"],
        "preflight_outcome_unknown"
    );
    assert_eq!(fs::read_to_string(tmp.path().join("calls")).unwrap(), calls);
    // A later runtime cannot inherit the old preflight or queued operation.
    store = read(&context, &record).unwrap();
    value = serde_json::to_value(&store).unwrap();
    value["incidents"][0]["failover"]["state"] = json!("switching");
    fs::write(&path, value.to_string()).unwrap();
    let mut replacement = record.clone();
    replacement.runtime.as_mut().unwrap().launch_id = "replacement-runtime".into();
    replacement.runtime.as_mut().unwrap().generation += 1;
    crate::write_session_record(&context, &replacement).unwrap();
    reconcile(&context, &replacement);
    assert_eq!(
        receipt(&context, &replacement)["reason"],
        "runtime_superseded"
    );
    assert_eq!(fs::read_to_string(tmp.path().join("calls")).unwrap(), calls);
}

#[test]
fn auth_failover_waits_for_same_account_budget_and_rejects_stale_capacity() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), true);
    let (context, record) = fixture(tmp.path());
    let mut store = read(&context, &record).unwrap();
    store.incidents[0].observed_at = jiff::Timestamp::now().to_string();
    save(&context, &record, &store).unwrap();
    reconcile(&context, &record);
    assert_eq!(receipt(&context, &record), Value::Null);
    assert!(!tmp.path().join("calls").exists());
    store.incidents[0].observed_at =
        jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() - 121)
            .unwrap()
            .to_string();
    save(&context, &record, &store).unwrap();
    let path = tmp.path().join("selection.json");
    let mut selection: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    selection["auth_failover"]["checked_at_epoch"] = json!(1);
    selection["auth_failover"]["expires_at_epoch"] = json!(2);
    fs::write(path, selection.to_string()).unwrap();
    reconcile(&context, &record);
    assert_eq!(
        receipt(&context, &record)["reason"],
        "capacity_unconfirmed_or_stale"
    );
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert!(
        crate::codex_account::view_for_record(&current)
            .next
            .is_none()
    );
}

#[test]
fn auth_failover_never_switches_with_an_unknown_official_refresh_outcome() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), true);
    let (context, record) = fixture(tmp.path());
    fs::write(session_dir(&context, &record.id).join("codex-auth-recovery.json"), json!({
        "schema_version":"agent-session.codex-auth-recovery.v1",
        "runtime_incarnation":record.runtime.as_ref().unwrap().launch_id,
        "runtime_generation":1,
        "operations":[{
            "operation_id":"uncertain-refresh", "account":"alpha", "binding_revision":7,
            "provider_session_id":"fixture-thread", "started_at_epoch_ms":1, "deadline_epoch_ms":2,
            "state":"response_unknown", "reason":null, "unknown_operation_count":0,
            "unknown_in_flight_effects":[], "untracked_effects":"unknown", "replayed":false
        }]
    }).to_string()).unwrap();
    reconcile(&context, &record);
    assert_eq!(
        receipt(&context, &record)["reason"],
        "same_account_outcome_unknown"
    );
    assert!(!tmp.path().join("calls").exists());
    let mut current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        crate::codex_account::authorize_proxy_input_locked(&context, &mut current, true)
            .unwrap_err()
            .code(),
        "auth-quarantined"
    );
}

#[test]
fn auth_failover_missing_policy_and_unavailable_preflight_never_fall_back() {
    let lock = GlobalStateLock::new();
    for mode in ["unadvertised", "unavailable"] {
        let tmp = tempfile::tempdir().unwrap();
        let _broker = broker(tmp.path(), &lock, json!("beta"), true);
        let (context, record) = fixture(tmp.path());
        let expected = if mode == "unadvertised" {
            let script = tmp.path().join("broker");
            let text = fs::read_to_string(&script)
                .unwrap()
                .replace("auth_failover_v1", "unrelated_v1");
            fs::write(script, text).unwrap();
            "policy_capability_missing"
        } else {
            fs::remove_file(tmp.path().join("selection.json")).unwrap();
            "preflight_unavailable"
        };
        reconcile(&context, &record);
        assert_eq!(receipt(&context, &record)["reason"], expected);
        assert!(
            crate::codex_account::view_for_record(
                &crate::load_session_record(&context, &record.id).unwrap()
            )
            .next
            .is_none()
        );
        let calls = fs::read_to_string(tmp.path().join("calls")).unwrap();
        reconcile(&context, &record);
        assert_eq!(fs::read_to_string(tmp.path().join("calls")).unwrap(), calls);
    }
}

#[test]
fn auth_failover_can_queue_the_explicit_repair_of_a_failed_binding() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), true);
    let (context, mut record) = fixture(tmp.path());
    record.extra.get_mut("codex_account_binding").unwrap()["state"] = json!("failed");
    crate::write_session_record(&context, &record).unwrap();
    reconcile(&context, &record);
    assert_eq!(receipt(&context, &record)["state"], "switching");
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        crate::codex_account::queued_switch_repair(&current).as_deref(),
        Some("beta")
    );
}

#[test]
fn auth_failover_refusal_does_not_override_a_later_explicit_account_switch() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), false);
    let (context, record) = fixture(tmp.path());
    reconcile(&context, &record);
    assert_eq!(receipt(&context, &record)["reason"], "policy_denied");
    let runtime = &record.runtime.as_ref().unwrap().launch_id;
    crate::session_account::switch(
        &context,
        &record.id,
        "beta",
        Some(runtime),
        Path::new("unused"),
    )
    .unwrap();
    let next = crate::codex_account::begin_next_apply(&context, &record.id, runtime)
        .unwrap()
        .unwrap();
    crate::codex_account::finish_next_apply(
        &context,
        &record.id,
        runtime,
        &next.account,
        next.revision,
        next.intent_id.as_deref().unwrap(),
        Ok(()),
    )
    .unwrap();
    let mut current = crate::load_session_record(&context, &record.id).unwrap();
    assert!(
        crate::codex_account::authorize_proxy_input_locked(&context, &mut current, false).is_ok(),
        "a successful explicit switch supersedes the earlier automatic refusal"
    );
}

#[test]
fn auth_failover_new_official_refresh_and_model_health_settle_prior_refusals() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), false);
    let (context, record) = fixture(tmp.path());
    reconcile(&context, &record);
    recover(
        &context,
        &record,
        "refresh_failed",
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    observe(
        &context,
        &record,
        "official-retry",
        AuthSource::CodexExternalRefresh,
        Confidence::Authoritative,
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    recover(
        &context,
        &record,
        "credentials_refreshed",
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_ok());
    assert_eq!(receipt(&context, &record)["state"], "credentials_refreshed");
    recover(
        &context,
        &record,
        "healthy",
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    assert_eq!(receipt(&context, &record)["state"], "healthy");
}

#[test]
fn auth_failover_tick_uses_post_broker_time_for_a_newer_stamp() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), true);
    let (context, record) = fixture(tmp.path());
    let before_broker = jiff::Timestamp::now().as_second() - 1;
    let path = tmp.path().join("selection.json");
    let mut selection: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    selection["auth_failover"]["checked_at_epoch"] = json!(before_broker + 1);
    selection["auth_failover"]["expires_at_epoch"] = json!(before_broker + 11);
    fs::write(path, selection.to_string()).unwrap();
    super::failover::tick(&context, &record, Path::new("unused"), before_broker).unwrap();
    let receipt = receipt(&context, &record);
    assert_eq!(receipt["state"], "switching", "{receipt}");
    assert_eq!(receipt["proof"]["checked_at_epoch"], before_broker + 1);
}

#[test]
fn auth_failover_codex_apply_uses_time_after_the_broker_returns() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), true);
    let (context, record) = fixture(tmp.path());
    reconcile(&context, &record);
    assert_eq!(receipt(&context, &record)["state"], "switching");
    let script = tmp.path().join("broker");
    let body = fs::read_to_string(&script).unwrap().replace(
        r#"select) cat "$root/selection.json" ;;"#,
        r#"select)
  sleep 1
  checked=$(date +%s)
  sed -e "s/\"checked_at_epoch\":[0-9]*/\"checked_at_epoch\":$checked/" \
      -e "s/\"expires_at_epoch\":[0-9]*/\"expires_at_epoch\":$((checked+10))/" "$root/selection.json"
  ;;"#,
    );
    fs::write(script, body).unwrap();
    let runtime = &record.runtime.as_ref().unwrap().launch_id;
    let next = crate::codex_account::begin_next_apply(&context, &record.id, runtime)
        .unwrap()
        .unwrap();
    crate::codex_account::finish_next_apply(
        &context,
        &record.id,
        runtime,
        &next.account,
        next.revision,
        next.intent_id.as_deref().unwrap(),
        Ok(()),
    )
    .unwrap();
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        crate::codex_account::selected_account(&current).as_deref(),
        Some("beta")
    );
    assert_eq!(current.runtime.as_ref().unwrap().launch_id, *runtime);
}

#[test]
fn auth_failover_concurrent_settlement_is_not_masked_by_a_stale_alert_receipt() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock, json!("beta"), false);
    let (context, record) = fixture(tmp.path());
    reconcile(&context, &record);
    let now = || jiff::Timestamp::now().to_string();
    recover(&context, &record, "refresh_failed", &now()).unwrap();
    observe(
        &context,
        &record,
        "official-retry",
        AuthSource::CodexExternalRefresh,
        Confidence::Authoritative,
        &now(),
    )
    .unwrap();
    recover(&context, &record, "credentials_refreshed", &now()).unwrap();

    // Delivery runs on a snapshot taken outside the record lock and alerts the
    // result that is current at that moment.
    let no_owner = |_: &AuthIncident, _, _: &Notification| {
        Err(CliError::runtime(
            "auth-owner-unavailable",
            "no owner route",
            None,
        ))
    };
    let mut pending = Store {
        schema_version: SCHEMA.into(),
        incidents: read(&context, &record).unwrap().incidents,
        ..Store::default()
    };
    deliver_with(&mut pending, no_owner);
    escalation::route_unowned(&context, &mut pending.incidents);

    // A supported settlement lands before the snapshot is merged back.
    recover(&context, &record, "healthy", &now()).unwrap();
    let mut store = read(&context, &record).unwrap();
    merge_delivery(&mut store, pending.incidents);
    // The next tick delivers the settled result and alerts it.
    deliver_with(&mut store, no_owner);
    escalation::route_unowned(&context, &mut store.incidents);

    let healthy = store
        .incidents
        .iter()
        .find(|incident| incident.recovery_result.as_deref() == Some("healthy"))
        .unwrap();
    let expected = format!("sha256:{}", key(&healthy.incident_id, "recovery:healthy"));
    let alerts = nils_common::observation::read_recent(&context.state_dir, 0).unwrap();
    assert!(
        alerts
            .iter()
            .any(|alert| alert.correlation.as_deref() == Some(expected.as_str())),
        "the healthy result was never alerted: {alerts:?}"
    );
}
