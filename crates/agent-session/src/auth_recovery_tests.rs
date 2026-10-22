use super::*;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;

fn failed_runtime(context: &CliContext, root: &Path, tmux: &Path, bound: bool) -> SessionRecord {
    seed_exited_session(context, root);
    let launch = running_session(context, tmux);
    session_ready(context, &launch);
    let mut record = load_session_record(context, ID).unwrap();
    if bound {
        crate::claude_account::bind_initial(&mut record, "alpha", "explicit").unwrap();
        crate::write_session_record(context, &record).unwrap();
    }
    crate::auth_incident::observe(
        context,
        &record,
        "synthetic-auth-failure",
        crate::auth_incident::AuthSource::ClaudeStopFailure,
        crate::activity::Confidence::Authoritative,
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    record
}

fn broker(root: &Path, lock: &GlobalStateLock) -> EnvGuard {
    let dir = root.join("alpha");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(dir.join(".credentials.json"), "{}").unwrap();
    fs::write(root.join("inventory.json"), serde_json::to_vec(&json!({"schema_version":"agent-session.account-broker.v2","provider":"claude","capabilities":["credential_state_v1"]})).unwrap()).unwrap();
    publish(root, 1);
    let materialized = serde_json::to_string(&json!({"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"alpha","config_dir":dir})).unwrap();
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$1\" >> {log}\ncase \"$1\" in\nlist) cat {list} ;;\ncredential-state) cat {state} ;;\nmaterialize) printf '%s\\n' {materialized} ;;\n*) exit 1 ;;\nesac\n",
        log = shell_words::quote(&root.join("broker.log").to_string_lossy()),
        list = shell_words::quote(&root.join("inventory.json").to_string_lossy()),
        state = shell_words::quote(&root.join("credential-state.json").to_string_lossy()),
        materialized = shell_words::quote(&materialized)
    );
    let path = executable(&root.join("broker"), &script);
    EnvGuard::set(
        lock,
        "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
        &json!([path]).to_string(),
    )
}

fn publish(root: &Path, n: u64) {
    fs::write(
        root.join("credential-state.json"),
        serde_json::to_vec(&crate::claude_account::generation::tests::envelope(n)).unwrap(),
    )
    .unwrap();
}

fn recovery_receipt(context: &CliContext) -> Value {
    let store: Value = serde_json::from_slice(
        &fs::read(crate::session_dir(context, ID).join("auth-incidents.json")).unwrap(),
    )
    .unwrap();
    store["incidents"][0]["same_account_recovery"].clone()
}

fn change_receipt(context: &CliContext, state: &str) {
    let path = crate::session_dir(context, ID).join("auth-incidents.json");
    let mut store: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    store["incidents"][0]["same_account_recovery"]["state"] = json!(state);
    fs::write(path, serde_json::to_vec(&store).unwrap()).unwrap();
}

fn tick(context: &CliContext, record: &SessionRecord, tmux: &Path, now: i64) {
    crate::auth_incident::recovery::tick(context, record, tmux, now).unwrap();
}

#[test]
fn auth_recovery_waits_for_publication_and_resumes_once_without_replaying_input() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, true);
    crate::coordination::auth_effects::test_support::seed(&context, &record);
    let before = log(tmp.path());
    tick(&context, &record, &tmux, 100);
    let pending = recovery_receipt(&context);
    assert_eq!(pending["state"], "waiting_generation");
    assert_eq!(pending["polls"], 1);
    assert!(!log(tmp.path())[before.len()..].contains("kill-session"));
    publish(tmp.path(), 2);
    // The fixture provider exits as the controlled stop probes it. Live
    // force-stop requires a private cgroup, covered by lifecycle owner tests.
    fs::write(tmp.path().join("exit-on-probe"), "").unwrap();
    let watcher = on_relaunched_prompt(&context, tmp.path(), &launch_id(&record).unwrap());
    tick(&context, &record, &tmux, 115);
    watcher.join().unwrap();
    let receipt = recovery_receipt(&context);
    assert_eq!(receipt["state"], "runtime_resumed", "{receipt}");
    assert_eq!(receipt["operation_id"], pending["operation_id"]);
    assert_eq!(receipt["replayed"], false);
    assert_eq!(receipt["reread"], "unsupported");
    assert_eq!(receipt["unknown_operation_count"], 2);
    assert_eq!(receipt["unknown_in_flight_effects"][0]["state"], "unknown");
    assert_eq!(receipt["untracked_effects"], "unknown");
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
        Some("alpha")
    );
    crate::coordination::auth_effects::test_support::assert_retired(&context, &record);
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &current).is_ok());
    let effects = &log(tmp.path())[before.len()..];
    assert!(!effects.contains("load-buffer"));
    assert!(!effects.contains("send-keys"));
    assert!(effects.contains("--resume"));
    let launches = log(tmp.path()).matches("new-session").count();
    tick(&context, &current, &tmux, 130);
    assert_eq!(log(tmp.path()).matches("new-session").count(), launches);
    // Simulate the controller dying after the replacement identity was saved.
    change_receipt(&context, "resuming");
    tick(&context, &current, &tmux, 145);
    assert_eq!(recovery_receipt(&context)["state"], "runtime_resumed");
    assert_eq!(log(tmp.path()).matches("new-session").count(), launches);

    // A consecutive fault needs another generation of this same account.
    crate::auth_incident::observe(
        &context,
        &current,
        "second-auth-failure",
        crate::auth_incident::AuthSource::ClaudeStopFailure,
        crate::activity::Confidence::Authoritative,
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    tick(&context, &current, &tmux, 160);
    assert_eq!(log(tmp.path()).matches("new-session").count(), launches);
    publish(tmp.path(), 3);
    fs::write(tmp.path().join("exit-on-probe"), "").unwrap();
    let root = tmp.path().to_path_buf();
    let watcher = on_relaunch(
        &context,
        &launch_id(&current).unwrap(),
        move |context, launch| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while log(&root).matches("new-session").count() < 3 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
            }
            session_ready(context, launch);
        },
    );
    tick(&context, &current, &tmux, 175);
    watcher.join().unwrap();
    let final_record = load_session_record(&context, ID).unwrap();
    assert_eq!(
        crate::claude_account::view_for_record(&final_record)
            .unwrap()
            .selected_account
            .as_deref(),
        Some("alpha")
    );
    assert_eq!(log(tmp.path()).matches("new-session").count(), launches + 1);
    let store: Value = serde_json::from_slice(
        &fs::read(crate::session_dir(&context, ID).join("auth-incidents.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store["incidents"][1]["same_account_recovery"]["state"],
        "runtime_resumed"
    );
    assert_ne!(
        store["incidents"][1]["same_account_recovery"]["operation_id"],
        receipt["operation_id"]
    );
    assert!(
        !fs::read_to_string(tmp.path().join("broker.log"))
            .unwrap()
            .lines()
            .any(|verb| verb == "select")
    );
}

#[test]
fn auth_recovery_observed_health_cancels_a_waiting_replacement() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, true);
    tick(&context, &record, &tmux, 100);
    crate::auth_incident::recover(
        &context,
        &record,
        "healthy",
        &jiff::Timestamp::now().to_string(),
    )
    .unwrap();
    publish(tmp.path(), 2);
    tick(&context, &record, &tmux, 115);
    assert_eq!(recovery_receipt(&context)["state"], "recovered_in_place");
    assert_eq!(log(tmp.path()).matches("new-session").count(), 1);
    assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_ok());
}

#[test]
fn auth_recovery_deadline_and_crash_ambiguity_never_restart_blindly() {
    let lock = GlobalStateLock::new();
    for scenario in [
        "deadline",
        "resume_unknown",
        "missing_baseline",
        "capability",
        "queued_account",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let _broker = broker(tmp.path(), &lock);
        let context = context(tmp.path());
        let tmux = fake_tmux(tmp.path());
        let mut record = failed_runtime(&context, tmp.path(), &tmux, true);
        if scenario == "missing_baseline" {
            record.extra.remove("claude_launch_publication");
            crate::write_session_record(&context, &record).unwrap();
        }
        if scenario == "capability" {
            fs::write(
                tmp.path().join("inventory.json"),
                r#"{"schema_version":"agent-session.account-broker.v2","provider":"claude"}"#,
            )
            .unwrap();
        }
        if scenario == "queued_account" {
            record
                .extra
                .insert("claude_account_next".into(), json!({"state":"queued"}));
            crate::write_session_record(&context, &record).unwrap();
        }
        let before = log(tmp.path());
        tick(&context, &record, &tmux, 100);
        if scenario == "resume_unknown" {
            change_receipt(&context, "resuming");
        }
        tick(&context, &record, &tmux, 221);
        let receipt = recovery_receipt(&context);
        assert_eq!(receipt["state"], "blocked", "{scenario}: {receipt}");
        assert_eq!(
            receipt["last_reason"],
            match scenario {
                "deadline" => "publication_deadline",
                "resume_unknown" => "resume_outcome_unknown",
                "missing_baseline" => "launch_generation_missing",
                "capability" => "capability_missing",
                _ => "binding_or_conversation_changed",
            }
        );
        assert!(!log(tmp.path())[before.len()..].contains("kill-session"));
        assert_eq!(log(tmp.path()).matches("new-session").count(), 1);
        assert!(crate::auth_incident::recovery::ensure_input_allowed(&context, &record).is_err());
    }
}

#[test]
fn auth_recovery_refuses_partial_and_changed_epoch_publication() {
    let lock = GlobalStateLock::new();
    for scenario in ["partial", "epoch", "old", "unreachable"] {
        let tmp = tempfile::tempdir().unwrap();
        let _broker = broker(tmp.path(), &lock);
        let context = context(tmp.path());
        let tmux = fake_tmux(tmp.path());
        let record = failed_runtime(&context, tmp.path(), &tmux, true);
        let mut value = crate::claude_account::generation::tests::envelope(if scenario == "old" {
            1
        } else {
            2
        });
        match scenario {
            "partial" => value["state"]["publication"]["status"] = json!("partial"),
            "epoch" => {
                for path in ["target", "last_published"] {
                    if path == "target" {
                        value["state"]["publication"][path]["authority_epoch"] =
                            json!("20000000-0000-4000-8000-000000000002");
                    } else {
                        value["state"]["publication"][path]["version"]["authority_epoch"] =
                            json!("20000000-0000-4000-8000-000000000002");
                    }
                }
                value["state"]["authority"]["version"]["authority_epoch"] =
                    json!("20000000-0000-4000-8000-000000000002");
            }
            "unreachable" => value = Value::Null,
            _ => {}
        }
        fs::write(tmp.path().join("credential-state.json"), value.to_string()).unwrap();
        let before = log(tmp.path());
        tick(&context, &record, &tmux, 100);
        assert_eq!(
            recovery_receipt(&context)["state"],
            "waiting_generation",
            "{scenario}"
        );
        assert!(!log(tmp.path())[before.len()..].contains("kill-session"));
        // A later publication recovers without a human or another incident.
        publish(tmp.path(), 2);
        fs::write(tmp.path().join("exit-on-probe"), "").unwrap();
        let watcher = on_relaunched_prompt(&context, tmp.path(), &launch_id(&record).unwrap());
        tick(&context, &record, &tmux, 115);
        watcher.join().unwrap();
        assert_eq!(
            recovery_receipt(&context)["state"],
            "runtime_resumed",
            "{scenario}"
        );
    }
}

#[test]
fn auth_recovery_stop_without_a_safe_boundary_never_relaunches() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, true);
    publish(tmp.path(), 2);
    tick(&context, &record, &tmux, 100);
    assert_eq!(recovery_receipt(&context)["state"], "blocked");
    assert_eq!(
        recovery_receipt(&context)["last_reason"],
        "session-termination-failed"
    );
    assert_eq!(log(tmp.path()).matches("new-session").count(), 1);
    assert_eq!(
        launch_id(&load_session_record(&context, ID).unwrap()),
        launch_id(&record)
    );
}

#[test]
fn auth_quarantine_blocks_bare_enter_even_with_allow_blocked() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    failed_runtime(&context, tmp.path(), &tmux, false);
    let before = log(tmp.path());
    let args =
        crate::send::unguarded_args(ID.to_string(), None, vec![SpecialKey::Enter], true, tmux);
    let result = crate::send::send_to_session(&context, args);
    assert_eq!(
        result.err().map(|e| e.code().to_string()),
        Some("auth-quarantined".into())
    );
    assert!(!log(tmp.path())[before.len()..].contains("send-keys"));
}

#[test]
fn auth_quarantine_blocks_structured_and_auto_resume_prompts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, false);
    let before = log(tmp.path());
    let result = crate::deliver_claude_structured_prompt_locked(
        &context,
        ID,
        &launch_id(&record).unwrap(),
        "synthetic prompt",
        &tmux,
    );
    assert_eq!(
        result.err().map(|e| e.code().to_string()),
        Some("auth-quarantined".into())
    );
    let _lock = crate::acquire_session_record_lock(&context, ID).unwrap();
    let result = crate::send_auto_resume_input(&context, &record, "continue", &tmux);
    assert_eq!(
        result.err().map(|e| e.code().to_string()),
        Some("auth-quarantined".into())
    );
    assert!(!log(tmp.path())[before.len()..].contains("paste-buffer"));
}

#[test]
fn auth_quarantine_blocks_raw_terminal_and_notifications_but_allows_interrupt() {
    let tmp = tempfile::tempdir().unwrap();
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, false);
    let error =
        crate::send_input_serialized(&context, &record, Some("prompt"), &[], &tmux).unwrap_err();
    assert_eq!(error.code(), "auth-quarantined");
    let candidate = queued_notification(&context, tmp.path());
    assert!(!try_notification(&context, &candidate));
    crate::send::send_to_session(
        &context,
        crate::send::unguarded_args(ID.into(), None, vec![SpecialKey::CtrlC], false, tmux),
    )
    .unwrap();
}

#[test]
fn auth_recovery_caps_publication_polls_even_without_clock_progress() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = broker(tmp.path(), &lock);
    let context = context(tmp.path());
    let tmux = fake_tmux(tmp.path());
    let record = failed_runtime(&context, tmp.path(), &tmux, true);
    for _ in 0..12 {
        tick(&context, &record, &tmux, 100);
    }
    assert_eq!(recovery_receipt(&context)["polls"], 8);
    assert_eq!(
        recovery_receipt(&context)["last_reason"],
        "publication_deadline"
    );
    assert_eq!(log(tmp.path()).matches("new-session").count(), 1);
}
