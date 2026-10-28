use super::*;
use pretty_assertions::{assert_eq, assert_ne};

pub(super) fn verified_broker(root: &Path, broker: &Path) -> String {
    for account in ["alpha", "beta"] {
        let mut value = crate::claude_account::generation::tests::envelope(1);
        value["account"] = json!(account);
        fs::write(
            root.join(format!("{account}-publication.json")),
            value.to_string(),
        )
        .unwrap();
    }
    let wrapper = executable(
        &root.join("verified-broker"),
        &format!(
            r#"#!/bin/sh
case "$1" in
list) {broker} "$@" | sed 's/"accounts":/"capabilities":["credential_state_v1"],"accounts":/' ;;
credential-state) cat {root}/"$5-publication.json" ;;
materialize) if [ -f {root}/busy-snapshot.json ]; then cp {root}/busy-snapshot.json {root}/sessions/idle-switch/activity.json; fi; exec {broker} "$@" ;;
*) exec {broker} "$@" ;;
esac
"#,
            broker = shell_words::quote(&broker.to_string_lossy()),
            root = shell_words::quote(&root.to_string_lossy())
        ),
    );
    json!([wrapper]).to_string()
}

#[test]
fn failed_claude_switch_then_new_resume_explicitly_abandons_pending_account() {
    use clap::Parser;
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    let context = CliContext {
        state_dir: tmp.path().into(),
        host: None,
    };
    seed_bound_claude_session(tmp.path(), "switch-source", tmp.path(), "alpha");
    let conversation = "00000000-0000-4000-8000-000000000007";
    let mut record = load_session_record(&context, "switch-source").unwrap();
    let provider = record.provider_resume.as_mut().unwrap();
    provider.session_id = conversation.into();
    provider.resume_args = vec!["--resume".into(), conversation.into()];
    crate::claude_account::bind_initial(&mut record, "alpha", "explicit").unwrap();
    crate::claude_account::queue_next(&mut record, "beta").unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let history = broker.config_dir("alpha").join("projects/project");
    fs::create_dir_all(&history).unwrap();
    fs::write(
        history.join(format!("{conversation}.jsonl")),
        format!(
            "{}\n",
            json!({"sessionId":conversation,"cwd":tmp.path(),"type":"user"})
        ),
    )
    .unwrap();
    let failing_tmux = executable(
        &tmp.path().join("failing-tmux"),
        "#!/bin/sh\ncase \"$1\" in has-session) echo \"can't find session: fixture\" >&2; exit 1;; new-session) exit 1;; *) exit 0;; esac\n",
    );
    let source_lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
    let error =
        crate::session_account::resume_after_switch_stop(&context, &record.id, &failing_tmux)
            .unwrap_err();
    drop(source_lock);
    assert_eq!(error.code(), "claude-account-switch-resume-failed");
    let failed = load_session_record(&context, &record.id).unwrap();
    assert_eq!(failed.extra["claude_account_next"]["state"], "failed");
    let tmux = startup_fixtures::live_resume_tmux(tmp.path(), &tmp.path().join("retry.log"));
    let args = crate::cli::Cli::parse_from([
        "agent-session",
        "start",
        "--agent",
        "claude",
        "--resume",
        conversation,
        "--account",
        "alpha",
        "--id",
        "replacement",
        "--no-parent",
        "--tmux-bin",
        tmux.to_str().unwrap(),
    ]);
    let crate::cli::Command::Start(args) = args.command else {
        unreachable!()
    };
    assert_eq!(crate::start_resume::run_start(&context, *args), 0);
    let source = load_session_record(&context, &record.id).unwrap();
    assert!(!source.extra.contains_key("claude_account_next"));
    let replacement = load_session_record(&context, "replacement").unwrap();
    assert_eq!(
        replacement.provider_resume.as_ref().unwrap().session_id,
        conversation
    );
    assert_eq!(
        crate::claude_account::view_for_record(&replacement)
            .unwrap()
            .selected_account
            .as_deref(),
        Some("alpha")
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("retry.log"))
            .unwrap()
            .matches("new-session")
            .count(),
        1
    );
    let selective_tmux = executable(
        &tmp.path().join("selective-tmux"),
        "#!/bin/sh\ncase \"$*\" in *'=hs-claude-switch-source'*) echo \"can't find session: fixture\" >&2; exit 1;; *) exit 0;; esac\n",
    );
    assert_eq!(
        crate::resume_session_by_id(&context, &record.id, &selective_tmux)
            .unwrap_err()
            .code(),
        "resume-source-live"
    );
}

#[test]
fn claude_switch_refuses_unverified_credentials_before_stopping() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, _broker, record, _probe) = idle_account_fixture(&lock, tmp.path());
    let mut publication = crate::claude_account::generation::tests::envelope(1);
    publication["account"] = json!("beta");
    publication["state"]["publication"]["stores"][0]["status"] = json!("pending");
    fs::write(
        tmp.path().join("beta-publication.json"),
        publication.to_string(),
    )
    .unwrap();
    for (id, kind) in [("start", "turn_started"), ("done", "turn_completed")] {
        crate::activity::ingest_event(
            &st.context,
            &record.id,
            idle_account_event(&record, id, kind),
        )
        .unwrap();
    }
    let before =
        fs::read(crate::session_dir(&st.context, &record.id).join("session.json")).unwrap();
    let error = crate::session_account::switch(&st.context, &record.id, "beta", None, &st.tmux_bin)
        .unwrap_err();
    assert_eq!(error.code(), "claude-account-switch-refused");
    assert_eq!(
        fs::read(crate::session_dir(&st.context, &record.id).join("session.json")).unwrap(),
        before
    );
    let log = fs::read_to_string(tmp.path().join("tmux.log")).unwrap_or_default();
    assert!(
        !log.contains("new-session") && !log.contains("if-shell") && !log.contains("kill-session")
    );
}

#[test]
fn claude_switch_rechecks_busy_turn_after_credential_preflight() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, _broker, record, _probe) = idle_account_fixture(&lock, tmp.path());
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        idle_account_event(&record, "start", "turn_started"),
    )
    .unwrap();
    fs::copy(
        crate::session_dir(&st.context, &record.id).join("activity.json"),
        tmp.path().join("busy-snapshot.json"),
    )
    .unwrap();
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        idle_account_event(&record, "done", "turn_completed"),
    )
    .unwrap();
    let result =
        crate::session_account::switch(&st.context, &record.id, "beta", None, &st.tmux_bin)
            .unwrap();
    let view = serde_json::to_value(result).unwrap();
    assert_eq!(
        view["claude_account"]["next"]["blocking_reason"],
        "turn_busy"
    );
    assert_eq!(view["claude_account"]["selected_account"], "alpha");
    let log = fs::read_to_string(tmp.path().join("tmux.log")).unwrap_or_default();
    assert!(
        !log.contains("new-session") && !log.contains("if-shell") && !log.contains("kill-session")
    );
}

#[test]
fn failed_claude_switch_retry_requires_current_credential_preflight() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = claude_broker_fixture(&lock, tmp.path());
    let context = CliContext {
        state_dir: tmp.path().into(),
        host: None,
    };
    seed_bound_claude_session(tmp.path(), "retry-source", tmp.path(), "alpha");
    let mut record = load_session_record(&context, "retry-source").unwrap();
    crate::claude_account::queue_next(&mut record, "beta").unwrap();
    let stopped = record.clone();
    crate::claude_account::mark_next_resume_failed(&mut record, &stopped, "launch-failed").unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let path = crate::session_dir(&context, &record.id).join("session.json");
    let before = fs::read(&path).unwrap();
    let mut publication = crate::claude_account::generation::tests::envelope(1);
    publication["account"] = json!("beta");
    publication["state"]["publication"]["status"] = json!("blocked");
    fs::write(
        tmp.path().join("beta-publication.json"),
        publication.to_string(),
    )
    .unwrap();
    let tmux = resume_tmux(tmp.path(), &tmp.path().join("retry.log"));
    let error = crate::resume_session_by_id(&context, &record.id, &tmux).unwrap_err();
    assert_eq!(error.code(), "claude-account-switch-refused");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(
        !fs::read_to_string(tmp.path().join("retry.log"))
            .unwrap_or_default()
            .contains("new-session")
    );
    publication["state"]["publication"]["status"] = json!("published");
    fs::write(
        tmp.path().join("beta-publication.json"),
        publication.to_string(),
    )
    .unwrap();
    crate::resume_session_by_id(&context, &record.id, &tmux).unwrap();
    let resumed = load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        crate::claude_account::view_for_record(&resumed)
            .unwrap()
            .selected_account
            .as_deref(),
        Some("beta")
    );
    assert!(!resumed.extra.contains_key("claude_account_next"));
    assert_eq!(
        fs::read_to_string(tmp.path().join("retry.log"))
            .unwrap()
            .matches("new-session")
            .count(),
        1
    );
}

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

#[tokio::test]
async fn claude_idle_switch_keeps_starting_and_needs_input_queued() {
    let lock = GlobalStateLock::new();
    for needs_input in [false, true] {
        let tmp = tempfile::TempDir::new().unwrap();
        let (st, broker, record, _probe) = idle_account_fixture(&lock, tmp.path());
        let (phase, reason) = if needs_input {
            crate::activity::ingest_event(
                &st.context,
                &record.id,
                idle_account_event(&record, "start", "turn_started"),
            )
            .unwrap();
            let mut attention = idle_account_event(&record, "approval", "attention_requested");
            attention.attention_id = Some("fixture-approval".into());
            attention.attention_kind = Some("approval".into());
            attention.source_kind = crate::activity::SourceKind::ProviderHook;
            attention.confidence = crate::activity::Confidence::Observed;
            crate::activity::ingest_event(&st.context, &record.id, attention).unwrap();
            (crate::activity::TurnPhase::NeedsInput, "turn_needs_input")
        } else {
            (crate::activity::TurnPhase::Starting, "turn_starting")
        };
        assert_eq!(
            crate::activity::state_for_view(&st.context, &record)
                .unwrap()
                .phase,
            phase
        );
        crate::session_account::switch(&st.context, &record.id, "beta", None, &st.tmux_bin)
            .unwrap();
        crate::session_account::claude_idle::apply_queued(&st.context, &record.id, &st.tmux_bin)
            .unwrap();
        let current = load_session_record(&st.context, &record.id).unwrap();
        let view = serde_json::to_value(crate::claude_account::view_for_record(&current)).unwrap();
        assert_eq!(view["selected_account"], "alpha");
        assert_eq!(view["next"]["state"], "queued");
        assert_eq!(view["next"]["blocking_reason"], reason);
        assert!(!broker.calls().contains("materialize"));
        let log = fs::read_to_string(tmp.path().join("tmux.log")).unwrap_or_default();
        for command in ["if-shell", "kill-session", "new-session"] {
            assert!(!log.contains(command), "{reason}: {log}");
        }
    }
}

#[tokio::test]
async fn claude_idle_switch_backs_off_persistent_preflight_refusal() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, broker, record, _probe) = idle_account_fixture(&lock, tmp.path());
    // Every tick sees the same live idle runtime; refusal prevents any stop.
    let tmux = logging_tmux(tmp.path(), &tmp.path().join("tmux.log"));
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        idle_account_event(&record, "start", "turn_started"),
    )
    .unwrap();
    crate::session_account::switch(&st.context, &record.id, "gamma", None, &tmux).unwrap();
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        idle_account_event(&record, "done", "turn_completed"),
    )
    .unwrap();
    let mut warnings = 0;
    for _ in 0..3 {
        warnings += usize::from(
            crate::session_account::claude_idle::apply_queued(&st.context, &record.id, &tmux)
                .is_err(),
        );
    }
    assert_eq!(
        broker
            .calls()
            .matches("materialize --provider claude --account gamma")
            .count(),
        1,
        "persistent refusal must not invoke the broker on every tick"
    );
    assert_eq!(warnings, 1, "only a changed refusal reason should warn");
    let current = load_session_record(&st.context, &record.id).unwrap();
    let view = serde_json::to_value(crate::claude_account::view_for_record(&current)).unwrap();
    assert_eq!(view["selected_account"], "alpha");
    assert_eq!(view["next"]["state"], "queued");
    assert_eq!(
        view["next"]["blocking_reason"],
        "claude-account-broker-rejected"
    );
}

#[tokio::test]
async fn claude_idle_switch_retry_window_bounds_broker_and_stop_attempts() {
    let lock = GlobalStateLock::new();
    for account in ["gamma", "beta"] {
        let tmp = tempfile::TempDir::new().unwrap();
        let (st, broker, record, probe) = idle_account_fixture(&lock, tmp.path());
        // Without the stopped probe, the fake live pane lacks identity
        // evidence, so beta's verified stop is refused before any signal.
        drop(probe);
        let tmux = logging_tmux(tmp.path(), &tmp.path().join("tmux.log"));
        crate::activity::ingest_event(
            &st.context,
            &record.id,
            idle_account_event(&record, "start", "turn_started"),
        )
        .unwrap();
        crate::session_account::switch(&st.context, &record.id, account, None, &tmux).unwrap();
        crate::activity::ingest_event(
            &st.context,
            &record.id,
            idle_account_event(&record, "done", "turn_completed"),
        )
        .unwrap();
        let base: jiff::Timestamp = "2030-01-01T00:00:00Z".parse().unwrap();
        let mut warnings = 0;
        for seconds in [0, 2, 4, 29, 30, 31] {
            let now = base
                .checked_add(jiff::SignedDuration::from_secs(seconds))
                .unwrap();
            warnings += usize::from(
                crate::session_account::claude_idle::apply_queued_at(
                    &st.context,
                    &record.id,
                    &tmux,
                    now,
                )
                .is_err(),
            );
        }
        assert_eq!(
            broker
                .calls()
                .matches(&format!(
                    "materialize --provider claude --account {account}"
                ))
                .count(),
            2
        );
        assert_eq!(
            warnings, 1,
            "an unchanged refusal must not warn again after retry"
        );
        let current = load_session_record(&st.context, &record.id).unwrap();
        let view = serde_json::to_value(crate::claude_account::view_for_record(&current)).unwrap();
        assert_eq!(view["selected_account"], "alpha");
        assert_eq!(view["next"]["state"], "queued");
        assert_eq!(
            view["next"]["blocking_reason"],
            if account == "gamma" {
                "claude-account-broker-rejected"
            } else {
                "session-termination-failed"
            }
        );
        let log = fs::read_to_string(tmp.path().join("tmux.log")).unwrap();
        assert!(
            !log.contains("kill-session")
                && !log.contains("if-shell")
                && !log.contains("new-session")
        );
    }
}

#[tokio::test]
async fn claude_idle_switch_backoff_does_not_fence_new_intent_or_runtime() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (st, broker, record, _probe) = idle_account_fixture(&lock, tmp.path());
    let tmux = logging_tmux(tmp.path(), &tmp.path().join("tmux.log"));
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        idle_account_event(&record, "start", "turn_started"),
    )
    .unwrap();
    crate::session_account::switch(&st.context, &record.id, "gamma", None, &tmux).unwrap();
    crate::activity::ingest_event(
        &st.context,
        &record.id,
        idle_account_event(&record, "done", "turn_completed"),
    )
    .unwrap();
    let now: jiff::Timestamp = "2030-01-01T00:00:00Z".parse().unwrap();
    assert!(
        crate::session_account::claude_idle::apply_queued_at(&st.context, &record.id, &tmux, now)
            .is_err()
    );
    let mut current = load_session_record(&st.context, &record.id).unwrap();
    let original_revision = current.extra["claude_account_next"]["revision"].clone();
    crate::claude_account::queue_next(&mut current, "gamma").unwrap();
    crate::write_session_record(&st.context, &current).unwrap();
    assert!(
        crate::session_account::claude_idle::apply_queued_at(&st.context, &record.id, &tmux, now)
            .is_err()
    );
    assert_ne!(
        current.extra["claude_account_next"]["revision"],
        original_revision
    );
    let mut current = load_session_record(&st.context, &record.id).unwrap();
    let runtime = current.runtime.as_mut().unwrap();
    runtime.launch_id = "replacement-runtime".into();
    runtime.generation += 1;
    crate::write_session_record(&st.context, &current).unwrap();
    crate::activity::activate_runtime(&st.context, &current).unwrap();
    for (id, kind) in [
        ("replacement-start", "turn_started"),
        ("replacement-done", "turn_completed"),
    ] {
        crate::activity::ingest_event(
            &st.context,
            &current.id,
            idle_account_event(&current, id, kind),
        )
        .unwrap();
    }
    crate::session_account::claude_idle::apply_queued_at(&st.context, &record.id, &tmux, now)
        .unwrap();
    assert_eq!(
        broker
            .calls()
            .matches("materialize --provider claude --account gamma")
            .count(),
        3,
        "new revision and replacement runtime must retry immediately"
    );
    let mut current = load_session_record(&st.context, &record.id).unwrap();
    crate::claude_account::queue_next(&mut current, "alpha").unwrap();
    crate::write_session_record(&st.context, &current).unwrap();
    crate::session_account::claude_idle::apply_queued_at(&st.context, &record.id, &tmux, now)
        .unwrap();
    assert_eq!(
        broker
            .calls()
            .matches("materialize --provider claude --account gamma")
            .count(),
        3,
        "cancellation must win during backoff"
    );
}
