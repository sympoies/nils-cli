//! Same-account rechecks distinguish publication from resident adoption before
//! releasing authentication quarantine.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

#[path = "auth_recheck_publication_tests.rs"]
mod publication_tests;

/// A real `StopFailure` hook leaves both the incident and the failed turn that
/// quarantine the runtime. Recovery then exhausts and the failover refuses for
/// lack of a policy-capable broker, exactly the state operators see.
fn quarantined(context: &CliContext, root: &Path, tmux: &Path) -> SessionRecord {
    seed_exited_session(context, root);
    let launch = running_session(context, tmux);
    session_ready(context, &launch);
    let mut record = load_session_record(context, ID).unwrap();
    crate::claude_account::bind_initial(&mut record, "alpha", "explicit").unwrap();
    crate::write_session_record(context, &record).unwrap();
    claude_hook(
        context,
        &launch,
        json!({
            "hook_event_name": "StopFailure", "error": "authentication_failed",
            "session_id": CONVERSATION,
        }),
    );
    tick(context, &record, tmux, 100);
    tick(context, &record, tmux, 221);
    crate::auth_incident::failover::tick(
        context,
        &record,
        tmux,
        jiff::Timestamp::now().as_second(),
    )
    .unwrap();
    let store = incident_store(context);
    assert_eq!(
        store["incidents"][0]["failover"]["state"], "blocked_no_credential",
        "{store}"
    );
    assert!(crate::auth_incident::recovery::ensure_input_allowed(context, &record).is_err());
    record
}

fn incident_store(context: &CliContext) -> Value {
    serde_json::from_slice(
        &fs::read(crate::session_dir(context, ID).join("auth-incidents.json")).unwrap(),
    )
    .unwrap()
}

fn recheck(context: &CliContext, record: &SessionRecord, tmux: &Path) -> Result<(), CliError> {
    crate::session_account::claude_switch_locked(
        context,
        ID,
        "alpha",
        &launch_id(record).unwrap(),
        tmux,
        true,
    )
    .map(|_| ())
}

#[test]
fn auth_recheck_requires_runtime_adoption_after_credentials_are_republished() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    crate::auth_incident::tick(&context, &record, &tmux, "fixture", Some(false)).unwrap();
    publish(tmp.path(), 2);
    let before = log(tmp.path());
    let path = crate::session_dir(&context, ID).join("auth-incidents.json");
    let incidents_before = fs::read(&path).unwrap();
    let error = recheck(&context, &record, &tmux).unwrap_err();
    let details = error.details().unwrap();
    assert_eq!(details["reason"], "runtime_adoption_required");
    assert_eq!(details["next_action"], "restart_same_account");
    assert_eq!(details["credential_materialized"], true);
    assert_eq!(details["runtime_adoption"], "unconfirmed");
    assert_eq!(details["provider_request"], "unverified");
    assert_eq!(fs::read(&path).unwrap(), incidents_before);
    let store = incident_store(&context);
    assert_eq!(store["incidents"][0]["status"], "auth_failed", "{store}");
    let current = load_session_record(&context, ID).unwrap();
    assert_eq!(launch_id(&current), launch_id(&record));
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &current).is_err());
    let effects = &log(tmp.path())[before.len()..];
    assert!(!effects.contains("kill-session"));
    assert!(!effects.contains("new-session"));
    assert!(!effects.contains("paste-buffer"));

    let watcher = on_relaunched_prompt(&context, tmp.path(), &launch_id(&record).unwrap());
    let result = restart(&context, &restart_args(&tmux, false, 10));
    watcher.join().unwrap();
    assert_eq!(result.unwrap().outcome, "ready");
    let replacement = load_session_record(&context, ID).unwrap();
    assert_ne!(launch_id(&replacement), launch_id(&record));
    assert_eq!(
        crate::claude_account::generation::baseline(&replacement)
            .unwrap()
            .version
            .generation,
        "2"
    );
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &replacement).is_ok());
    assert_eq!(
        incident_store(&context)["incidents"][0]["status"],
        "auth_failed"
    );
    super::guarded_send::assert_submitted(&context, tmp.path(), &tmux, &replacement);
    super::guarded_send::assert_retained_after_tick(&context, &tmux, &replacement);
}

#[test]
fn auth_recheck_unchanged_valid_publication_requires_independent_provider_proof() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    // A valid broker observation and materialization prove neither runtime
    // credential adoption nor a successful provider request after the failure.
    let mut publication = crate::claude_account::generation::tests::envelope(1);
    publication["state"]["authority"]["auth"]["state"] = json!("valid");
    fs::write(
        tmp.path().join("credential-state.json"),
        publication.to_string(),
    )
    .unwrap();
    let path = crate::session_dir(&context, ID).join("auth-incidents.json");
    let before = fs::read(&path).unwrap();
    let effects = log(tmp.path());
    let error = recheck(&context, &record, &tmux).unwrap_err();
    assert_eq!(error.code(), "claude-account-recheck-refused");
    let details = error.details().unwrap();
    assert_eq!(details["reason"], "provider_verification_required");
    assert_eq!(details["next_action"], "restart_same_account");
    assert_eq!(details["credential_materialized"], true);
    assert_eq!(details["runtime_adoption"], "unconfirmed");
    assert_eq!(details["provider_request"], "unverified");
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(log(tmp.path()), effects);
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
}

#[test]
fn auth_recheck_guidance_preserves_drafts_and_restarts_the_same_account() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    crate::auth_incident::tick(&context, &record, &tmux, "fixture", Some(false)).unwrap();
    let launch = launch_id(&record).unwrap();
    let screen = fs::read(tmp.path().join("screen")).unwrap();
    fs::write(
        tmp.path().join("screen"),
        "────────────────────\n❯ synthetic draft\n────────────────────\n  status\n",
    )
    .unwrap();
    let before = log(tmp.path());
    let error = restart(&context, &restart_args(&tmux, false, 10)).unwrap_err();
    assert_eq!(error.code(), "restart-session-not-idle");
    assert_eq!(
        launch_id(&load_session_record(&context, ID).unwrap()),
        Some(launch.clone())
    );
    assert!(!log(tmp.path())[before.len()..].contains("paste-buffer"));
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
    // The operator preserves the draft before taking the guarded restart.
    fs::write(tmp.path().join("screen"), screen).unwrap();
    let watcher = on_relaunched_prompt(&context, tmp.path(), &launch);
    let result = restart(&context, &restart_args(&tmux, false, 10));
    watcher.join().unwrap();
    assert_eq!(result.unwrap().outcome, "ready");
    let current = load_session_record(&context, ID).unwrap();
    assert_ne!(launch_id(&current), Some(launch));
    assert_eq!(
        serde_json::to_value(&current.provider_resume).unwrap(),
        serde_json::to_value(&record.provider_resume).unwrap()
    );
    assert_eq!(
        crate::claude_account::view_for_record(&current)
            .unwrap()
            .selected_account
            .as_deref(),
        Some("alpha")
    );
    assert_eq!(
        crate::claude_account::generation::baseline(&current),
        crate::claude_account::generation::baseline(&record)
    );
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &current).is_ok());
    assert_eq!(
        fs::read_to_string(tmp.path().join("pasted-input")).unwrap(),
        "/exit"
    );
    // Readiness belongs to the new incarnation; it is not provider health and
    // must not rewrite the failed predecessor's evidence as recovered.
    assert_eq!(
        incident_store(&context)["incidents"][0]["status"],
        "auth_failed"
    );
    super::guarded_send::assert_submitted(&context, tmp.path(), &tmux, &current);
    super::guarded_send::assert_retained_after_tick(&context, &tmux, &current);
    claude_hook(
        &context,
        &launch_id(&current).unwrap(),
        json!({
            "hook_event_name": "StopFailure", "error": "authentication_failed",
            "session_id": CONVERSATION,
        }),
    );
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &current).is_err());
    assert_eq!(
        incident_store(&context)["incidents"][1]["status"],
        "auth_failed"
    );
}

#[test]
fn auth_recheck_revoked_publication_remains_quarantined() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    let mut publication = crate::claude_account::generation::tests::envelope(1);
    publication["state"]["authority"]["auth"]["state"] = json!("revoked");
    fs::write(
        tmp.path().join("credential-state.json"),
        publication.to_string(),
    )
    .unwrap();
    let path = crate::session_dir(&context, ID).join("auth-incidents.json");
    let before = fs::read(&path).unwrap();
    let error = recheck(&context, &record, &tmux).unwrap_err();
    assert_eq!(error.code(), "claude-account-recheck-refused");
    assert_eq!(
        error.details().unwrap()["reason"],
        "credentials_not_republished"
    );
    assert_eq!(fs::read(path).unwrap(), before);
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
}

#[test]
fn auth_recheck_refuses_while_credentials_are_unrepaired() {
    let lock = GlobalStateLock::new();
    for scenario in ["not_republished", "credentials_missing"] {
        let tmp = tempfile::tempdir().unwrap();
        let _broker = broker(tmp.path(), &lock);
        let context = context(tmp.path());
        let tmux = fake_tmux(tmp.path());
        let record = quarantined(&context, tmp.path(), &tmux);
        if scenario == "credentials_missing" {
            publish(tmp.path(), 2);
            fs::remove_file(tmp.path().join("alpha/.credentials.json")).unwrap();
        }
        let before =
            fs::read(crate::session_dir(&context, ID).join("auth-incidents.json")).unwrap();
        let error = recheck(&context, &record, &tmux).unwrap_err();
        assert_eq!(error.code(), "claude-account-recheck-refused", "{scenario}");
        assert_eq!(
            fs::read(crate::session_dir(&context, ID).join("auth-incidents.json")).unwrap(),
            before,
            "{scenario}"
        );
        let current = load_session_record(&context, ID).unwrap();
        let error =
            crate::auth_incident::recovery::ensure_input_allowed(&context, &current).unwrap_err();
        assert_eq!(error.code(), "auth-quarantined", "{scenario}");
    }
}

#[test]
fn auth_recheck_does_not_hide_a_later_authentication_failure() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    publish(tmp.path(), 2);
    // Independent observed health may settle in place; publication alone may
    // not. A later recheck must not conceal a new failure on that same runtime.
    let launch = launch_id(&record).unwrap();
    claude_hook(
        &context,
        &launch,
        json!({"hook_event_name": "UserPromptSubmit", "session_id": CONVERSATION,
            "prompt": "synthetic healthy turn"}),
    );
    claude_hook(
        &context,
        &launch,
        json!({"hook_event_name": "Notification", "notification_type": "idle_prompt",
            "session_id": CONVERSATION}),
    );
    assert_eq!(
        incident_store(&context)["incidents"][0]["recovery_result"],
        "healthy"
    );
    recheck(&context, &record, &tmux).unwrap();
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_ok());
    std::thread::sleep(std::time::Duration::from_millis(20));
    claude_hook(
        &context,
        &launch,
        json!({"hook_event_name": "UserPromptSubmit", "session_id": CONVERSATION, "prompt": "work"}),
    );
    claude_hook(
        &context,
        &launch,
        json!({
            "hook_event_name": "StopFailure", "error": "authentication_failed",
            "session_id": CONVERSATION,
        }),
    );
    let current = load_session_record(&context, ID).unwrap();
    let store = incident_store(&context);
    assert_eq!(store["incidents"].as_array().unwrap().len(), 2);
    assert_eq!(store["incidents"][1]["status"], "auth_failed");
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &current).is_err());
}

#[test]
fn auth_recheck_leaves_a_session_healed_by_a_later_turn_untouched() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    std::thread::sleep(std::time::Duration::from_millis(20));
    crate::auth_incident::recover(
        &context,
        &record,
        "healthy",
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    let store = incident_store(&context);
    assert_eq!(store["incidents"][0]["status"], "recovered", "{store}");
    assert_eq!(store["incidents"][0]["recovery_result"], "healthy");
    // The recovery receipt stays blocked, but it no longer fences input, so a
    // same-account switch has nothing to settle and must not rewrite the result.
    let before = fs::read(crate::session_dir(&context, ID).join("auth-incidents.json")).unwrap();
    recheck(&context, &record, &tmux).unwrap();
    assert_eq!(
        fs::read(crate::session_dir(&context, ID).join("auth-incidents.json")).unwrap(),
        before
    );
}

#[test]
fn daemon_recheck_keeps_failed_resident_quarantined_after_publication() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    publish(tmp.path(), 2);
    let path = crate::session_dir(&context, ID).join("auth-incidents.json");
    let mut store = incident_store(&context);
    store["incidents"][0]["observed_at"] = json!(
        jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() - 31)
            .unwrap()
            .to_string()
    );
    fs::write(path, serde_json::to_vec(&store).unwrap()).unwrap();
    let before = fs::read_to_string(tmp.path().join("broker.log")).unwrap();
    crate::auth_incident::tick(&context, &record, &tmux, "fixture", Some(false)).unwrap();
    let after = fs::read_to_string(tmp.path().join("broker.log")).unwrap();
    let store = incident_store(&context);
    assert_eq!(
        store["incidents"][0]["status"],
        "auth_failed",
        "publication cannot prove adoption; broker calls after tick: {:?}",
        &after[before.len()..]
    );
    assert_eq!(
        store["incidents"][0]["auto_settle"]["attempts"][0]["result"],
        "runtime_adoption_required"
    );
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
    assert!(after[before.len()..].contains("materialize"));
    let current = load_session_record(&context, ID).unwrap();
    assert_eq!(launch_id(&current), launch_id(&record));
}

#[test]
fn scheduled_recheck_refusals_exhaust_and_survive_daemon_recreation() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(&fs::canonicalize(tmp.path()).unwrap());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    let start = incident_store(&context)["incidents"][0]["observed_at"]
        .as_str()
        .unwrap()
        .parse::<jiff::Timestamp>()
        .unwrap()
        .as_second();
    let before = fs::read_to_string(tmp.path().join("broker.log")).unwrap();
    for age in [0, 29] {
        crate::auth_incident::auto_settle::tick(&context, &record, start + age).unwrap();
    }
    assert_eq!(
        fs::read_to_string(tmp.path().join("broker.log")).unwrap(),
        before
    );
    for (age, count) in [(30, 1), (119, 1), (120, 2), (599, 2), (600, 3), (900, 3)] {
        // Recreate daemon inputs from disk on every tick: no in-memory budget.
        let context = CliContext {
            state_dir: context.state_dir.clone(),
            host: None,
        };
        let record = load_session_record(&context, ID).unwrap();
        crate::auth_incident::auto_settle::tick(&context, &record, start + age).unwrap();
        let store = incident_store(&context);
        let receipt = &store["incidents"][0]["auto_settle"];
        assert_eq!(receipt["attempts"].as_array().unwrap().len(), count);
        assert_eq!(
            receipt["attempts"][count - 1]["result"],
            "provider_verification_required"
        );
    }
    let store = incident_store(&context);
    assert_eq!(store["incidents"][0]["auto_settle"]["state"], "exhausted");
    assert_eq!(
        store["incidents"][0]["recovery_result"],
        "recheck_exhausted"
    );
    let after = fs::read_to_string(tmp.path().join("broker.log")).unwrap();
    assert_eq!(after[before.len()..].matches("materialize").count(), 3);
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
    crate::auth_incident::tick(&context, &record, &tmux, "fixture", Some(false)).unwrap();
    let store = incident_store(&context);
    let notification = &store["incidents"][0]["recovery_notification"];
    assert_eq!(notification["degraded_reason"], "owner_unavailable");
    assert_eq!(notification["escalation"]["route"], "observation_spool");
    assert_eq!(notification["escalation"]["state"], "recorded");
    let alerts = crate::auth_incident::alert_delivery::list(&context, 100).unwrap();
    let recovery_alert = alerts["alerts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|alert| alert["result"] == "recheck_exhausted")
        .unwrap();
    assert_eq!(recovery_alert["recovery_action"], "restart_same_account");
    let events = nils_common::observation::read_recent(&context.state_dir, 0).unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.code == "auth-recovery-owner-unavailable")
    );
}

#[test]
fn scheduled_recheck_preserves_a_queued_different_account_intent() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let mut record = quarantined(&context, tmp.path(), &tmux);
    publish(tmp.path(), 2);
    record.extra.insert(
        "claude_account_next".into(),
        json!({
            "schema_version":"agent-session.claude-account-next.v1", "account":"beta",
            "revision":3, "state":"queued", "updated_at":"2030-01-01T00:00:00Z"
        }),
    );
    crate::write_session_record(&context, &record).unwrap();
    let before = fs::read(crate::session_dir(&context, ID).join("session.json")).unwrap();
    let broker_before = fs::read_to_string(tmp.path().join("broker.log")).unwrap();
    crate::auth_incident::auto_settle::tick(&context, &record, i64::MAX / 1000).unwrap();
    assert_eq!(
        fs::read(crate::session_dir(&context, ID).join("session.json")).unwrap(),
        before
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("broker.log")).unwrap(),
        broker_before
    );
    let store = incident_store(&context);
    assert_eq!(
        store["incidents"][0]["auto_settle"]["refusal"],
        "next_account_queued"
    );
    assert_eq!(store["incidents"][0]["status"], "auth_failed");
}

#[test]
fn scheduled_recheck_applies_to_a_carried_quarantine_on_the_new_runtime() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    let start = jiff::Timestamp::now().as_second();
    crate::auth_incident::auto_settle::tick(&context, &record, start + 30).unwrap();
    let mut replacement = record.clone();
    let runtime = replacement.runtime.as_mut().unwrap();
    runtime.launch_id = "replacement-runtime".into();
    runtime.generation += 1;
    runtime.started_at = jiff::Timestamp::from_second(start + 50)
        .unwrap()
        .to_string();
    replacement.extra.get_mut("claude_account_binding").unwrap()["applied_runtime_id"] =
        json!("replacement-runtime");
    crate::write_session_record(&context, &replacement).unwrap();
    let path = crate::session_dir(&context, ID).join("auth-incidents.json");
    let mut store = incident_store(&context);
    store["incidents"][0]["failover"]["new_runtime_incarnation"] = json!("replacement-runtime");
    store["incidents"][0]["failover"]["new_runtime_generation"] =
        json!(replacement.runtime.as_ref().unwrap().generation);
    fs::write(path, serde_json::to_vec(&store).unwrap()).unwrap();
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &replacement).is_err());
    let before = fs::read(crate::session_dir(&context, ID).join("session.json")).unwrap();
    let incidents_before =
        fs::read(crate::session_dir(&context, ID).join("auth-incidents.json")).unwrap();
    let broker_before = fs::read_to_string(tmp.path().join("broker.log")).unwrap();
    let error = crate::auth_incident::auto_settle::tick(&context, &record, start + 80).unwrap_err();
    assert_eq!(error.code(), "claude-account-recheck-refused");
    assert_eq!(error.details().unwrap()["reason"], "runtime_changed");
    assert_eq!(
        fs::read(crate::session_dir(&context, ID).join("session.json")).unwrap(),
        before
    );
    assert_eq!(
        fs::read(crate::session_dir(&context, ID).join("auth-incidents.json")).unwrap(),
        incidents_before
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("broker.log")).unwrap(),
        broker_before
    );
    publish(tmp.path(), 2);
    crate::auth_incident::auto_settle::tick(&context, &replacement, start + 79).unwrap();
    assert_eq!(
        incident_store(&context)["incidents"][0]["auto_settle"]["attempts"],
        json!([])
    );
    crate::auth_incident::auto_settle::tick(&context, &replacement, start + 80).unwrap();
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &replacement).is_ok());
    let projected =
        serde_json::to_value(crate::auth_incident::view(&context, &replacement).unwrap()).unwrap();
    assert_eq!(
        projected["auto_settle"]["runtime_incarnation"],
        "replacement-runtime"
    );
    assert_eq!(projected["auto_settle"]["state"], "settled");
}

#[test]
fn scheduled_recheck_records_account_mismatch_and_in_flight_refusals() {
    let lock = GlobalStateLock::new();
    for reason in [
        "account_changed",
        "recovery_in_progress",
        "failover_in_progress",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let _broker = broker(tmp.path(), &lock);
        let context = context(tmp.path());
        let tmux = fake_tmux(tmp.path());
        let mut record = quarantined(&context, tmp.path(), &tmux);
        publish(tmp.path(), 2);
        if reason == "account_changed" {
            record.extra.get_mut("claude_account_binding").unwrap()["selected_account"] =
                json!("beta");
            crate::write_session_record(&context, &record).unwrap();
        } else if reason == "recovery_in_progress" {
            change_receipt(&context, "resuming");
        } else {
            let path = crate::session_dir(&context, ID).join("auth-incidents.json");
            let mut store = incident_store(&context);
            store["incidents"][0]["failover"]["state"] = json!("candidate_preflight");
            fs::write(path, serde_json::to_vec(&store).unwrap()).unwrap();
        }
        let before = fs::read(crate::session_dir(&context, ID).join("session.json")).unwrap();
        let broker_before = fs::read_to_string(tmp.path().join("broker.log")).unwrap();
        crate::auth_incident::auto_settle::tick(
            &context,
            &record,
            jiff::Timestamp::now().as_second() + 30,
        )
        .unwrap();
        let store = incident_store(&context);
        let receipt = &store["incidents"][0]["auto_settle"];
        if reason == "account_changed" {
            assert_eq!(receipt["refusal"], reason);
            assert_eq!(receipt["attempts"], json!([]));
        } else {
            assert_eq!(receipt["attempts"][0]["result"], "recovery_in_progress");
        }
        assert_eq!(store["incidents"][0]["status"], "auth_failed");
        assert_eq!(
            fs::read_to_string(tmp.path().join("broker.log")).unwrap(),
            broker_before
        );
        assert_eq!(
            fs::read(crate::session_dir(&context, ID).join("session.json")).unwrap(),
            before
        );
    }
}

#[test]
fn scheduled_recheck_reconciles_observed_health_and_interrupted_settlement() {
    let lock = GlobalStateLock::new();
    for result in ["healthy", "interrupted"] {
        let tmp = tempfile::tempdir().unwrap();
        let _broker = broker(tmp.path(), &lock);
        let context = context(tmp.path());
        let tmux = fake_tmux(tmp.path());
        let record = quarantined(&context, tmp.path(), &tmux);
        let start = jiff::Timestamp::now().as_second();
        crate::auth_incident::auto_settle::tick(&context, &record, start).unwrap();
        crate::auth_incident::recover(
            &context,
            &record,
            "healthy",
            &jiff::Timestamp::now().to_string(),
        )
        .unwrap();
        if result == "interrupted" {
            let path = crate::session_dir(&context, ID).join("auth-incidents.json");
            let mut store = incident_store(&context);
            store["incidents"][0]["auto_settle"]["attempts"] = json!([
                {"attempted_at_epoch":start, "result":"started"}
            ]);
            fs::write(path, serde_json::to_vec(&store).unwrap()).unwrap();
        }
        let before = fs::read_to_string(tmp.path().join("broker.log")).unwrap();
        crate::auth_incident::auto_settle::tick(&context, &record, start + 1000).unwrap();
        let store = incident_store(&context);
        assert_eq!(store["incidents"][0]["auto_settle"]["state"], "settled");
        assert_eq!(
            fs::read_to_string(tmp.path().join("broker.log")).unwrap(),
            before
        );
    }
}

#[test]
fn scheduled_recheck_bounds_credential_preflight_refusal_output() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    publish(tmp.path(), 2);
    fs::remove_file(tmp.path().join("alpha/.credentials.json")).unwrap();
    crate::auth_incident::auto_settle::tick(
        &context,
        &record,
        jiff::Timestamp::now().as_second() + 30,
    )
    .unwrap();
    let projected =
        serde_json::to_value(crate::auth_incident::view(&context, &record).unwrap()).unwrap();
    let receipt = &projected["auto_settle"];
    assert_eq!(
        receipt["attempts"][0]["result"],
        "credential_preflight_refused"
    );
    assert_eq!(receipt["state"], "scheduled");
    assert_eq!(receipt["runtime_incarnation"], launch_id(&record).unwrap());
    assert_eq!(
        receipt["runtime_generation"],
        record.runtime.as_ref().unwrap().generation
    );
    assert_eq!(receipt["refusal"], Value::Null);
    assert!(receipt["started_at_epoch"].is_i64());
    assert!(receipt["attempts"][0]["attempted_at_epoch"].is_i64());
    assert_eq!(receipt.as_object().unwrap().len(), 6);
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
}

#[test]
fn delayed_first_scheduled_recheck_retains_the_failed_launch_publication_fence() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, true);
    assert!(crate::claude_account::generation::baseline(&record).is_some());
    assert_eq!(
        incident_store(&context)["incidents"][0]["same_account_recovery"],
        Value::Null
    );
    let now = jiff::Timestamp::now().as_second() + 31;
    crate::auth_incident::auto_settle::tick(&context, &record, now).unwrap();
    let store = incident_store(&context);
    assert_eq!(store["incidents"][0]["status"], "auth_failed");
    assert_eq!(
        store["incidents"][0]["auto_settle"]["attempts"][0]["result"],
        "provider_verification_required"
    );
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
    assert_eq!(store["incidents"][0]["same_account_recovery"], Value::Null);
    publish(tmp.path(), 2);
    crate::auth_incident::auto_settle::tick(&context, &record, now + 120).unwrap();
    assert_eq!(
        incident_store(&context)["incidents"][0]["auto_settle"]["attempts"][1]["result"],
        "runtime_adoption_required"
    );
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
}

#[test]
fn scheduled_recheck_prioritizes_current_failure_over_carried_baseline() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let original = quarantined(&context, tmp.path(), &tmux);
    let old_checkpoint = crate::claude_account::generation::baseline(&original).unwrap();
    publish(tmp.path(), 2);
    let mut replacement = original.clone();
    let runtime = replacement.runtime.as_mut().unwrap();
    runtime.launch_id = "replacement-runtime".into();
    runtime.generation += 1;
    runtime.started_at = jiff::Timestamp::now().to_string();
    replacement.extra.get_mut("claude_account_binding").unwrap()["applied_runtime_id"] =
        json!("replacement-runtime");
    crate::claude_account::generation::capture(&mut replacement, "alpha");
    assert!(
        crate::claude_account::generation::baseline(&replacement)
            .unwrap()
            .newer_than(&old_checkpoint)
    );
    crate::write_session_record(&context, &replacement).unwrap();
    let path = crate::session_dir(&context, ID).join("auth-incidents.json");
    let mut store = incident_store(&context);
    let prior = &mut store["incidents"][0]["same_account_recovery"];
    prior["state"] = json!("blocked");
    prior["last_reason"] = json!("runtime_not_ready");
    prior["new_runtime_incarnation"] = json!("replacement-runtime");
    prior["new_runtime_generation"] = json!(replacement.runtime.as_ref().unwrap().generation);
    fs::write(path, serde_json::to_vec(&store).unwrap()).unwrap();
    crate::auth_incident::observe(
        &context,
        &replacement,
        "replacement-auth-failure",
        crate::auth_incident::AuthSource::ClaudeStopFailure,
        crate::activity::Confidence::Authoritative,
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    let store = incident_store(&context);
    assert_eq!(store["incidents"].as_array().unwrap().len(), 2);
    assert_eq!(store["incidents"][1]["same_account_recovery"], Value::Null);
    let now = jiff::Timestamp::now().as_second() + 31;
    crate::auth_incident::auto_settle::tick(&context, &replacement, now).unwrap();
    let store = incident_store(&context);
    assert!(
        store["incidents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|incident| incident["status"] == "auth_failed"),
        "unchanged failed publication must not settle either incident: {store}"
    );
    assert_eq!(
        store["incidents"][1]["auto_settle"]["attempts"][0]["result"],
        "provider_verification_required"
    );
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &replacement).is_err());
    publish(tmp.path(), 3);
    crate::auth_incident::auto_settle::tick(&context, &replacement, now + 120).unwrap();
    let store = incident_store(&context);
    assert!(
        store["incidents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|incident| incident["status"] == "auth_failed")
    );
    assert_eq!(
        store["incidents"][1]["auto_settle"]["attempts"][1]["result"],
        "runtime_adoption_required"
    );
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &replacement).is_err());
}
