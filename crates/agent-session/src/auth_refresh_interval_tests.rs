//! Synthetic lifecycle evidence, not live Claude or broker acceptance (#2954).
//! The provider model rejects the resident generation during exchange; later
//! publication cannot change it without a supported adoption path.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

fn safe_request(context: &CliContext, record: &SessionRecord, accepted: &str) -> bool {
    crate::auth_incident::recovery::ensure_input_allowed(context, record).is_ok()
        && crate::claude_account::generation::baseline(record)
            .is_some_and(|publication| publication.version.generation == accepted)
}

fn exchange_failure(context: &CliContext, root: &Path, tmux: &Path) -> SessionRecord {
    seed_exited_session(context, root);
    let launch = running_session(context, tmux);
    session_ready(context, &launch);
    let mut record = load_session_record(context, ID).unwrap();
    crate::claude_account::bind_initial(&mut record, "alpha", "explicit").unwrap();
    crate::write_session_record(context, &record).unwrap();
    assert!(safe_request(context, &record, "1"));
    // The request started with accepted generation 1. The exchange now rejects
    // it, while generation 2 is not yet available to the access-only consumer.
    assert!(!safe_request(context, &record, "2"));
    claude_hook(
        context,
        &launch,
        json!({"hook_event_name":"StopFailure", "error":"authentication_failed",
            "session_id":CONVERSATION}),
    );
    record
}

#[test]
fn refresh_interval_publication_cannot_admit_the_revoked_resident_runtime() {
    let lock = GlobalStateLock::new();
    for delayed_first in [false, true] {
        assert_refresh_interval(&lock, delayed_first);
    }
}

fn assert_refresh_interval(lock: &GlobalStateLock, delayed_first: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = exchange_failure(&context, tmp.path(), &tmux);
    let launch = launch_id(&record).unwrap();
    let start = jiff::Timestamp::now().as_second();
    let before = log(tmp.path());
    if !delayed_first {
        tick(&context, &record, &tmux, start);
        assert_eq!(recovery_receipt(&context)["state"], "waiting_generation");
    }
    // Virtual t=15: publication completes independently of runtime adoption.
    publish(tmp.path(), 2);
    assert_eq!(
        crate::claude_account::generation::published("alpha")
            .unwrap()
            .unwrap()
            .version
            .generation,
        "2"
    );
    assert!(!safe_request(&context, &record, "2"));

    // A delayed first tick runs scheduled recheck before lifecycle recovery.
    crate::auth_incident::auto_settle::tick(&context, &record, start + 30).unwrap();
    assert!(
        crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err(),
        "publication cleared quarantine without adoption; a safe request still fails"
    );
    assert_eq!(
        launch_id(&load_session_record(&context, ID).unwrap()),
        Some(launch.clone())
    );
    assert!(!log(tmp.path())[before.len()..].contains("paste-buffer"));

    // Existing lifecycle consumes the proven replacement without replaying
    // the failed request. Readiness is separately required before admission.
    fs::write(tmp.path().join("exit-on-probe"), "").unwrap();
    let watcher = on_relaunched_prompt(&context, tmp.path(), &launch);
    let lifecycle_started = Instant::now();
    tick(&context, &record, &tmux, start + 30);
    watcher.join().unwrap();
    let current = load_session_record(&context, ID).unwrap();
    assert_eq!(recovery_receipt(&context)["state"], "runtime_resumed");
    assert_eq!(current.id, record.id);
    assert_eq!(
        current.provider_resume.as_ref().unwrap().session_id,
        CONVERSATION
    );
    assert_ne!(launch_id(&current), Some(launch));
    assert!(safe_request(&context, &current, "2"));
    assert_eq!(recovery_receipt(&context)["replayed"], false);
    assert!(!log(tmp.path())[before.len()..].contains("paste-buffer"));
    eprintln!(
        "synthetic timeline (delayed_first={delayed_first}): rejection=0s publication=15s adoption/safe-request=30s + {}ms lifecycle; publication-to-safe-request=15s + lifecycle; live timing UNRUN",
        lifecycle_started.elapsed().as_millis()
    );
}

#[test]
fn refresh_interval_missing_failed_or_late_publication_remains_bounded() {
    let lock = GlobalStateLock::new();
    for outcome in ["missing", "failed", "late"] {
        let tmp = tempfile::tempdir().unwrap();
        let _broker = broker(tmp.path(), &lock);
        let context = context(tmp.path());
        let tmux = fake_tmux(tmp.path());
        let record = exchange_failure(&context, tmp.path(), &tmux);
        let start = jiff::Timestamp::now().as_second();
        if outcome != "late" {
            let mut state = crate::claude_account::generation::tests::envelope(2);
            state["state"]["publication"]["status"] = json!(outcome);
            fs::write(tmp.path().join("credential-state.json"), state.to_string()).unwrap();
        }
        for age in [0, 30, 120] {
            tick(&context, &record, &tmux, start + age);
            crate::auth_incident::auto_settle::tick(&context, &record, start + age).unwrap();
        }
        let receipt = recovery_receipt(&context);
        assert_eq!(receipt["state"], "blocked");
        assert_eq!(receipt["last_reason"], "publication_deadline");
        // No account-failover policy is advertised. Establish that separate
        // refusal before the final recheck result is enqueued for escalation.
        crate::auth_incident::failover::tick(&context, &record, &tmux, start + 120).unwrap();
        if outcome == "late" {
            // Completion after the controller's deadline must not reopen the
            // revoked resident runtime through the independent recheck route.
            publish(tmp.path(), 2);
        }
        crate::auth_incident::auto_settle::tick(&context, &record, start + 600).unwrap();
        let store: Value = serde_json::from_slice(
            &fs::read(crate::session_dir(&context, ID).join("auth-incidents.json")).unwrap(),
        )
        .unwrap();
        let schedule = &store["incidents"][0]["auto_settle"];
        assert_eq!(schedule["state"], "exhausted", "{outcome}: {store}");
        assert_eq!(schedule["attempts"].as_array().unwrap().len(), 3);
        if outcome == "late" {
            assert_eq!(
                schedule["attempts"][2]["result"],
                "runtime_adoption_required"
            );
        }
        assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
        assert!(!safe_request(&context, &record, "2"));
        assert_eq!(log(tmp.path()).matches("new-session").count(), 1);
        let calls = fs::read_to_string(tmp.path().join("broker.log")).unwrap();
        tick(&context, &record, &tmux, start + 900);
        crate::auth_incident::auto_settle::tick(&context, &record, start + 900).unwrap();
        assert_eq!(
            fs::read_to_string(tmp.path().join("broker.log")).unwrap(),
            calls
        );
        crate::auth_incident::tick(&context, &record, &tmux, "fixture", Some(false)).unwrap();
        let alerts = crate::auth_incident::alert_delivery::list(&context, 100).unwrap();
        let alert = alerts["alerts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|alert| alert["result"] == "recheck_exhausted")
            .unwrap();
        assert_eq!(
            alert["recovery_action"],
            if outcome == "late" {
                "restart_same_account"
            } else {
                "same_account_credential_recheck"
            }
        );
    }
}
