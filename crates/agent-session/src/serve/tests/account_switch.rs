use super::*;

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
        locked
            .registry
            .brokers
            .get_mut(&record.id)
            .unwrap()
            .heartbeat_epoch = 0;
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
