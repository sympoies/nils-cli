use super::*;
use pretty_assertions::{assert_eq, assert_ne};

#[test]
fn claude_switch_preserves_unverified_replacement_cleanup_failure() {
    use crate::coordination::broker::test_support;
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let _broker = claude_broker_fixture(&lock, tmp.path());
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    seed_bound_claude_session(tmp.path(), "uncertain-switch", &cwd, "alpha");
    let mut record = load_session_record(&context, "uncertain-switch").unwrap();
    crate::claude_account::queue_next(&mut record, "beta").unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let old = record.runtime.as_ref().unwrap().launch_id.clone();
    // An explicit unknown broker identity refuses provisioning, while the
    // fake tmux kill does not stop the new pane's real process boundary.
    test_support::seed_live_broker(&context, &record.id, &old, json!({}));
    let heartbeat = crate::coordination::heartbeat_path(&context.state_dir, &record.id);
    fs::remove_file(heartbeat).unwrap();
    let tmux = resume_tmux(tmp.path(), &tmp.path().join("tmux.log"));
    let error =
        crate::session_account::resume_after_switch_stop(&context, &record.id, &tmux).unwrap_err();
    assert_eq!(error.code(), "session-termination-failed");
    assert_ne!(
        error.details().unwrap().get("session_state"),
        Some(&json!("stopped"))
    );
    let retained = load_session_record(&context, &record.id).unwrap();
    let view =
        serde_json::to_value(crate::session_view(&context, &retained, None, Some(&tmux))).unwrap();
    assert_eq!(view["claude_account"]["next"]["state"], "failed");
    assert_eq!(
        view["claude_account"]["next"]["failure_reason"],
        "session-termination-failed"
    );
    assert_eq!(view["startup"]["state"], "failed");
    assert_eq!(view["startup"]["retry_safe"], false);
    assert!(retained.extra["startup"].get("cleanup").is_some());
}

fn resume_after_bounded_helper_exit(stale_heartbeat: bool) {
    use crate::coordination::broker::test_support;
    use pretty_assertions::{assert_eq, assert_ne};
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    seed_bound_claude_session(tmp.path(), "helper-exited", &cwd, "alpha");
    let mut record = load_session_record(&context, "helper-exited").unwrap();
    let old = record.runtime.as_ref().unwrap().launch_id.clone();
    let conversation = serde_json::to_value(record.provider_resume.as_ref().unwrap()).unwrap();
    let mut identity =
        test_support::process_group_identity(test_support::verified_absent_process_group());
    identity["launch_id"] = json!(old);
    #[cfg(target_os = "macos")]
    {
        identity["macos_boot_id"] = json!(crate::capture_macos_boot_id().unwrap());
    }
    record.extra.remove("tmux_runtime_never_launched");
    record
        .extra
        .insert("delete_tmux_identity".into(), identity.clone());
    crate::claude_account::queue_next(&mut record, "beta").unwrap();
    // Teardown killed the pane, then failed while a bounded helper was
    // still live. Its later exit does not revoke the old ready broker.
    crate::set_tmux_termination_state(
        &mut record,
        crate::TmuxTerminationState::KillConfirmed {
            thaw_on_recovery: false,
        },
    )
    .unwrap();
    crate::write_session_record(&context, &record).unwrap();
    test_support::seed_live_broker(&context, &record.id, &old, identity);
    if stale_heartbeat {
        let mut locked = crate::coordination::lock_registry(&context).unwrap();
        let mut registry = serde_json::to_value(&locked.registry).unwrap();
        registry["brokers"][record.id.as_str()]["heartbeat_epoch"] = json!(0);
        locked.registry = serde_json::from_value(registry).unwrap();
        locked.save().unwrap();
        drop(locked);
        fs::remove_file(crate::coordination::heartbeat_path(
            &context.state_dir,
            &record.id,
        ))
        .unwrap();
    }
    let tmux = resume_tmux(tmp.path(), &tmp.path().join("tmux.log"));
    // Preserve the old heartbeat until replacement provisioning, as the
    // real held launch does. Do not repair the old broker in the fixture.
    let script = fs::read_to_string(&tmux).unwrap().replace(
        r#"if [ -n "$heartbeat" ] && [ -n "$incarnation" ]; then"#,
        r#"if [ -n "$heartbeat" ] && [ -n "$incarnation" ]; then gate="$(dirname "$heartbeat")/broker-provisioned"; (i=0; while [ ! -f "$gate" ] && [ "$i" -lt 1000 ]; do i=$((i + 1)); sleep 0.01; done; printf '%s:%s\n' "$incarnation" "$(date +%s)" > "$heartbeat"; chmod 600 "$heartbeat") >/dev/null 2>&1 & fi; if false; then"#,
    );
    fs::write(&tmux, script).unwrap();

    crate::resume_session_locked(&context, record, &tmux)
        .expect("resume must retire the switch's stopped runtime after the bounded helper exits");
    let resumed = load_session_record(&context, "helper-exited").unwrap();
    assert_eq!(
        serde_json::to_value(resumed.provider_resume.as_ref().unwrap()).unwrap(),
        conversation,
    );
    assert_ne!(resumed.runtime.as_ref().unwrap().launch_id, old);
    assert_eq!(resumed.runtime.as_ref().unwrap().generation, 2);
    let view = serde_json::to_value(crate::claude_account::view_for_record(&resumed)).unwrap();
    assert_eq!(view["selected_account"], "beta");
    assert!(view.get("next").is_none());
    assert!(!resumed.extra.contains_key("delete_tmux_termination_state"));
    assert!(
        fs::read_to_string(tmp.path().join("tmux.log"))
            .unwrap()
            .contains(&format!(
                "CLAUDE_CONFIG_DIR={}",
                broker.config_dir("beta").display(),
            ))
    );
}

#[test]
fn claude_switch_retry_after_helper_exit_retires_the_ready_broker() {
    resume_after_bounded_helper_exit(false);
}

#[cfg(target_os = "macos")]
#[test]
fn claude_switch_retry_after_helper_exit_proves_stopped_with_a_stale_heartbeat() {
    resume_after_bounded_helper_exit(true);
}

#[tokio::test]
async fn queued_claude_switch_applies_after_provider_hook_idle() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, broker, record, _probe) = idle_account_fixture(&lock, tmp.path());
    let tmux = st.tmux_bin.clone();
    let log = tmp.path().join("tmux.log");
    let runtime = &record.runtime.as_ref().unwrap().launch_id;
    let event = |id: &str, kind: &str, candidate: bool| {
        serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": id, "runtime_id": runtime, "provider": "claude",
            "provider_session_id": "resume-session-id", "provider_turn_id": "turn-switch",
            "kind": kind, "source_kind": "provider_hook", "confidence": "observed",
            "completion_candidate": candidate
        }))
        .unwrap()
    };
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        event("start", "turn_started", false),
    )
    .unwrap();
    let queued =
        crate::session_account::switch(&st.context, &record.id, "beta", None, &tmux).unwrap();
    let queued = serde_json::to_value(queued).unwrap();
    assert_eq!(queued["claude_account"]["next"]["state"], "queued");
    let revision = queued["claude_account"]["next"]["revision"].clone();
    assert!(
        !broker
            .calls()
            .contains("materialize --provider claude --account beta")
    );
    assert!(!crate::auto_resume::view_for_record(&st.context, &record).enabled);
    // No auto-resume scheduler is enabled or started by this fixture.
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        event("stop", "stop_observed", true),
    )
    .unwrap();
    let task = tokio::spawn(account_drain::run(st.clone()));
    let applied = tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            let current = load_session_record(&st.context, &record.id).unwrap();
            let view =
                serde_json::to_value(crate::claude_account::view_for_record(&current)).unwrap();
            if view["selected_account"] == "beta"
                && fs::read_to_string(&log)
                    .unwrap_or_default()
                    .contains("new-session")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    task.abort();
    let _ = task.await;
    applied.expect("queued Claude switch must drain within six seconds of Stop");
    // Binding is persisted before launch. Wait for the blocking worker's
    // record lock before asserting completion or dropping its fixture files.
    let context = st.context.clone();
    let id = record.id.clone();
    let resumed = tokio::task::spawn_blocking(move || {
        let _lock = crate::acquire_session_record_lock(&context, &id).unwrap();
        load_session_record(&context, &id).unwrap()
    })
    .await
    .unwrap();
    let view = serde_json::to_value(crate::claude_account::view_for_record(&resumed)).unwrap();
    assert_eq!(view["revision"], revision);
    assert!(view.get("next").is_none());
    assert_eq!(
        serde_json::to_value(resumed.provider_resume).unwrap(),
        serde_json::to_value(record.provider_resume).unwrap()
    );
    assert_ne!(resumed.runtime.as_ref().unwrap().launch_id, *runtime);
    let launch = fs::read_to_string(log).unwrap();
    assert!(launch.contains("new-session"));
    assert!(launch.contains(&format!(
        "CLAUDE_CONFIG_DIR={}",
        broker.config_dir("beta").display()
    )));
}

fn idle_account_fixture(
    lock: &GlobalStateLock,
    root: &Path,
) -> (
    Arc<ServeState>,
    ClaudeBrokerFixture,
    SessionRecord,
    crate::tmux_probe_fixture::Guard,
) {
    let cwd = root.join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let broker = claude_broker_fixture(lock, root);
    seed_bound_claude_session(root, "idle-switch", &cwd, "alpha");
    let log = root.join("tmux.log");
    let tmux = resume_tmux(root, &log);
    // The runtime is present at the idle check, then proven absent at stop.
    let script = fs::read_to_string(&tmux).unwrap().replace(
        "has-session) printf '%s\\n' \"can't find session: fixture\" >&2; exit 1",
        &format!("has-session) if [ ! -f {marker} ]; then touch {marker}; exit 0; fi; printf '%s\\n' \"can't find session: fixture\" >&2; exit 1", marker = shell_words::quote(&root.join("idle-probe").to_string_lossy())),
    );
    fs::write(&tmux, script).unwrap();
    let _probe =
        crate::tmux_probe_fixture::install(&tmux, crate::tmux_probe_fixture::Probe::Stopped);
    let st = state(root, Some(TOKEN), tmux.clone());
    let record = load_session_record(&st.context, "idle-switch").unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    (st, broker, record, _probe)
}

fn idle_account_event(record: &SessionRecord, id: &str, kind: &str) -> crate::activity::TurnEvent {
    serde_json::from_value(json!({
        "schema_version": crate::activity::TURN_EVENT_VERSION,
        "event_id": id, "runtime_id": record.runtime.as_ref().unwrap().launch_id,
        "provider": "claude", "provider_session_id": "resume-session-id",
        "provider_turn_id": "fixture-turn", "kind": kind, "confidence": "authoritative"
    }))
    .unwrap()
}

#[tokio::test]
async fn claude_idle_switch_reports_blockers_and_reissue_applies_immediately() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, broker, record, _probe) = idle_account_fixture(&lock, tmp.path());
    let tmux = &st.tmux_bin;
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        idle_account_event(&record, "start", "turn_started"),
    )
    .unwrap();
    let queued =
        crate::session_account::switch(&st.context, &record.id, "gamma", None, tmux).unwrap();
    let queued = serde_json::to_value(queued).unwrap();
    assert_eq!(
        queued["claude_account"]["next"]["blocking_reason"],
        "turn_busy"
    );
    let revision = queued["claude_account"]["next"]["revision"].clone();
    crate::session_account::claude_idle::apply_queued(&st.context, &record.id, tmux).unwrap();
    assert!(
        !broker
            .calls()
            .contains("materialize --provider claude --account gamma")
    );
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        idle_account_event(&record, "done", "turn_completed"),
    )
    .unwrap();
    let error = crate::session_account::claude_idle::apply_queued(&st.context, &record.id, tmux)
        .unwrap_err();
    assert_eq!(error.code(), "claude-account-switch-refused");
    let current = load_session_record(&st.context, &record.id).unwrap();
    let blocked = serde_json::to_value(crate::claude_account::view_for_record(&current)).unwrap();
    assert_eq!(blocked["selected_account"], "alpha");
    assert_eq!(blocked["next"]["state"], "queued");
    assert_eq!(blocked["next"]["revision"], revision);
    assert_eq!(
        blocked["next"]["blocking_reason"],
        "claude-account-broker-rejected"
    );
    let calls = fs::read_to_string(tmp.path().join("tmux.log")).unwrap();
    assert!(!calls.contains("new-session") && !calls.contains("kill-session"));
    // Reset the one-shot running probe for the next idle request. No daemon
    // tick is needed: an external re-issue must switch synchronously.
    fs::remove_file(tmp.path().join("idle-probe")).unwrap();
    let switched =
        crate::session_account::switch(&st.context, &record.id, "beta", None, tmux).unwrap();
    let switched = serde_json::to_value(switched).unwrap();
    assert_eq!(switched["claude_account"]["selected_account"], "beta");
    assert!(switched["claude_account"].get("next").is_none());
}

#[tokio::test]
async fn claude_idle_switch_does_not_apply_a_cancelled_intent() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, broker, record, _probe) = idle_account_fixture(&lock, tmp.path());
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        idle_account_event(&record, "start", "turn_started"),
    )
    .unwrap();
    for account in ["beta", "alpha"] {
        crate::session_account::switch(&st.context, &record.id, account, None, &st.tmux_bin)
            .unwrap();
    }
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        idle_account_event(&record, "done", "turn_completed"),
    )
    .unwrap();
    crate::session_account::claude_idle::apply_queued(&st.context, &record.id, &st.tmux_bin)
        .unwrap();
    let current = load_session_record(&st.context, &record.id).unwrap();
    let view = serde_json::to_value(crate::claude_account::view_for_record(&current)).unwrap();
    assert_eq!(view["selected_account"], "alpha");
    assert!(view.get("next").is_none());
    assert!(!broker.calls().contains("materialize"));
}

#[tokio::test]
async fn claude_idle_switch_keeps_stopped_and_unknown_turns_queued() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, broker, record, _probe) = idle_account_fixture(&lock, tmp.path());
    fs::remove_file(crate::session_dir(&st.context, &record.id).join("activity.json")).unwrap();
    let queued =
        crate::session_account::switch(&st.context, &record.id, "beta", None, &st.tmux_bin)
            .unwrap();
    let queued = serde_json::to_value(queued).unwrap();
    assert_eq!(
        queued["claude_account"]["next"]["blocking_reason"],
        "turn_state_unavailable"
    );
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    for (id, kind) in [("start", "turn_started"), ("done", "turn_completed")] {
        crate::activity::ingest_event(
            &st.context,
            &record.id,
            idle_account_event(&record, id, kind),
        )
        .unwrap();
    }
    // Consume the one-shot live probe: this session is stopped at the tick.
    fs::write(tmp.path().join("idle-probe"), "").unwrap();
    crate::session_account::claude_idle::apply_queued(&st.context, &record.id, &st.tmux_bin)
        .unwrap();
    let current = load_session_record(&st.context, &record.id).unwrap();
    let view = serde_json::to_value(crate::claude_account::view_for_record(&current)).unwrap();
    assert_eq!(view["selected_account"], "alpha");
    assert_eq!(view["next"]["state"], "queued");
    assert_eq!(view["next"]["blocking_reason"], "session_stopped");
    assert!(!broker.calls().contains("materialize"));
}
