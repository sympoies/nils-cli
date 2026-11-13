//! Fresh guarded input after replacement models terminal submission,
//! not a successful provider response or replay of the failed request.
use super::*;
use pretty_assertions::assert_eq;

pub(super) fn assert_submitted(
    context: &CliContext,
    root: &Path,
    tmux: &Path,
    record: &SessionRecord,
) {
    let launch = launch_id(record).unwrap();
    assert!(crate::auth_incident::view(context, record).is_none());
    fs::write(root.join("echo-paste"), "").unwrap();
    // A graceful restart may have pasted /exit. Count only this new request.
    fs::write(root.join("pasted-input"), "").unwrap();
    let before = log(root);
    let text = "synthetic fresh guarded prompt";
    let mut args = crate::send::unguarded_args(
        ID.into(),
        Some(text.into()),
        vec![SpecialKey::Enter],
        false,
        tmux.to_path_buf(),
    );
    args.guards = crate::send_guard::GuardArgs {
        expect_runtime: Some(launch.clone()),
        expect_provider_session: Some(CONVERSATION.into()),
        require_idle: true,
        require_empty_composer: true,
        idempotency_key: Some(format!("fresh-request-{launch}")),
        ..Default::default()
    };
    let result = crate::send::send_to_session(context, args).unwrap();
    assert!(result.sent_text);
    assert!(result.guards.is_some());
    let submission = result.submission.unwrap();
    assert_eq!(
        submission.outcome,
        crate::send_submit::SubmitOutcome::Submitted
    );
    assert_eq!(submission.enter_presses, 1);
    assert_eq!(fs::read_to_string(root.join("pasted-input")).unwrap(), text);
    let effects = log(root);
    assert_eq!(effects[before.len()..].matches("paste-buffer").count(), 1);
    assert!(
        root.join("running").exists(),
        "submission did not simulate provider exit"
    );
    assert_eq!(
        launch_id(&load_session_record(context, ID).unwrap()),
        Some(launch)
    );
}

fn assert_alert(context: &CliContext, incident: &Value, phase: &str, result: &str) {
    let alerts = crate::auth_incident::alert_delivery::list(context, 100).unwrap();
    assert!(
        alerts["alerts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|alert| alert["incident_id"] == incident["incident_id"]
                && alert["phase"] == phase
                && alert["result"] == result),
        "missing {phase}/{result}: {alerts}"
    );
}

pub(super) fn assert_retained_after_tick(
    context: &CliContext,
    tmux: &Path,
    record: &SessionRecord,
) {
    let path = crate::session_dir(context, ID).join("auth-incidents.json");
    let before: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let incident = &before["incidents"][0];
    // Admission and a completed guarded send precede daemon reconciliation.
    assert_eq!(incident["status"], "auth_failed");
    assert!(crate::auth_incident::view(context, record).is_none());
    assert_alert(context, incident, "incident", "authentication");
    assert_alert(context, incident, "recovery", "blocked_no_credential");
    crate::auth_incident::tick(context, record, tmux, "fixture", Some(false)).unwrap();
    let after: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(after["incidents"].as_array().unwrap().len(), 1);
    assert_eq!(
        after["incidents"][0]["incident_id"],
        incident["incident_id"]
    );
    assert_eq!(after["incidents"][0]["status"], "runtime_replaced");
    assert!(crate::auth_incident::view(context, record).is_none());
    for (phase, result) in [
        ("incident", "authentication"),
        ("recovery", "blocked_no_credential"),
        ("recovery", "runtime_replaced"),
    ] {
        assert_alert(context, incident, phase, result);
    }
}
