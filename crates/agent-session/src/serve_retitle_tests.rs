#[cfg(test)]
mod retitle_tests {
    use super::*;

    #[tokio::test]
    async fn session_retitle_readiness_is_an_authenticated_sanitized_contract() {
        let lock = GlobalStateLock::new();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", "");
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let app = router(state(
            &tmp.path().join("state"),
            Some(TOKEN),
            minimal_tmux(tmp.path()),
        ));

        let (status, body) = call(app, get_auth("/retitle/readiness", Some(TOKEN))).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["machine"], MACHINE);
        assert_eq!(
            body["data"]["retitle"]["schema_version"],
            "agent-session.session-retitle.readiness.v2"
        );
        assert_eq!(
            body["data"]["retitle"]["capability"],
            "agent-session.session-retitle.v2"
        );
        assert_eq!(body["data"]["retitle"]["status"], "unavailable");
        assert_eq!(
            body["data"]["retitle"]["reason_code"],
            "provider_not_configured"
        );
        assert_eq!(body["data"]["retitle"]["next_action"], "configure_provider");
        let rendered = body.to_string();
        assert!(!rendered.contains(tmp.path().to_string_lossy().as_ref()));
        assert!(!rendered.contains("s3cr3t-token"));
    }

    #[tokio::test]
    async fn retitle_v3_readiness_is_additive_authenticated_and_sanitized() {
        let lock = GlobalStateLock::new();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", "");
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let app = router(state(
            &tmp.path().join("state"),
            Some(TOKEN),
            minimal_tmux(tmp.path()),
        ));

        let (status, body) = call(app, get_auth("/retitle/v3/readiness", Some(TOKEN))).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"]["retitle"]["schema_version"],
            "agent-session.session-retitle.readiness.v3"
        );
        assert_eq!(
            body["data"]["retitle"]["capability"],
            "agent-session.session-retitle.v3"
        );
        assert_eq!(body["data"]["retitle"]["status"], "degraded");
        let rendered = body.to_string();
        assert!(!rendered.contains(tmp.path().to_string_lossy().as_ref()));
        assert!(!rendered.contains(TOKEN));
    }

    #[test]
    fn retitle_v3_continuation_registry_is_single_flight_and_releases() {
        let running = Arc::new(StdMutex::new(HashSet::new()));
        let operation_hash =
            "sha256:1111111111111111111111111111111111111111111111111111111111111111";

        let first = RetitleV3ContinuationGuard::register(running.clone(), operation_hash)
            .expect("first continuation owns the operation");
        assert!(RetitleV3ContinuationGuard::is_registered(
            &running,
            operation_hash
        ));
        assert!(RetitleV3ContinuationGuard::register(running.clone(), operation_hash).is_none());
        drop(first);
        assert!(!RetitleV3ContinuationGuard::is_registered(
            &running,
            operation_hash
        ));
        assert!(RetitleV3ContinuationGuard::register(running, operation_hash).is_some());
    }

    #[test]
    fn retitle_v3_continuation_follow_up_stops_permanent_failures() {
        assert_eq!(
            RetitleV3ContinuationOutcome::Completed.follow_up_delay(),
            Some(Duration::ZERO)
        );
        assert_eq!(
            RetitleV3ContinuationOutcome::Retryable.follow_up_delay(),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            RetitleV3ContinuationOutcome::Stopped.follow_up_delay(),
            None
        );
    }

    #[tokio::test(start_paused = true)]
    async fn retitle_v3_retry_delay_holds_the_single_flight_guard() {
        let running = Arc::new(StdMutex::new(HashSet::new()));
        let operation_hash =
            "sha256:2222222222222222222222222222222222222222222222222222222222222222";
        let guard = RetitleV3ContinuationGuard::register(running.clone(), operation_hash)
            .expect("continuation owns the operation");
        let follow_up = tokio::spawn(wait_for_retitle_v3_follow_up(
            guard,
            RetitleV3ContinuationOutcome::Retryable,
        ));

        tokio::task::yield_now().await;
        assert!(RetitleV3ContinuationGuard::is_registered(
            &running,
            operation_hash
        ));
        assert!(RetitleV3ContinuationGuard::register(running.clone(), operation_hash).is_none());

        tokio::time::advance(Duration::from_secs(29)).await;
        assert!(RetitleV3ContinuationGuard::is_registered(
            &running,
            operation_hash
        ));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(follow_up.await.unwrap());
        assert!(!RetitleV3ContinuationGuard::is_registered(
            &running,
            operation_hash
        ));
    }

    #[tokio::test]
    async fn retitle_v3_continuation_terminalizes_permanent_refresh_failure_before_readoption() {
        let lock = GlobalStateLock::new();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", "");
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let codex_home = tmp.path().join("codex-home");
        let transcript_dir = codex_home.join("sessions/2026/09/10");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        let transcript = transcript_dir.join("rollout.jsonl");
        std::fs::write(
            &transcript,
            concat!(
                "{\"timestamp\":\"2026-09-10T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"provider-permanent-refresh\",\"cwd\":\"/tmp\",\"source\":\"cli\",\"timestamp\":\"2026-09-10T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-09-10T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Fix automatic session titles\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-one\",\"content_item_kinds\":[\"user.text\"]}}}\n"
            ),
        )
        .unwrap();
        let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_string_lossy().as_ref());
        let id = "permanent-refresh-failure";
        let tmux = minimal_tmux(tmp.path());
        seed_session_with_runtime(tmp.path(), id, "codex", &format!("hs-{id}"));
        let record_path = tmp.path().join(format!("sessions/{id}/session.json"));
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["provider_resume"] = json!({
            "provider":"codex",
            "session_id":"provider-permanent-refresh",
            "captured_at":"2026-09-10T00:00:02Z",
            "capture_method":"fixture",
            "resume_args":[]
        });
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        write_automatic_activity(tmp.path(), id, 1, Some("turn-one"), None);
        let state = state(tmp.path(), Some(TOKEN), tmux.clone());
        let view = crate::session_view(
            &state.context,
            &crate::load_session_record(&state.context, id).unwrap(),
            Some("running".into()),
            Some(&tmux),
        );
        let request = automatic_retitle_candidate(&view).unwrap().request;
        let accepted =
            crate::retitle_v3::refresh_once(&state.context, &state.history_catalog, id, &request)
                .unwrap();
        assert_eq!(accepted.status, "accepted");
        std::fs::remove_file(transcript).unwrap();

        let outcome = continue_retitle_v3_operation(
            state.clone(),
            id.to_string(),
            accepted.operation_hash.clone(),
        )
        .await;

        assert_eq!(outcome, RetitleV3ContinuationOutcome::Completed);
        let replay =
            crate::retitle_v3::operation_response(&state.context, id, &accepted.operation_hash)
                .unwrap()
                .unwrap();
        assert_eq!(replay.status, "terminal");
        assert_eq!(replay.outcome.as_deref(), Some("terminal_failure"));
        assert_eq!(
            crate::retitle_v3::selected_pending_operation_hash(
                &crate::load_session_record(&state.context, id).unwrap(),
            ),
            None,
        );
        spawn_selected_retitle_v3_continuation(state.clone(), id.to_string()).await;
        assert!(!RetitleV3ContinuationGuard::is_registered(
            &state.retitle_v3_continuations,
            &accepted.operation_hash,
        ));
    }

    #[test]
    fn automatic_retitle_tracker_retries_an_accepted_non_terminal_operation() {
        let mut tracker = AutomaticRetitleTracker::default();
        let now = Instant::now();
        assert!(tracker.begin("accepted-operation", now));

        for seconds in 1..=u64::from(crate::retitle::MAX_AUTOMATIC_ATTEMPTS) + 2 {
            tracker.finish(
                "accepted-operation",
                Ok("accepted"),
                now + Duration::from_secs(seconds - 1),
            );
            assert!(
                tracker.begin("accepted-operation", now + Duration::from_secs(seconds)),
                "accepted polling must not consume the provider-failure budget",
            );
        }
        tracker.finish(
            "accepted-operation",
            Ok("terminal"),
            now + Duration::from_secs(6),
        );
        assert!(!tracker.begin("accepted-operation", now + Duration::from_secs(60)));
    }

    #[tokio::test]
    async fn retitle_v3_cold_recovery_scans_the_session_catalog_once() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let calls = Arc::new(AtomicUsize::new(0));
        let observed_calls = calls.clone();
        let collector: SessionCollector = Arc::new(move |_, _| {
            observed_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        });
        let state = state_with_session_collector(
            tmp.path(),
            Some(TOKEN),
            minimal_tmux(tmp.path()),
            collector,
        );

        recover_retitle_v3_operations_once(state).await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn pre_observation_retitle_replay_omits_the_additive_attempt_field() {
        let lock = GlobalStateLock::new();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", "");
        let tmp = tempfile::TempDir::new().expect("tempdir");
        seed_session_with_runtime(
            tmp.path(),
            "pre-observation-retitle",
            "codex",
            "hs-pre-observation-retitle",
        );
        let request = crate::retitle::RetitleRequest {
            schema_version: crate::retitle::REQUEST_SCHEMA.into(),
            trigger: crate::retitle::RetitleTrigger::Manual,
            idempotency_key: "manual-pre-observation-replay".into(),
            expected_session_incarnation: "launch-pre-observation-retitle".into(),
            expected_title_revision: 0,
            expected_activity_revision: None,
            expected_provider_turn_id: None,
        };
        let record_path = tmp
            .path()
            .join("sessions/pre-observation-retitle/session.json");
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["session_retitle_v2"] = json!({
            "schema_version":"agent-session.session-retitle-state.v2",
            "receipts":[{
                "key_hash":crate::retitle::hash_identity(&request.idempotency_key),
                "request_hash":crate::retitle::request_hash(&request),
                "state":"complete",
                "diagnostic_code":"no_change",
                "provider_kind":"command",
                "attempt_count":1,
                "coverage_complete":true,
                "coverage_truncated":false,
                "coverage_turn_count":1
            }]
        });
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let app = router(state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path())));

        let (status, body) = call(
            app,
            post_json(
                "/sessions/pre-observation-retitle/retitle",
                Some(TOKEN),
                serde_json::to_value(request).unwrap(),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "body={body}");
        assert_eq!(body["data"]["retitle"]["outcome"], "replayed");
        assert!(
            !body["data"]["retitle"]
                .as_object()
                .unwrap()
                .contains_key("attempt")
        );
    }

    #[test]
    fn orphaned_in_progress_admission_recovers_the_receipt_in_place() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        seed_session_with_runtime(
            tmp.path(),
            "orphaned-retitle",
            "codex",
            "hs-orphaned-retitle",
        );
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let request = crate::retitle::RetitleRequest {
            schema_version: crate::retitle::REQUEST_SCHEMA.into(),
            trigger: crate::retitle::RetitleTrigger::Manual,
            idempotency_key: "manual-orphan-recovery".into(),
            expected_session_incarnation: "launch-orphaned-retitle".into(),
            expected_title_revision: 0,
            expected_activity_revision: None,
            expected_provider_turn_id: None,
        };

        assert!(matches!(
            crate::retitle::admit(&context, "orphaned-retitle", &request).unwrap(),
            crate::retitle::Admission::Evaluate(_)
        ));
        assert!(matches!(
            crate::retitle::admit(&context, "orphaned-retitle", &request).unwrap(),
            crate::retitle::Admission::Evaluate(_)
        ));

        let stored: Value = serde_json::from_slice(
            &std::fs::read(tmp.path().join("sessions/orphaned-retitle/session.json")).unwrap(),
        )
        .unwrap();
        let receipt = &stored["session_retitle_v2"]["receipts"][0];
        assert_eq!(
            stored["session_retitle_v2"]["receipts"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(receipt["attempt_count"], 2);
        assert_eq!(
            receipt["observation"]["attempts"][0]["stage"],
            "interrupted"
        );
        assert_eq!(
            receipt["observation"]["attempts"][0]["failure_stage"],
            "worker_interrupted"
        );
        assert_eq!(receipt["observation"]["attempts"][1]["number"], 2);
        assert_eq!(receipt["observation"]["attempts"][1]["stage"], "admission");
    }

    #[tokio::test]
    async fn manual_retitle_mutates_once_and_replays_without_provider_content() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let provider_calls = tmp.path().join("provider.calls");
        let provider = executable(
            &tmp.path().join("title-provider"),
            &format!(
                "#!/bin/sh\nprintf x >> {}\nsleep 0.1\nprintf '%s\\n' '{{\"topic_action\":\"set\",\"topic\":\"Retitle Redesign\",\"activity\":\"Validate daemon\",\"references\":[\"#449\"]}}'\n",
                shell_words::quote(&provider_calls.to_string_lossy())
            ),
        );
        let config = json!({
            "provider":"openai_compatible",
            "base_url":"http://127.0.0.1:9/v1",
            "model":"ghp_0123456789abcdefghijklmnopqrstuvwxyz",
            "timeout_ms":1000,
            "fallback": {
                "provider":"command",
                "argv":[provider],
                "timeout_ms":5000
            },
            "context":{"max_chars":4000,"per_message_chars":1000,"recent_turns":8}
        })
        .to_string();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", &config);
        let codex_home = tmp.path().join("codex-home");
        let transcript_dir = codex_home.join("sessions/2026/09/07");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        std::fs::write(
            transcript_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-09-07T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"provider-retitle\",\"cwd\":\"/tmp\",\"source\":\"cli\",\"timestamp\":\"2026-09-07T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-09-07T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Implement issue #449 without leaking /private/path\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-a\",\"content_item_kinds\":[\"user.text\"]}}}\n"
            ),
        )
        .unwrap();
        let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_string_lossy().as_ref());
        seed_session_with_runtime(tmp.path(), "retitle", "codex", "hs-codex-retitle");
        let record_path = tmp.path().join("sessions/retitle/session.json");
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["provider_resume"] = json!({
            "provider":"codex",
            "session_id":"provider-retitle",
            "captured_at":"2026-09-07T00:00:02Z",
            "capture_method":"fixture",
            "resume_args":[]
        });
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let app = router(state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path())));
        let request = json!({
            "schema_version":"agent-session.session-retitle.request.v2",
            "trigger":"manual",
            "idempotency_key":"manual-retitle-0001",
            "expected_session_incarnation":"launch-retitle",
            "expected_title_revision":0
        });

        let first_request = call(
            app.clone(),
            post_json("/sessions/retitle/retitle", Some(TOKEN), request.clone()),
        );
        let second_request = call(
            app.clone(),
            post_json("/sessions/retitle/retitle", Some(TOKEN), request.clone()),
        );
        let ((first_status, first), (second_status, second)) =
            tokio::join!(first_request, second_request);
        assert_eq!(first_status, StatusCode::OK, "body={first}");
        assert_eq!(second_status, StatusCode::OK, "body={second}");
        let mut outcomes = [
            first["data"]["retitle"]["outcome"].as_str().unwrap(),
            second["data"]["retitle"]["outcome"].as_str().unwrap(),
        ];
        outcomes.sort_unstable();
        assert_eq!(outcomes, ["committed", "replayed"]);
        let committed = if first["data"]["retitle"]["outcome"] == "committed" {
            &first
        } else {
            &second
        };
        assert_eq!(committed["data"]["retitle"]["provider_kind"], "command");
        assert_eq!(
            committed["data"]["retitle"]["attempt"]["schema_version"],
            "agent-session.session-retitle-attempt.v1"
        );
        assert_eq!(committed["data"]["retitle"]["attempt"]["trigger"], "manual");
        assert_eq!(
            committed["data"]["retitle"]["attempt"]["attempts"][0]["providers"][0]["outcome"],
            "unavailable"
        );
        assert_eq!(
            committed["data"]["retitle"]["attempt"]["attempts"][0]["providers"][1]["outcome"],
            "success"
        );
        assert!(
            committed["data"]["retitle"]["attempt"]["attempts"][0]["context"]
                ["provider_input_chars"]
                .as_u64()
                .is_some_and(|size| size > 0)
        );
        assert_eq!(committed["data"]["retitle"]["session"]["title_revision"], 1);
        assert_eq!(
            committed["data"]["retitle"]["session"]["title_state"]["topic"],
            "Retitle Redesign"
        );
        assert_eq!(
            committed["data"]["retitle"]["session"]["title_state"]["references"][0],
            "#449"
        );
        assert!(!committed.to_string().contains("/private/path"));
        assert!(!committed.to_string().contains("manual-retitle-0001"));
        assert!(!committed.to_string().contains("ghp_0123456789"));
        assert_eq!(std::fs::read_to_string(&provider_calls).unwrap(), "x");

        let stored = std::fs::read_to_string(&record_path).unwrap();
        assert!(!stored.contains("ghp_0123456789"));
        assert!(
            !retitle_observation_log(
                &serde_json::from_value(committed["data"]["retitle"]["attempt"].clone()).unwrap()
            )
            .to_string()
            .contains("ghp_0123456789")
        );

        let (status, listed) = call(app.clone(), get_auth("/sessions", Some(TOKEN))).await;
        assert_eq!(status, StatusCode::OK, "body={listed}");
        assert!(!listed.to_string().contains("ghp_0123456789"));
        assert_eq!(
            listed["data"]["sessions"][0]["retitle_attempt"],
            committed["data"]["retitle"]["attempt"]
        );

        let mut replay = request;
        replay["expected_title_revision"] = json!(1);
        // The key is bound to its complete original request, so a changed fence
        // is rejected instead of reusing the provider result ambiguously.
        let (status, conflict) = call(
            app.clone(),
            post_json("/sessions/retitle/retitle", Some(TOKEN), replay),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "body={conflict}");
        assert_eq!(conflict["error"]["code"], "idempotency-key-reused");

        let first_attempt = committed["data"]["retitle"]["attempt"].clone();
        let (status, second) = call(
            app.clone(),
            post_json(
                "/sessions/retitle/retitle",
                Some(TOKEN),
                json!({
                    "schema_version":"agent-session.session-retitle.request.v2",
                    "trigger":"manual",
                    "idempotency_key":"manual-retitle-0002",
                    "expected_session_incarnation":"launch-retitle",
                    "expected_title_revision":1
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body={second}");
        assert_eq!(second["data"]["retitle"]["outcome"], "unchanged");
        assert_ne!(second["data"]["retitle"]["attempt"], first_attempt);
        assert_eq!(std::fs::read_to_string(&provider_calls).unwrap(), "xx");

        let (status, listed) = call(app.clone(), get_auth("/sessions", Some(TOKEN))).await;
        assert_eq!(status, StatusCode::OK, "body={listed}");
        assert_eq!(
            listed["data"]["sessions"][0]["retitle_attempt"],
            second["data"]["retitle"]["attempt"]
        );

        // An identical retry replays the committed receipt even though its
        // original title fence is now behind the committed revision. Its
        // attempt telemetry must come from the matched receipt, not the newer
        // receipt projected by the session list.
        let restarted_app = router(state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path())));
        let (status, replayed) = call(
            restarted_app,
            post_json(
                "/sessions/retitle/retitle",
                Some(TOKEN),
                json!({
                    "schema_version":"agent-session.session-retitle.request.v2",
                    "trigger":"manual",
                    "idempotency_key":"manual-retitle-0001",
                    "expected_session_incarnation":"launch-retitle",
                    "expected_title_revision":0
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body={replayed}");
        assert_eq!(replayed["data"]["retitle"]["outcome"], "replayed");
        assert_eq!(replayed["data"]["retitle"]["provider_kind"], "command");
        assert_eq!(
            replayed["data"]["retitle"]["diagnostic_code"],
            "idempotency_replay"
        );
        assert_eq!(replayed["data"]["retitle"]["attempt"], first_attempt);
        assert_ne!(
            replayed["data"]["retitle"]["attempt"],
            second["data"]["retitle"]["attempt"]
        );
    }

    #[tokio::test]
    async fn malformed_provider_failure_persists_stage_chain_without_content() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let config = json!({
            "provider":"command",
            "model":"model sk-secret-canary",
            "argv":[
                "/bin/sh",
                "-c",
                "printf '%s\\n' '{\"unexpected\":\"provider output canary /private/session/path sk-secret-canary\"}'"
            ],
            "timeout_ms":5000,
            "context":{"max_chars":4000,"per_message_chars":1000,"recent_turns":8}
        })
        .to_string();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", &config);
        let codex_home = tmp.path().join("codex-home");
        let transcript_dir = codex_home.join("sessions/2026/09/09");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        std::fs::write(
            transcript_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-09-09T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"provider-observation\",\"cwd\":\"/work/example\",\"source\":\"cli\",\"timestamp\":\"2026-09-09T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-09-09T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"user prompt canary /private/session/path sk-secret-canary\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-observation\",\"content_item_kinds\":[\"user.text\"]}}}\n"
            ),
        )
        .unwrap();
        let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_string_lossy().as_ref());
        seed_session_with_runtime(
            tmp.path(),
            "retitle-observation",
            "codex",
            "hs-codex-retitle-observation",
        );
        let record_path = tmp.path().join("sessions/retitle-observation/session.json");
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["provider_resume"] = json!({
            "provider":"codex",
            "session_id":"provider-observation",
            "captured_at":"2026-09-09T00:00:02Z",
            "capture_method":"fixture",
            "resume_args":[]
        });
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let app = router(state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path())));

        let (status, body) = call(
            app.clone(),
            post_json(
                "/sessions/retitle-observation/retitle",
                Some(TOKEN),
                json!({
                    "schema_version":"agent-session.session-retitle.request.v2",
                    "trigger":"manual",
                    "idempotency_key":"manual-observation-failure",
                    "expected_session_incarnation":"launch-retitle-observation",
                    "expected_title_revision":0
                }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={body}");
        assert_eq!(body["error"]["code"], "retitle-provider-malformed-response");
        assert!(!body.to_string().contains("provider output canary"));
        assert!(!body.to_string().contains("/private/session/path"));
        let stored: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        let observation = &stored["session_retitle_v2"]["receipts"][0]["observation"];
        assert_eq!(observation["trigger"], "manual");
        assert_eq!(observation["terminal_outcome"], "failed");
        assert_eq!(
            observation["attempts"][0]["failure_stage"],
            "provider_parse"
        );
        assert_eq!(
            observation["attempts"][0]["providers"][0]["outcome"],
            "malformed_response"
        );
        assert_eq!(
            observation["attempts"][0]["providers"][0]["provider_kind"],
            "command"
        );
        assert!(observation["attempts"][0]["duration_bucket"].is_string());
        assert!(observation["attempts"][0]["context"]["provider_input_chars"].is_number());
        let rendered = observation.to_string();
        for forbidden in [
            "user prompt canary",
            "provider output canary",
            "/private/session/path",
            "sk-secret-canary",
            "model sk-secret-canary",
            "manual-observation-failure",
        ] {
            assert!(!rendered.contains(forbidden));
        }
        let log = retitle_observation_log(
            &crate::retitle::latest_attempt_observation(
                &crate::load_session_record(
                    &CliContext {
                        state_dir: tmp.path().to_path_buf(),
                        host: None,
                    },
                    "retitle-observation",
                )
                .unwrap(),
            )
            .unwrap(),
        );
        assert_eq!(log["event"], "agent_session_retitle_attempt");
        for forbidden in [
            "user prompt canary",
            "provider output canary",
            "/private/session/path",
            "sk-secret-canary",
            "model sk-secret-canary",
        ] {
            assert!(!log.to_string().contains(forbidden));
        }

        let (status, listed) = call(app, get_auth("/sessions", Some(TOKEN))).await;
        assert_eq!(status, StatusCode::OK, "body={listed}");
        assert_eq!(
            listed["data"]["sessions"][0]["retitle_attempt"],
            observation.clone()
        );
    }

    #[tokio::test]
    async fn retitle_failed_receipt_write_does_not_return_an_observation_for_logging() {
        let lock = GlobalStateLock::new();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", "");
        let tmp = tempfile::TempDir::new().expect("tempdir");
        seed_session_with_runtime(
            tmp.path(),
            "retitle-write-failure",
            "codex",
            "hs-retitle-write-failure",
        );
        let request = crate::retitle::RetitleRequest {
            schema_version: crate::retitle::REQUEST_SCHEMA.into(),
            trigger: crate::retitle::RetitleTrigger::Manual,
            idempotency_key: "manual-write-failure".into(),
            expected_session_incarnation: "launch-retitle-write-failure".into(),
            expected_title_revision: 0,
            expected_activity_revision: None,
            expected_provider_turn_id: None,
        };
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        assert!(matches!(
            crate::retitle::admit(&context, "retitle-write-failure", &request).unwrap(),
            crate::retitle::Admission::Evaluate(_)
        ));
        let session_dir = tmp.path().join("sessions/retitle-write-failure");
        let original_mode = std::fs::metadata(&session_dir)
            .unwrap()
            .permissions()
            .mode();
        std::fs::set_permissions(&session_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let serve_state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
        let failure = RetitleEvaluationFailure {
            error: retitle_worker_failed(),
            failure_stage: "provider_worker",
            context: None,
            providers: Vec::new(),
            started: Instant::now(),
        };

        let observation =
            record_retitle_failure(&serve_state, "retitle-write-failure", &request, &failure).await;

        std::fs::set_permissions(&session_dir, std::fs::Permissions::from_mode(original_mode))
            .unwrap();
        assert!(observation.is_none());
        let stored = crate::load_session_record(&context, "retitle-write-failure").unwrap();
        let stored =
            serde_json::to_value(crate::retitle::latest_attempt_observation(&stored).unwrap())
                .unwrap();
        assert!(stored.get("terminal_outcome").is_none());
        assert_eq!(stored["attempts"][0]["stage"], "admission");
    }

    #[test]
    fn retitle_success_duration_includes_the_durable_commit_write() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        seed_session_with_runtime(
            tmp.path(),
            "retitle-commit-duration",
            "codex",
            "hs-retitle-commit-duration",
        );
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let request = crate::retitle::RetitleRequest {
            schema_version: crate::retitle::REQUEST_SCHEMA.into(),
            trigger: crate::retitle::RetitleTrigger::Manual,
            idempotency_key: "manual-commit-duration".into(),
            expected_session_incarnation: "launch-retitle-commit-duration".into(),
            expected_title_revision: 0,
            expected_activity_revision: None,
            expected_provider_turn_id: None,
        };
        assert!(matches!(
            crate::retitle::admit(&context, "retitle-commit-duration", &request).unwrap(),
            crate::retitle::Admission::Evaluate(_)
        ));
        let desired: SessionTitleState = serde_json::from_value(json!({
            "topic": "Durable commit timing",
            "topic_source": "auto",
            "references": [],
            "activity": null
        }))
        .unwrap();
        let started = Instant::now();
        crate::delay_next_session_record_write(Duration::from_millis(275));

        let committed = crate::retitle::commit(
            &context,
            "retitle-commit-duration",
            &request,
            desired,
            &crate::retitle::CoverageView {
                source: "provider_transcript",
                complete: true,
                truncated: false,
                turn_count: 1,
            },
            "command",
            &crate::retitle::RetitleAttemptEvidence {
                context: None,
                providers: Vec::new(),
                elapsed: Duration::ZERO,
                started: Some(started),
            },
        )
        .unwrap();

        let observation = serde_json::to_value(committed.observation.unwrap()).unwrap();
        assert_eq!(observation["attempts"][0]["duration_bucket"], "250_999_ms");
        let stored = crate::load_session_record(&context, "retitle-commit-duration").unwrap();
        assert_eq!(
            serde_json::to_value(crate::retitle::latest_attempt_observation(&stored).unwrap())
                .unwrap(),
            observation
        );
    }

    #[tokio::test]
    async fn retitle_terminal_success_is_not_returned_when_observation_persistence_fails() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let serve_state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));

        for (id, trigger) in [
            (
                "manual-terminal-write-failure",
                crate::retitle::RetitleTrigger::Manual,
            ),
            (
                "automatic-terminal-write-failure",
                crate::retitle::RetitleTrigger::Initial,
            ),
        ] {
            seed_session_with_runtime(tmp.path(), id, "codex", &format!("hs-{id}"));
            let automatic = trigger.is_automatic();
            if automatic {
                write_automatic_activity(tmp.path(), id, 1, Some("turn-a"), None);
            }
            let request = crate::retitle::RetitleRequest {
                schema_version: crate::retitle::REQUEST_SCHEMA.into(),
                trigger,
                idempotency_key: format!("{id}-key"),
                expected_session_incarnation: format!("launch-{id}"),
                expected_title_revision: 0,
                expected_activity_revision: automatic.then_some(1),
                expected_provider_turn_id: automatic.then(|| "turn-a".to_string()),
            };
            assert!(matches!(
                crate::retitle::admit(&context, id, &request).unwrap(),
                crate::retitle::Admission::Evaluate(_)
            ));
            let desired: SessionTitleState = serde_json::from_value(json!({
                "topic": "Terminal write failure",
                "topic_source": "auto",
                "references": [],
                "activity": null
            }))
            .unwrap();
            let started = Instant::now();
            crate::fail_session_record_write_on_nth_call(2);
            let error = match crate::retitle::commit(
                &context,
                id,
                &request,
                desired,
                &crate::retitle::CoverageView {
                    source: "provider_transcript",
                    complete: true,
                    truncated: false,
                    turn_count: 1,
                },
                "command",
                &crate::retitle::RetitleAttemptEvidence {
                    context: None,
                    providers: Vec::new(),
                    elapsed: Duration::ZERO,
                    started: Some(started),
                },
            ) {
                Ok(_) => panic!("terminal observation write should fail"),
                Err(error) => error,
            };
            let failure = RetitleEvaluationFailure {
                error,
                failure_stage: "commit",
                context: None,
                providers: Vec::new(),
                started,
            };

            assert!(
                record_retitle_failure(&serve_state, id, &request, &failure)
                    .await
                    .is_none()
            );
            assert!(matches!(
                crate::retitle::admit(&context, id, &request).unwrap(),
                crate::retitle::Admission::Replay(..)
            ));
            let stored = crate::load_session_record(&context, id).unwrap();
            assert_eq!(stored.title.as_deref(), Some("Terminal write failure"));
            assert!(crate::retitle::latest_attempt_observation(&stored).is_none());
            let stored = serde_json::to_value(stored).unwrap();
            assert_eq!(
                stored["session_retitle_v2"]["receipts"][0]["state"],
                "complete"
            );
            assert!(
                stored["session_retitle_v2"]["receipts"][0]
                    .get("observation")
                    .is_none()
            );
        }
    }

    #[test]
    fn retitle_two_phase_commit_does_not_rewrite_resume_sidecar() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        seed_session_with_runtime(tmp.path(), "retitle-write-count", "codex", "hs-write-count");
        let record_path = tmp.path().join("sessions/retitle-write-count/session.json");
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["provider_resume"] = json!({
            "provider":"codex",
            "session_id":"provider-write-count",
            "captured_at":"2026-09-09T00:00:00Z",
            "capture_method":"fixture",
            "resume_args":[]
        });
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let request = crate::retitle::RetitleRequest {
            schema_version: crate::retitle::REQUEST_SCHEMA.into(),
            trigger: crate::retitle::RetitleTrigger::Manual,
            idempotency_key: "manual-write-count".into(),
            expected_session_incarnation: "launch-retitle-write-count".into(),
            expected_title_revision: 0,
            expected_activity_revision: None,
            expected_provider_turn_id: None,
        };
        assert!(matches!(
            crate::retitle::admit(&context, "retitle-write-count", &request).unwrap(),
            crate::retitle::Admission::Evaluate(_)
        ));
        crate::reset_session_record_write_counts();

        crate::retitle::commit(
            &context,
            "retitle-write-count",
            &request,
            serde_json::from_value(json!({
                "topic": "No duplicate sidecar write",
                "topic_source": "auto",
                "references": [],
                "activity": null
            }))
            .unwrap(),
            &crate::retitle::CoverageView {
                source: "provider_transcript",
                complete: true,
                truncated: false,
                turn_count: 1,
            },
            "command",
            &crate::retitle::RetitleAttemptEvidence {
                context: None,
                providers: Vec::new(),
                elapsed: Duration::ZERO,
                started: Some(Instant::now()),
            },
        )
        .unwrap();

        assert_eq!(crate::session_record_write_counts(), (2, 0));
    }

    #[tokio::test]
    async fn stale_manual_retitle_fails_before_primary_or_fallback_invocation() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let primary_calls = tmp.path().join("primary.calls");
        let fallback_calls = tmp.path().join("fallback.calls");
        let primary = executable(
            &tmp.path().join("primary-provider"),
            &format!(
                "#!/bin/sh\nprintf x >> {}\nprintf '%s\\n' '{{\"topic_action\":\"set\",\"topic\":\"Primary\",\"activity\":null,\"references\":[]}}'\n",
                shell_words::quote(&primary_calls.to_string_lossy())
            ),
        );
        let fallback = executable(
            &tmp.path().join("fallback-provider"),
            &format!(
                "#!/bin/sh\nprintf x >> {}\nprintf '%s\\n' '{{\"topic_action\":\"set\",\"topic\":\"Fallback\",\"activity\":null,\"references\":[]}}'\n",
                shell_words::quote(&fallback_calls.to_string_lossy())
            ),
        );
        let config = json!({
            "provider":"command",
            "argv":[primary],
            "timeout_ms":1000,
            "fallback":{"provider":"command","argv":[fallback],"timeout_ms":1000}
        })
        .to_string();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", &config);
        seed_session_with_runtime(tmp.path(), "retitle-fence", "codex", "hs-retitle-fence");
        let app = router(state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path())));

        let (status, body) = call(
            app,
            post_json(
                "/sessions/retitle-fence/retitle",
                Some(TOKEN),
                json!({
                    "schema_version":"agent-session.session-retitle.request.v2",
                    "trigger":"manual",
                    "idempotency_key":"manual-retitle-stale-fence",
                    "expected_session_incarnation":"launch-retitle-fence",
                    "expected_title_revision":99
                }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT, "body={body}");
        assert_eq!(body["error"]["code"], "title-revision-conflict");
        assert!(!primary_calls.exists());
        assert!(!fallback_calls.exists());
    }

    #[tokio::test]
    async fn automatic_retitle_commit_accepts_same_turn_progress_and_rejects_a_newer_turn() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let provider = executable(
            &tmp.path().join("retitle-provider"),
            &format!(
                "#!/bin/sh\nstate_dir={}\ncalls=\"$state_dir/provider.calls\"\ncount=0\n[ ! -f \"$calls\" ] || count=$(cat \"$calls\")\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"$calls\"\n: > \"$state_dir/provider-started-$count\"\nwhile [ ! -f \"$state_dir/provider-release-$count\" ]; do sleep 0.01; done\nprintf '%s\\n' '{{\"topic_action\":\"set\",\"topic\":\"Automatic title\",\"activity\":null,\"references\":[]}}'\n",
                shell_words::quote(&tmp.path().to_string_lossy())
            ),
        );
        let config = json!({
            "provider":"command",
            "argv":[provider],
            "timeout_ms":5000,
            "context":{"max_chars":4000,"per_message_chars":1000,"recent_turns":8}
        })
        .to_string();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", &config);
        let codex_home = tmp.path().join("codex-home");
        let transcript_dir = codex_home.join("sessions/2026/09/07");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        std::fs::write(
            transcript_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-09-07T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"provider-automatic-fence\",\"cwd\":\"/tmp\",\"source\":\"cli\",\"timestamp\":\"2026-09-07T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-09-07T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Keep the original objective\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-a\",\"content_item_kinds\":[\"user.text\"]}}}\n"
            ),
        )
        .unwrap();
        let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_string_lossy().as_ref());
        let tmux = minimal_tmux(tmp.path());

        let seed_automatic = |id: &str, turn_id: &str, existing_title: bool| {
            seed_session_with_runtime(tmp.path(), id, "codex", &format!("hs-{id}"));
            let record_path = tmp.path().join(format!("sessions/{id}/session.json"));
            let mut record: Value =
                serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
            record["provider_resume"] = json!({
                "provider":"codex",
                "session_id":"provider-automatic-fence",
                "captured_at":"2026-09-07T00:00:02Z",
                "capture_method":"fixture",
                "resume_args":[]
            });
            if existing_title {
                record["title"] = json!("Existing title");
                record["title_revision"] = json!(1);
            }
            std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
            write_automatic_activity(tmp.path(), id, 1, Some(turn_id), None);
        };
        let automatic_request = |id: &str| {
            let context = CliContext {
                state_dir: tmp.path().to_path_buf(),
                host: None,
            };
            let record = crate::load_session_record(&context, id).unwrap();
            let view = crate::session_view(&context, &record, Some("running".into()), Some(&tmux));
            serde_json::to_value(automatic_retitle_candidate(&view).unwrap().request).unwrap()
        };

        seed_automatic("same-turn", "turn-a", false);
        let app = router(state(tmp.path(), Some(TOKEN), tmux.clone()));
        let same_turn_request = automatic_request("same-turn");
        let same_turn = tokio::spawn(call(
            app.clone(),
            post_json(
                "/sessions/same-turn/retitle-v3",
                Some(TOKEN),
                same_turn_request.clone(),
            ),
        ));
        wait_for_test_path(&tmp.path().join("provider-started-1")).await;
        std::fs::OpenOptions::new()
            .append(true)
            .open(transcript_dir.join("rollout.jsonl"))
            .unwrap()
            .write_all(
                b"{\"timestamp\":\"2026-09-07T00:00:02Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Running focused tests\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-a\",\"content_item_kinds\":[\"assistant.text\"]}}}\n",
            )
            .unwrap();
        write_automatic_activity(tmp.path(), "same-turn", 2, Some("turn-a"), None);
        std::fs::write(tmp.path().join("provider-release-1"), "release").unwrap();
        let (status, accepted) = same_turn.await.unwrap();
        assert_eq!(status, StatusCode::ACCEPTED, "body={accepted}");
        let operation_hash = accepted["data"]["retitle"]["operation_hash"]
            .as_str()
            .unwrap();
        let body = wait_for_retitle_v3_operation(&app, "same-turn", operation_hash).await;
        assert_eq!(body["data"]["retitle"]["title"], "Automatic title");
        assert_ne!(
            body["data"]["retitle"]["title"],
            "objective: keep · original · objective"
        );
        let automatic_attempts = body["data"]["retitle"]["provider_attempts"].clone();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let (status, replayed) = call(
            app.clone(),
            post_json(
                "/sessions/same-turn/retitle-v3",
                Some(TOKEN),
                same_turn_request,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body={replayed}");
        assert_eq!(replayed["data"]["retitle"]["outcome"], "committed");
        assert_eq!(
            replayed["data"]["retitle"]["provider_attempts"],
            automatic_attempts
        );

        seed_automatic("newer-turn", "turn-a", true);
        let newer_turn = tokio::spawn(call(
            app.clone(),
            post_json(
                "/sessions/newer-turn/retitle-v3",
                Some(TOKEN),
                automatic_request("newer-turn"),
            ),
        ));
        wait_for_test_path(&tmp.path().join("provider-started-2")).await;
        write_automatic_activity(tmp.path(), "newer-turn", 2, Some("turn-b"), Some("turn-a"));
        std::fs::write(tmp.path().join("provider-release-2"), "release").unwrap();
        let (status, accepted) = newer_turn.await.unwrap();
        assert_eq!(status, StatusCode::ACCEPTED, "body={accepted}");
        let operation_hash = accepted["data"]["retitle"]["operation_hash"]
            .as_str()
            .unwrap();
        let body = wait_for_retitle_v3_operation(&app, "newer-turn", operation_hash).await;
        assert_eq!(body["data"]["retitle"]["outcome"], "degraded_cached");
        assert_eq!(
            body["data"]["retitle"]["failure_class"],
            "retitle-v3-turn-conflict"
        );
        let record = crate::load_session_record(&context, "newer-turn").unwrap();
        assert_eq!(record.title.as_deref(), Some("Existing title"));
        assert_eq!(record.title_revision, 1);

        seed_automatic("older-revision", "turn-a", true);
        let older_revision = tokio::spawn(call(
            app.clone(),
            post_json(
                "/sessions/older-revision/retitle-v3",
                Some(TOKEN),
                automatic_request("older-revision"),
            ),
        ));
        wait_for_test_path(&tmp.path().join("provider-started-3")).await;
        write_automatic_activity(tmp.path(), "older-revision", 0, Some("turn-a"), None);
        std::fs::write(tmp.path().join("provider-release-3"), "release").unwrap();
        let (status, accepted) = older_revision.await.unwrap();
        assert_eq!(status, StatusCode::ACCEPTED, "body={accepted}");
        let operation_hash = accepted["data"]["retitle"]["operation_hash"]
            .as_str()
            .unwrap();
        let body = wait_for_retitle_v3_operation(&app, "older-revision", operation_hash).await;
        assert_eq!(body["data"]["retitle"]["outcome"], "degraded_cached");
        assert_eq!(
            body["data"]["retitle"]["failure_class"],
            "retitle-v3-turn-conflict"
        );
        let record = crate::load_session_record(&context, "older-revision").unwrap();
        assert_eq!(record.title.as_deref(), Some("Existing title"));
        assert_eq!(record.title_revision, 1);
    }

    #[tokio::test]
    async fn retitle_v3_receipts_are_listed_read_only_authenticated_and_content_free() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let provider = executable(
            &tmp.path().join("receipts-provider"),
            "#!/bin/sh\nprintf '%s\\n' '{\"topic_action\":\"set\",\"topic\":\"Fix automatic session titles\",\"activity\":null,\"references\":[]}'\n",
        );
        let config = json!({
            "provider":"command",
            "argv":[provider],
            "timeout_ms":1000,
            "context":{"max_chars":4000,"per_message_chars":1000,"recent_turns":8}
        })
        .to_string();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", &config);
        let codex_home = tmp.path().join("codex-home");
        let transcript_dir = codex_home.join("sessions/2026/09/10");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        std::fs::write(
            transcript_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-09-10T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"provider-receipts\",\"cwd\":\"/tmp\",\"source\":\"cli\",\"timestamp\":\"2026-09-10T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-09-10T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Fix automatic session titles\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-receipts\",\"content_item_kinds\":[\"user.text\"]}}}\n"
            ),
        )
        .unwrap();
        let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_string_lossy().as_ref());
        let id = "retitle-receipts";
        let tmux = minimal_tmux(tmp.path());
        seed_session_with_runtime(tmp.path(), id, "codex", &format!("hs-{id}"));
        let record_path = tmp.path().join(format!("sessions/{id}/session.json"));
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["provider_resume"] = json!({
            "provider":"codex",
            "session_id":"provider-receipts",
            "captured_at":"2026-09-10T00:00:02Z",
            "capture_method":"fixture",
            "resume_args":[]
        });
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        write_automatic_activity(tmp.path(), id, 1, Some("turn-receipts"), None);
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let view = crate::session_view(
            &context,
            &crate::load_session_record(&context, id).unwrap(),
            Some("running".into()),
            Some(&tmux),
        );
        let request = automatic_retitle_candidate(&view).unwrap().request;
        let operation_hash = crate::retitle_v3::request_operation_hash(id, &request);
        let app = router(state(tmp.path(), Some(TOKEN), tmux));

        let receipts_path = format!("/sessions/{id}/retitle-v3/operations");
        let (status, denied) = call(app.clone(), get_auth(&receipts_path, None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "body={denied}");

        let (status, empty) = call(app.clone(), get_auth(&receipts_path, Some(TOKEN))).await;
        assert_eq!(status, StatusCode::OK, "body={empty}");
        assert_eq!(
            empty["data"]["retitle"]["schema_version"],
            "agent-session.session-retitle.receipts.v3"
        );
        assert_eq!(
            empty["data"]["retitle"]["receipts"]
                .as_array()
                .unwrap()
                .len(),
            0
        );

        let (status, accepted) = call(
            app.clone(),
            post_json(
                &format!("/sessions/{id}/retitle-v3"),
                Some(TOKEN),
                serde_json::to_value(request).unwrap(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "body={accepted}");
        let completed = wait_for_retitle_v3_operation(&app, id, &operation_hash).await;
        assert_eq!(completed["data"]["retitle"]["outcome"], "committed");

        let (status, listed) = call(app.clone(), get_auth(&receipts_path, Some(TOKEN))).await;
        assert_eq!(status, StatusCode::OK, "body={listed}");
        let retitle = &listed["data"]["retitle"];
        assert_eq!(retitle["capability"], "agent-session.session-retitle.v3");
        assert_eq!(retitle["max_receipts"], 8);
        let receipts = retitle["receipts"].as_array().unwrap();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0]["operation_hash"], operation_hash);
        assert_eq!(receipts[0]["trigger"], "automatic");
        assert_eq!(receipts[0]["outcome"], "committed");
        assert_eq!(receipts[0]["terminal"], true);
        assert_eq!(receipts[0]["result_is_current"], true);
        assert_eq!(receipts[0]["execution_claim_held"], false);
        assert_eq!(
            receipts[0]["result_fence"]["title_revision"],
            retitle["current_fence"]["title_revision"]
        );
        // The receipt list is the diagnostic surface a multi-principal console
        // may read, so the committed title must not travel with it even though
        // the single-operation response carries one.
        let rendered = serde_json::to_string(&listed).unwrap();
        assert!(
            !rendered.contains("Fix automatic session titles"),
            "receipt list leaked session content: {rendered}"
        );
    }

    #[tokio::test]
    async fn automatic_initial_retitle_retries_one_invalid_provider_result_in_place() {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let calls = tmp.path().join("provider.calls");
        let provider = executable(
            &tmp.path().join("retry-initial-provider"),
            &format!(
                "#!/bin/sh\ncalls={}\ncount=0\n[ ! -f \"$calls\" ] || count=$(cat \"$calls\")\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"$calls\"\nif [ \"$count\" -eq 1 ]; then\n  printf '%s\\n' '{{\"topic_action\":\"set\",\"topic\":\"objective: automatic · title · bug\",\"activity\":null,\"references\":[]}}'\nelse\n  printf '%s\\n' '{{\"topic_action\":\"set\",\"topic\":\"Fix automatic session titles\",\"activity\":null,\"references\":[]}}'\nfi\n",
                shell_words::quote(&calls.to_string_lossy())
            ),
        );
        let config = json!({
            "provider":"command",
            "argv":[provider],
            "timeout_ms":1000,
            "context":{"max_chars":4000,"per_message_chars":1000,"recent_turns":8}
        })
        .to_string();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", &config);
        let codex_home = tmp.path().join("codex-home");
        let transcript_dir = codex_home.join("sessions/2026/09/10");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        std::fs::write(
            transcript_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-09-10T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"provider-initial-retry\",\"cwd\":\"/tmp\",\"source\":\"cli\",\"timestamp\":\"2026-09-10T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-09-10T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Fix automatic session titles\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-retry\",\"content_item_kinds\":[\"user.text\"]}}}\n"
            ),
        )
        .unwrap();
        let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_string_lossy().as_ref());
        let id = "automatic-initial-retry";
        let tmux = minimal_tmux(tmp.path());
        seed_session_with_runtime(tmp.path(), id, "codex", &format!("hs-{id}"));
        let record_path = tmp.path().join(format!("sessions/{id}/session.json"));
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["provider_resume"] = json!({
            "provider":"codex",
            "session_id":"provider-initial-retry",
            "captured_at":"2026-09-10T00:00:02Z",
            "capture_method":"fixture",
            "resume_args":[]
        });
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        write_automatic_activity(tmp.path(), id, 1, Some("turn-retry"), None);
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let view = crate::session_view(
            &context,
            &crate::load_session_record(&context, id).unwrap(),
            Some("running".into()),
            Some(&tmux),
        );
        let request = automatic_retitle_candidate(&view).unwrap().request;
        let expected_operation_hash = crate::retitle_v3::request_operation_hash(id, &request);
        let app = router(state(tmp.path(), Some(TOKEN), tmux));

        let (status, accepted) = call(
            app.clone(),
            post_json(
                &format!("/sessions/{id}/retitle-v3"),
                Some(TOKEN),
                serde_json::to_value(request).unwrap(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "body={accepted}");
        assert_eq!(
            accepted["data"]["retitle"]["operation_hash"],
            expected_operation_hash
        );
        let completed = wait_for_retitle_v3_operation(&app, id, &expected_operation_hash).await;
        assert_eq!(completed["data"]["retitle"]["outcome"], "committed");
        assert_eq!(
            completed["data"]["retitle"]["title"],
            "Fix automatic session titles"
        );
        assert_eq!(std::fs::read_to_string(calls).unwrap(), "2");
        let stored: Value = serde_json::from_slice(&std::fs::read(record_path).unwrap()).unwrap();
        assert_eq!(
            stored["session_retitle_v3"]["receipts"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn automatic_retitle_retry_completion_persists_projects_and_logs_the_full_attempt_chain()
    {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let provider = executable(
            &tmp.path().join("retry-provider"),
            &format!(
                "#!/bin/sh\ncalls={}\ncount=0\n[ ! -f \"$calls\" ] || count=$(cat \"$calls\")\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"$calls\"\n[ \"$count\" -ne 1 ] || exit 1\nprintf '%s\\n' '{{\"topic_action\":\"set\",\"topic\":\"Retry completed\",\"activity\":null,\"references\":[]}}'\n",
                shell_words::quote(&tmp.path().join("provider.calls").to_string_lossy())
            ),
        );
        let config = json!({
            "provider":"command",
            "model":"command-model-must-be-omitted",
            "argv":[provider],
            "timeout_ms":5000,
            "context":{"max_chars":4000,"per_message_chars":1000,"recent_turns":8}
        })
        .to_string();
        let _config = EnvGuard::set(&lock, "AGENT_SESSION_RETITLE_CONFIG", &config);
        let codex_home = tmp.path().join("codex-home");
        let transcript_dir = codex_home.join("sessions/2026/09/09");
        std::fs::create_dir_all(&transcript_dir).unwrap();
        std::fs::write(
            transcript_dir.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-09-09T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"provider-automatic-retry-chain\",\"cwd\":\"/tmp\",\"source\":\"cli\",\"timestamp\":\"2026-09-09T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-09-09T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Complete retry observability\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-retry\",\"content_item_kinds\":[\"user.text\"]}}}\n"
            ),
        )
        .unwrap();
        let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_string_lossy().as_ref());
        seed_session_with_runtime(
            tmp.path(),
            "automatic-retry-chain",
            "codex",
            "hs-automatic-retry-chain",
        );
        let record_path = tmp
            .path()
            .join("sessions/automatic-retry-chain/session.json");
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["provider_resume"] = json!({
            "provider":"codex",
            "session_id":"provider-automatic-retry-chain",
            "captured_at":"2026-09-09T00:00:02Z",
            "capture_method":"fixture",
            "resume_args":[]
        });
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        write_automatic_activity(
            tmp.path(),
            "automatic-retry-chain",
            1,
            Some("turn-retry"),
            None,
        );
        let request = json!({
            "schema_version":"agent-session.session-retitle.request.v2",
            "trigger":"prompt",
            "idempotency_key":"automatic-retry-chain-request",
            "expected_session_incarnation":"launch-automatic-retry-chain",
            "expected_title_revision":0,
            "expected_activity_revision":1,
            "expected_provider_turn_id":"turn-retry"
        });
        let app = router(state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path())));

        let (status, failed) = call(
            app.clone(),
            post_json(
                "/sessions/automatic-retry-chain/retitle",
                Some(TOKEN),
                request.clone(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={failed}");
        assert_eq!(failed["error"]["code"], "retitle-provider-unavailable");

        let (status, completed) = call(
            app.clone(),
            post_json(
                "/sessions/automatic-retry-chain/retitle",
                Some(TOKEN),
                request,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "body={completed}");
        let observation = &completed["data"]["retitle"]["attempt"];
        assert_eq!(observation["attempts"].as_array().unwrap().len(), 2);
        assert_eq!(observation["attempts"][0]["number"], 1);
        assert_eq!(observation["attempts"][0]["stage"], "failed");
        assert_eq!(
            observation["attempts"][0]["diagnostic_code"],
            "retitle-provider-unavailable"
        );
        assert_eq!(observation["attempts"][1]["number"], 2);
        assert_eq!(observation["attempts"][1]["stage"], "complete");
        assert_eq!(observation["terminal_outcome"], "committed");
        assert!(
            !observation
                .to_string()
                .contains("command-model-must-be-omitted")
        );

        let stored: Value = serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        assert_eq!(
            stored["session_retitle_v2"]["receipts"][0]["observation"],
            observation.clone()
        );
        let log = retitle_observation_log(&serde_json::from_value(observation.clone()).unwrap());
        assert_eq!(log["retitle"], observation.clone());
        let (status, listed) = call(app, get_auth("/sessions", Some(TOKEN))).await;
        assert_eq!(status, StatusCode::OK, "body={listed}");
        assert_eq!(
            listed["data"]["sessions"][0]["retitle_attempt"],
            observation.clone()
        );
    }

    #[test]
    fn automatic_retitle_candidates_fence_turn_phase_and_incarnation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tmux = minimal_tmux(tmp.path());
        seed_session_with_runtime(tmp.path(), "automatic", "codex", "hs-codex-automatic");
        let record_path = tmp.path().join("sessions/automatic/session.json");
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["provider_resume"] = json!({
            "provider":"codex",
            "session_id":"provider-automatic",
            "captured_at":"2026-09-07T00:00:00Z",
            "capture_method":"fixture",
            "resume_args":[]
        });
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let activity_path = tmp.path().join("sessions/automatic/activity.json");
        let write_activity = |runtime_id: &str,
                              revision: u64,
                              current: Option<&str>,
                              last: Option<&str>| {
            std::fs::write(
                &activity_path,
                serde_json::to_vec_pretty(&json!({
                    "schema_version":"agent-session.activity.v1",
                    "runtime_id":runtime_id,
                    "runtime_generation":1,
                    "state":{
                        "schema_version":"agent-session.turn-state.v1",
                        "phase":if current.is_some() { "working" } else { "waiting" },
                        "phase_changed_at":"2026-09-07T00:00:00Z",
                        "revision":revision,
                        "source":{"kind":"provider_hook","provider":"codex","confidence":"authoritative"},
                        "current_turn":current.map(|id| json!({"provider_turn_id":id,"started_at":"2026-09-07T00:00:00Z"})),
                        "last_turn":last.map(|id| json!({"provider_turn_id":id,"completed_at":"2026-09-07T00:00:01Z","outcome":"completed"}))
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        };
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let candidate = |runtime_id: &str| {
            let record = crate::load_session_record(&context, "automatic").unwrap();
            let view = crate::session_view(&context, &record, Some("running".into()), Some(&tmux));
            let candidate = automatic_retitle_candidate(&view).unwrap();
            assert_eq!(candidate.request.expected.session_incarnation, runtime_id);
            candidate
        };

        write_activity("launch-automatic", 1, Some("turn-a"), None);
        let first = candidate("launch-automatic");
        assert_eq!(first.request.trigger, "automatic");
        write_activity("launch-automatic", 2, Some("turn-a"), None);
        let progressed_same_turn = candidate("launch-automatic");
        assert_eq!(first.tracking_key, progressed_same_turn.tracking_key);
        assert_ne!(
            first.request.idempotency_key,
            progressed_same_turn.request.idempotency_key
        );
        assert_eq!(
            progressed_same_turn.request.expected.activity_revision,
            Some(2)
        );

        let mut titled_record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        titled_record["title"] = json!("Existing title");
        titled_record["title_revision"] = json!(1);
        std::fs::write(
            &record_path,
            serde_json::to_vec_pretty(&titled_record).unwrap(),
        )
        .unwrap();
        let titled_before_manual_change = candidate("launch-automatic");
        titled_record["title"] = json!("Manual title");
        titled_record["title_revision"] = json!(2);
        std::fs::write(
            &record_path,
            serde_json::to_vec_pretty(&titled_record).unwrap(),
        )
        .unwrap();
        let titled_after_manual_change = candidate("launch-automatic");
        assert_eq!(
            titled_before_manual_change.tracking_key,
            titled_after_manual_change.tracking_key
        );
        assert_ne!(
            titled_before_manual_change.request.idempotency_key,
            titled_after_manual_change.request.idempotency_key
        );

        write_activity("launch-automatic", 2, Some("turn-b"), Some("turn-a"));
        let repeated_prompt_new_turn = candidate("launch-automatic");
        assert_ne!(first.tracking_key, repeated_prompt_new_turn.tracking_key);
        assert_ne!(
            first.request.idempotency_key,
            repeated_prompt_new_turn.request.idempotency_key
        );
        assert_eq!(
            repeated_prompt_new_turn
                .request
                .expected
                .provider_turn_id
                .as_deref(),
            Some("turn-b")
        );
        write_activity("launch-automatic", 3, None, Some("turn-b"));
        let recovery = candidate("launch-automatic");
        assert_eq!(recovery.request.trigger, "automatic");
        assert_ne!(
            repeated_prompt_new_turn.request.idempotency_key,
            recovery.request.idempotency_key
        );

        let mut recreated: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        recreated["runtime"]["launch_id"] = json!("launch-automatic-v2");
        std::fs::write(&record_path, serde_json::to_vec_pretty(&recreated).unwrap()).unwrap();
        write_activity("launch-automatic-v2", 1, Some("turn-b"), None);
        let replacement = candidate("launch-automatic-v2");
        assert_ne!(
            replacement.request.idempotency_key,
            repeated_prompt_new_turn.request.idempotency_key
        );
    }

    #[test]
    fn automatic_retitle_new_revision_bypasses_a_failed_prior_receipt() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tmux = minimal_tmux(tmp.path());
        seed_session_with_runtime(tmp.path(), "automatic-retry", "codex", "hs-automatic-retry");
        let record_path = tmp.path().join("sessions/automatic-retry/session.json");
        let mut record: Value =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record["provider_resume"] = json!({
            "provider":"codex",
            "session_id":"provider-automatic-retry",
            "captured_at":"2026-09-07T00:00:00Z",
            "capture_method":"fixture",
            "resume_args":[]
        });
        std::fs::write(&record_path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let candidate = || {
            let record = crate::load_session_record(&context, "automatic-retry").unwrap();
            let view = crate::session_view(&context, &record, Some("running".into()), Some(&tmux));
            automatic_retitle_candidate(&view).unwrap()
        };

        write_automatic_activity(tmp.path(), "automatic-retry", 1, Some("turn-a"), None);
        let first = candidate();
        write_automatic_activity(tmp.path(), "automatic-retry", 2, Some("turn-a"), None);
        let retry = candidate();
        assert_eq!(first.tracking_key, retry.tracking_key);
        assert_ne!(first.request.idempotency_key, retry.request.idempotency_key);
        assert_eq!(
            crate::retitle_v3::request_operation_hash("automatic-retry", &first.request),
            crate::retitle_v3::request_operation_hash("automatic-retry", &retry.request)
        );
    }

    #[test]
    fn automatic_retitle_retry_tracker_requeues_with_backoff_then_stops() {
        let start = Instant::now();
        let mut tracker = AutomaticRetitleTracker::default();
        assert!(tracker.begin("turn-key", start));
        tracker.finish(
            "turn-key",
            Err(&crate::retitle::retitle_error(
                "retitle-provider-timeout",
                "title provider timed out",
                true,
                "retry",
                "retry_request",
            )),
            start,
        );
        assert!(!tracker.begin("turn-key", start));
        assert!(tracker.begin("turn-key", start + Duration::from_secs(1)));
        tracker.finish(
            "turn-key",
            Err(&crate::retitle::retitle_error(
                "retitle-provider-timeout",
                "title provider timed out",
                true,
                "retry",
                "retry_request",
            )),
            start + Duration::from_secs(1),
        );
        assert!(!tracker.begin("turn-key", start + Duration::from_secs(2)));
        assert!(tracker.begin("turn-key", start + Duration::from_secs(3)));
        tracker.finish(
            "turn-key",
            Err(&crate::retitle::retitle_error(
                "retitle-provider-timeout",
                "title provider timed out",
                true,
                "retry",
                "retry_request",
            )),
            start + Duration::from_secs(3),
        );
        assert!(!tracker.begin("turn-key", start + Duration::from_secs(30)));

        assert!(tracker.begin("turn-conflict", start));
        tracker.finish(
            "turn-conflict",
            Err(&crate::retitle::retitle_error(
                "retitle-turn-conflict",
                "provider turn changed before retitle",
                false,
                "refresh_session",
                "process_newest_turn",
            )),
            start,
        );
        assert!(!tracker.begin("turn-conflict", start));
        assert!(tracker.begin("turn-conflict", start + Duration::from_secs(1)));

        assert!(tracker.begin("history-unavailable", start));
        assert!(!crate::retitle::is_retryable_automatic_error_code(
            "retitle-v3-history-unavailable"
        ));
        tracker.finish(
            "history-unavailable",
            Err(&crate::retitle::retitle_error(
                "retitle-v3-history-unavailable",
                "provider history is not yet available",
                false,
                "retry",
                "retry_request",
            )),
            start,
        );
        assert!(!tracker.begin("history-unavailable", start));
        assert!(tracker.begin("history-unavailable", start + Duration::from_secs(1)));

        assert!(tracker.begin("nonretryable", start));
        tracker.finish(
            "nonretryable",
            Err(&crate::retitle::retitle_error(
                "retitle-state-conflict",
                "retitle state changed",
                false,
                "refresh_session",
                "refresh_session",
            )),
            start,
        );
        assert!(!tracker.begin("nonretryable", start + Duration::from_secs(30)));
    }

    #[test]
    fn retitle_provider_failure_stage_projection_is_table_driven_and_bounded() {
        for stage in [
            "queue",
            "context_worker",
            "context_read",
            "provider_worker",
            "provider_setup",
            "provider_call",
            "provider_response",
            "provider_parse",
            "provider_budget",
            "provider",
            "commit_worker",
            "commit",
        ] {
            assert_eq!(stable_retitle_failure_stage(stage), stage);
        }
        assert_eq!(
            stable_retitle_failure_stage("private_internal_stage"),
            "unknown"
        );

        for (input, expected) in [
            (None, "provider"),
            (Some("provider_setup"), "provider_setup"),
            (Some("provider_call"), "provider_call"),
            (Some("provider_response"), "provider_response"),
            (Some("provider_parse"), "provider_parse"),
            (Some("provider_budget"), "provider_budget"),
            (Some("private_internal_stage"), "provider"),
        ] {
            let providers = input
                .map(|stage| {
                    vec![crate::retitle::ProviderAttemptObservation {
                        provider_kind: "command".into(),
                        model_label: None,
                        outcome: "failed".into(),
                        failure_class: Some("failed".into()),
                        failure_stage: Some(stage.into()),
                        started_at: "2026-09-09T00:00:00Z".into(),
                        finished_at: "2026-09-09T00:00:00Z".into(),
                        duration_bucket: "under_10_ms".into(),
                    }]
                })
                .unwrap_or_default();
            assert_eq!(observed_provider_failure_stage(&providers), expected);
        }
    }
}
