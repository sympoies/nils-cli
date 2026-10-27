//! A same-account switch is the supported way to settle an authentication
//! quarantine once the account credentials are valid again.
use super::*;
use pretty_assertions::assert_eq;

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
fn auth_recheck_settles_the_quarantine_once_credentials_are_republished() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = quarantined(&context, tmp.path(), &tmux);
    publish(tmp.path(), 2);
    let before = log(tmp.path());
    recheck(&context, &record, &tmux).unwrap();
    let store = incident_store(&context);
    assert_eq!(store["incidents"][0]["status"], "recovered", "{store}");
    assert_eq!(
        store["incidents"][0]["recovery_result"],
        "credentials_refreshed"
    );
    assert_eq!(
        store["incidents"][0]["failover"]["state"],
        "credentials_refreshed"
    );
    let current = load_session_record(&context, ID).unwrap();
    assert_eq!(launch_id(&current), launch_id(&record));
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &current).is_ok());
    crate::send::send_to_session(
        &context,
        crate::send::unguarded_args(
            ID.into(),
            Some("synthetic prompt".into()),
            vec![SpecialKey::Enter],
            true,
            tmux.to_path_buf(),
        ),
    )
    .unwrap();
    let effects = &log(tmp.path())[before.len()..];
    assert!(!effects.contains("kill-session"));
    assert!(!effects.contains("new-session"));
    assert!(effects.contains("paste-buffer"));
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
    recheck(&context, &record, &tmux).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let launch = launch_id(&record).unwrap();
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
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &current).is_err());
}
