//! Real broker subprocess results through the usage continuation producer.
use super::*;
use pretty_assertions::assert_eq;

async fn selection_tick(
    script_body: &str,
    expected_reason: Option<&str>,
    selected: bool,
    exhausted: bool,
) {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(&broker, format!("#!/bin/sh\n{script_body}\n")).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let argv = serde_json::to_string(&vec![broker.to_string_lossy()]).unwrap();
    let _broker = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER", &argv);
    let launch = seed_codex_app_server_session(tmp.path(), "selection");
    let st = state(tmp.path(), Some(TOKEN), codex_tmux(tmp.path(), "selection"));
    let mut record = bind_codex(&st.context, "selection", tmp.path());
    crate::codex_account::seed_bound_binding_for_test(&mut record, "account-a", &launch);
    crate::write_session_record(&st.context, &record).unwrap();
    activity::activate_runtime(&st.context, &record).unwrap();
    crate::codex_account::authorize_proxy_input_locked(&st.context, &mut record, true).unwrap();
    let failure = activity::ingest_codex_app_server_failure_with_kind(
        &st.context,
        &record.id,
        &launch,
        "thread-capacity",
        "turn-capacity",
        codex_app_server::StructuredFailureKind::UsageExhausted,
    )
    .unwrap();
    auto_resume::set_enabled_with_policy(
        &st.context,
        &record.id,
        true,
        auto_resume::NEXT_ACCOUNT_THEN_RESUME_POLICY,
        "2020-01-01T00:00:00Z",
    )
    .unwrap();
    auto_resume::arm_usage_exhaustion(
        &st.context,
        &record.id,
        failure
            .turn_state
            .last_turn
            .unwrap()
            .provider_turn_id
            .unwrap(),
        activity::state_for_view(&st.context, &record)
            .unwrap()
            .revision,
        "2020-01-01T00:00:01Z",
    )
    .unwrap();
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch.clone(),
            handle,
        },
    );
    let recover_to_no_account = expected_reason == Some("codex-account-broker-invalid-response");
    let responder = tokio::spawn(async move {
        for _ in 0..if recover_to_no_account { 3 } else { 2 } {
            let Some(codex_app_server::ControlCommand::Usage(response)) = commands.recv().await
            else {
                panic!("selection must only request usage, without submitting a continuation");
            };
            response
                .send(Ok(UsageSnapshot {
                    authoritative: true,
                    has_exhausted_windows: exhausted,
                    exhausted_reset_epochs: if exhausted {
                        vec![4_102_444_800]
                    } else {
                        Vec::new()
                    },
                    soonest_reset_epoch: None,
                }))
                .unwrap();
        }
    });
    let mut first_schedule = None;
    let mut first_durable = None;
    for _ in 0..2 {
        process_codex_auto_resume_id(
            st.clone(),
            CodexAutoResumeTarget {
                id: record.id.clone(),
                launch_id: launch.clone(),
                binding: crate::codex_account::binding_snapshot(&record),
            },
        )
        .await;
        let view = auto_resume::view_for_record(&st.context, &record);
        assert_eq!(view.failure_reason.as_deref(), expected_reason);
        assert_eq!(
            view.state,
            if selected {
                "switching_account"
            } else if exhausted {
                "scheduled"
            } else {
                "transient_failure"
            }
        );
        assert_eq!(view.attempts, 0);
        assert!(
            !serde_json::to_string(&view)
                .unwrap()
                .contains("BROKER_PRIVATE_CANARY")
        );
        let durable = fs::read_to_string(
            crate::session_dir(&st.context, &record.id).join("auto-resume.json"),
        )
        .unwrap();
        assert!(!durable.contains("BROKER_PRIVATE_CANARY"));
        let mut durable: Value = serde_json::from_str(&durable).unwrap();
        if !exhausted {
            assert_eq!(durable["attempt"], 1);
            assert!(durable["next_check_at"].is_string());
        }
        durable.as_object_mut().unwrap().remove("failure_reason");
        if let Some(first) = &first_durable {
            assert_eq!(&durable, first);
        }
        first_durable = Some(durable);
        if !selected && exhausted {
            let scheduled = view
                .scheduled_at
                .as_ref()
                .unwrap()
                .parse::<jiff::Timestamp>()
                .unwrap()
                .as_second();
            assert!((4_102_444_800..=4_102_444_830).contains(&scheduled));
            if let Some(first) = first_schedule {
                assert_eq!(scheduled, first);
            }
            first_schedule = Some(scheduled);
        } else {
            assert!(view.scheduled_at.is_none());
        }
    }
    if recover_to_no_account {
        fs::write(&broker, "#!/bin/sh\nexit 10\n").unwrap();
        process_codex_auto_resume_id(
            st.clone(),
            CodexAutoResumeTarget {
                id: record.id.clone(),
                launch_id: launch.clone(),
                binding: crate::codex_account::binding_snapshot(&record),
            },
        )
        .await;
        let view = auto_resume::view_for_record(&st.context, &record);
        assert_eq!(view.failure_reason.as_deref(), Some("no_account_available"));
        assert_eq!(
            view.state,
            if exhausted {
                "scheduled"
            } else {
                "transient_failure"
            }
        );
        assert_eq!(view.attempts, 0);
        assert!(
            !serde_json::to_string(&view)
                .unwrap()
                .contains("BROKER_PRIVATE_CANARY")
        );
        let durable = fs::read_to_string(
            crate::session_dir(&st.context, &record.id).join("auto-resume.json"),
        )
        .unwrap();
        assert!(!durable.contains("BROKER_PRIVATE_CANARY"));
        let mut durable: Value = serde_json::from_str(&durable).unwrap();
        durable.as_object_mut().unwrap().remove("failure_reason");
        assert_eq!(durable, first_durable.unwrap());
        if exhausted {
            assert_eq!(
                view.scheduled_at
                    .unwrap()
                    .parse::<jiff::Timestamp>()
                    .unwrap()
                    .as_second(),
                first_schedule.unwrap()
            );
        } else {
            assert!(view.scheduled_at.is_none());
        }
    }
    responder.await.unwrap();
    let current = crate::load_session_record(&st.context, &record.id).unwrap();
    let next = crate::codex_account::view_for_record(&current).next;
    assert_eq!(
        next.and_then(|next| next.account).as_deref(),
        selected.then_some("account-b")
    );
}

#[tokio::test]
async fn continuation_selection_fault_surfaces_typed_cause_without_changing_schedule() {
    selection_tick(
        "printf 'BROKER_PRIVATE_CANARY\\n'; printf 'BROKER_PRIVATE_CANARY\\n' >&2",
        Some("codex-account-broker-invalid-response"),
        false,
        true,
    )
    .await;
}

#[tokio::test]
async fn continuation_selection_no_account_keeps_existing_quiet_schedule() {
    selection_tick("exit 10", Some("no_account_available"), false, true).await;
}

#[tokio::test]
async fn continuation_selection_success_queues_the_selected_account_once() {
    selection_tick(r#"printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"account-b"}'"#,
        None, true, true).await;
}

#[tokio::test]
async fn continuation_selection_fault_preserves_durable_retry_budget_until_no_account() {
    selection_tick(
        "printf 'BROKER_PRIVATE_CANARY\\n'; printf 'BROKER_PRIVATE_CANARY\\n' >&2",
        Some("codex-account-broker-invalid-response"),
        false,
        false,
    )
    .await;
}
