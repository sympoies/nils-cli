mod codex_idle_notification_tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use pretty_assertions::assert_eq;

    fn idle_notification_fixture() -> (tempfile::TempDir, CliContext, SessionRecord) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        seed_session_with_runtime(tmp.path(), "idle-recipient", "codex", "fixture-recipient");
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let record = load_session_record(&context, "idle-recipient").expect("record");
        crate::activity::activate_runtime(&context, &record).expect("activate runtime");
        (tmp, context, record)
    }

    fn idle_notification_event(
        context: &CliContext,
        record: &SessionRecord,
        event_id: &str,
        kind: &str,
        turn_id: &str,
        confidence: &str,
    ) -> crate::activity::ActivityResult {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": record.runtime.as_ref().expect("runtime").launch_id,
            "provider": "codex",
            "provider_turn_id": turn_id,
            "kind": kind,
            "confidence": confidence
        }))
        .expect("event");
        crate::activity::ingest_event(context, &record.id, event).expect("ingest event")
    }

    fn complete_idle_notification_turn(context: &CliContext, record: &SessionRecord) {
        idle_notification_event(
            context,
            record,
            "idle-start",
            "turn_started",
            "idle-turn",
            "authoritative",
        );
        idle_notification_event(
            context,
            record,
            "idle-complete",
            "turn_completed",
            "idle-turn",
            "authoritative",
        );
        assert!(terminal_notification_waiting(
            context,
            record,
            AgentKind::Codex
        ));
    }

    #[test]
    fn codex_idle_notification_late_stop_preserves_completed_turn_eligibility() {
        let (_tmp, context, record) = idle_notification_fixture();
        complete_idle_notification_turn(&context, &record);
        idle_notification_event(
            &context,
            &record,
            "late-stop",
            "stop_observed",
            "idle-turn",
            "observed",
        );
        let duplicate = idle_notification_event(
            &context,
            &record,
            "duplicate-complete",
            "turn_completed",
            "idle-turn",
            "authoritative",
        );
        assert!(
            duplicate.duplicate,
            "replayed completion must remain deduplicated"
        );
        let projected = activity::state_for_view(&context, &record).expect("projection");
        assert_eq!(projected.phase, activity::TurnPhase::Waiting);
        assert!(projected.current_turn.is_none());
        assert_eq!(
            projected
                .last_turn
                .as_ref()
                .expect("completed turn")
                .outcome,
            "completed"
        );
        assert!(
            terminal_notification_waiting(&context, &record, AgentKind::Codex),
            "a late Stop that starts no turn must not disable wake after exact completion"
        );
    }

    #[test]
    fn codex_idle_notification_stop_alone_does_not_authorize_wake() {
        let (_tmp, context, record) = idle_notification_fixture();
        idle_notification_event(
            &context,
            &record,
            "open-start",
            "turn_started",
            "open-turn",
            "authoritative",
        );
        idle_notification_event(
            &context,
            &record,
            "open-stop",
            "stop_observed",
            "open-turn",
            "observed",
        );
        let projected = activity::state_for_view(&context, &record).expect("projection");
        assert_eq!(projected.phase, activity::TurnPhase::Working);
        assert!(projected.current_turn.is_some());
        assert!(!terminal_notification_waiting(
            &context,
            &record,
            AgentKind::Codex
        ));
    }

    #[test]
    fn codex_idle_notification_new_turn_cancels_completed_turn_eligibility() {
        let (_tmp, context, record) = idle_notification_fixture();
        complete_idle_notification_turn(&context, &record);
        idle_notification_event(
            &context,
            &record,
            "new-start",
            "turn_started",
            "new-turn",
            "authoritative",
        );
        let projected_new_turn = activity::state_for_view(&context, &record)
            .expect("new turn projection")
            .current_turn
            .expect("new turn")
            .provider_turn_id;
        assert!(projected_new_turn.is_some());
        idle_notification_event(
            &context,
            &record,
            "new-stop",
            "stop_observed",
            "new-turn",
            "observed",
        );
        let projected = activity::state_for_view(&context, &record).expect("projection");
        assert_eq!(projected.phase, activity::TurnPhase::Working);
        assert_eq!(
            projected
                .current_turn
                .as_ref()
                .expect("new turn")
                .provider_turn_id
                .as_deref(),
            projected_new_turn.as_deref()
        );
        assert!(!terminal_notification_waiting(
            &context,
            &record,
            AgentKind::Codex
        ));
    }

    #[test]
    fn codex_idle_notification_unbound_stop_cannot_inherit_completion_authority() {
        for (turn_id, source_kind) in [
            (None, "provider_hook"),
            (Some("unrelated-turn"), "provider_hook"),
            (Some("idle-turn"), "console_observation"),
            (Some("idle-turn"), "terminal_heuristic"),
        ] {
            let (_tmp, context, record) = idle_notification_fixture();
            complete_idle_notification_turn(&context, &record);
            let event = serde_json::from_value(json!({
                "schema_version": crate::activity::TURN_EVENT_VERSION,
                "event_id": "unbound-stop",
                "source_kind": source_kind,
                "runtime_id": record.runtime.as_ref().expect("runtime").launch_id,
                "provider": "codex",
                "provider_turn_id": turn_id,
                "kind": "stop_observed",
                "confidence": "observed"
            }))
            .expect("event");
            crate::activity::ingest_event(&context, &record.id, event).expect("ingest Stop");
            let projected = activity::state_for_view(&context, &record).expect("projection");
            assert_eq!(projected.source.confidence, activity::Confidence::Observed);
            assert!(!terminal_notification_waiting(
                &context,
                &record,
                AgentKind::Codex
            ));
        }
    }

    #[tokio::test]
    async fn quiet_working_codex_native_idle_failure_precedes_submission_side_effects() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );

        for probe_result in ["active", "unavailable"] {
            let tmp = tempfile::TempDir::new().expect("tempdir");
            let id = format!("quiet-native-{probe_result}");
            let launch_id = seed_codex_app_server_session(tmp.path(), &id);
            let mut record = load_session_record(
                &CliContext {
                    state_dir: tmp.path().to_path_buf(),
                    host: None,
                },
                &id,
            )
            .expect("seeded Codex session");
            let provider_id = format!("provider-{probe_result}");
            record.provider_resume = Some(crate::ProviderResume {
                provider: "codex".into(),
                session_id: provider_id.clone(),
                captured_at: "2000-01-01T00:00:00Z".into(),
                capture_method: "fixture".into(),
                resume_args: Vec::new(),
                extra: std::collections::BTreeMap::new(),
            });
            let context = CliContext {
                state_dir: tmp.path().to_path_buf(),
                host: None,
            };
            crate::write_session_record(&context, &record).expect("write provider binding");
            crate::activity::activate_runtime(&context, &record).expect("activate runtime");
            let event = serde_json::from_value(json!({
                "schema_version": crate::activity::TURN_EVENT_VERSION,
                "event_id": format!("{id}-turn-start"),
                "runtime_id": launch_id.clone(),
                "provider": "codex",
                "provider_session_id": provider_id.clone(),
                "provider_turn_id": format!("turn-{probe_result}"),
                "kind": "turn_started",
                "confidence": "authoritative"
            }))
            .expect("turn start event");
            crate::activity::ingest_event(&context, &record.id, event).expect("start turn");

            let activity_path = tmp.path().join("sessions").join(&id).join("activity.json");
            let mut activity: Value =
                serde_json::from_slice(&fs::read(&activity_path).expect("activity snapshot"))
                    .expect("activity JSON");
            activity["state"]["phase_changed_at"] = json!("2000-01-01T00:00:00Z");
            activity["state"]["current_turn"]["started_at"] = json!("2000-01-01T00:00:00Z");
            fs::write(
                &activity_path,
                serde_json::to_vec_pretty(&activity).expect("activity snapshot JSON"),
            )
            .expect("write quiet activity snapshot");
            let projected = crate::activity::state_for_view(&context, &record)
                .expect("quiet projection");
            assert_eq!(projected.phase, crate::activity::TurnPhase::Working);
            assert_eq!(projected.source.confidence, crate::activity::Confidence::Inferred);
            assert!(crate::activity::codex_notification_maybe_idle(&projected));
            crate::auto_resume::set_enabled(
                &context,
                &record.id,
                true,
                "2030-01-01T00:00:00Z",
            )
            .expect("enable auto-resume");
            crate::auto_resume::rearm_usage_exhaustion_for_runtime(
                &context,
                &record.id,
                &launch_id,
                format!("turn-{probe_result}"),
                projected.revision,
                "2030-01-01T00:00:01Z",
            )
            .expect("arm auto-resume");

            let socket_path = tmp.path().join(format!("{id}.sock"));
            let listener = tokio::net::UnixListener::bind(&socket_path).expect("probe socket");
            let unavailable = probe_result == "unavailable";
            let probe = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.expect("probe connection");
                let mut socket = tokio_tungstenite::accept_async(stream)
                    .await
                    .expect("probe websocket");
                while let Some(Ok(message)) = socket.next().await {
                    let request: Value =
                        serde_json::from_str(message.to_text().expect("probe request text"))
                            .expect("probe request JSON");
                    let Some(method) = request.get("method").and_then(Value::as_str) else {
                        continue;
                    };
                    if method == "initialized" {
                        continue;
                    }
                    if method == "thread/read" && unavailable {
                        return;
                    }
                    let result = match method {
                        "initialize" => json!({}),
                        "thread/loaded/list" => {
                            json!({"data":[provider_id],"nextCursor":null})
                        }
                        "thread/read" => json!({
                            "thread": {
                                "id": provider_id,
                                "status": {"type": "active"}
                            }
                        }),
                        _ => panic!("unexpected native probe method: {method}"),
                    };
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            json!({"id":request["id"],"result":result})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .expect("native probe response");
                    if method == "thread/read" {
                        return;
                    }
                }
            });

            let session_path = tmp.path().join("sessions").join(&id).join("session.json");
            let auto_resume_path = tmp
                .path()
                .join("sessions")
                .join(&id)
                .join("auto-resume.json");
            let session_before = fs::read(&session_path).expect("session before submission");
            let auto_resume_before =
                fs::read(&auto_resume_path).expect("auto-resume before submission");
            assert_eq!(
                crate::auto_resume::view_for_record(&context, &record).state,
                "armed"
            );

            let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
            let (handle, mut commands) = crate::codex_app_server::control_channel();
            let response = submit_structured_prompt_locked(
                &st,
                &id,
                &launch_id,
                StructuredPromptGuards {
                    expected_session_incarnation: Some(&launch_id),
                    notification: Some(CoordinationNotificationFence {
                        candidate: crate::coordination::NotificationCandidate {
                            target_session_id: id.clone(),
                            target_incarnation: launch_id.clone(),
                            generation: 1,
                            queued_at_epoch: 0,
                            attempted_at_epoch: 0,
                            attempted_at: None,
                        },
                    }),
                    ..Default::default()
                },
                &handle,
                "must not be submitted while native idle is unproven",
            )
            .await
            .expect_err("native idle failure must reject fenced submission");
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("response body");
            let body: Value = serde_json::from_slice(&bytes).expect("response JSON");
            assert_eq!(
                body["error"]["code"],
                "coordination-notification-superseded"
            );
            assert!(matches!(
                commands.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
            ));
            assert_eq!(
                fs::read(&session_path).expect("session after submission"),
                session_before,
                "failed native idle verification must not write the input fence or update the session"
            );
            assert_eq!(
                fs::read(&auto_resume_path).expect("auto-resume after submission"),
                auto_resume_before,
                "failed native idle verification must not cancel auto-resume"
            );
            let after = load_session_record(&context, &id).expect("session after recheck");
            assert!(!after.extra.contains_key("codex_account_input_fence"));
            assert_eq!(
                crate::auto_resume::view_for_record(&context, &after).state,
                "armed"
            );
            probe.await.expect("native probe server");
        }
    }
}
