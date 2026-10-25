#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_list_reports_explicit_last_prompt_state_during_recovery() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let codex_home = tmp.path().join("codex-home");
    let transcript = codex_home.join("sessions/2026/07/session.jsonl");
    std::fs::create_dir_all(transcript.parent().expect("transcript parent"))
        .expect("transcript dir");
    let provider_id = "list-background-recovery";
    let metadata = format!(
        "{}\n",
        json!({
            "type":"session_meta",
            "payload":{
                "id":provider_id,
                "session_id":provider_id,
                "cwd":"/tmp",
                "source":"cli",
                "timestamp":"2099-01-01T00:00:00Z"
            }
        })
    );
    let prompt_line = |text: &str| {
        format!(
            "{}\n",
            json!({
                "type":"event_msg",
                "payload":{"type":"user_message","message":text}
            })
        )
    };
    let assistant_line = |text: &str| {
        format!(
            "{}\n",
            json!({
                "type":"event_msg",
                "payload":{"type":"agent_message","message":text}
            })
        )
    };
    std::fs::write(
        &transcript,
        format!("{metadata}{}", prompt_line("cold prompt")),
    )
    .expect("transcript");
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());
    seed_session_with_runtime(&state_dir, "prompt-list", "codex", "hs-prompt-list");
    let record_path = state_dir.join("sessions/prompt-list/session.json");
    let mut record: Value =
        serde_json::from_slice(&std::fs::read(&record_path).expect("session record"))
            .expect("session json");
    record["provider_resume"] = json!({
        "provider":"codex",
        "session_id":provider_id,
        "captured_at":"2099-01-01T00:00:00Z",
        "capture_method":"codex-explicit-session-id",
        "resume_args":["resume", provider_id]
    });
    std::fs::write(
        &record_path,
        serde_json::to_vec_pretty(&record).expect("session json"),
    )
    .expect("session record");
    let tmux_calls = tmp.path().join("tmux.calls");
    let tmux = failing_delete_tmux(tmp.path(), &state_dir, "prompt-list", 77, &tmux_calls);
    let state = state(&state_dir, Some(TOKEN), tmux);
    let persisted =
        load_session_record(&state.context, "prompt-list").expect("persisted prompt-list session");
    assert!(
        state
            .provider_prompt_discovery
            .resolve_source(&persisted)
            .await
            .is_some(),
        "the HTTP fixture must provide one exact transcript identity"
    );

    let (status, first) = call(router(state.clone()), get("/sessions")).await;
    assert_eq!(status, StatusCode::OK, "body={first}");
    assert_eq!(
        first["data"]["sessions"][0]["status"], "running",
        "the fixture must enter prompt enrichment: body={first}"
    );
    assert!(
        first["data"]["sessions"][0]
            .as_object()
            .is_some_and(|session| !session.contains_key("last_prompt")),
        "the first response must not wait for or expose partial cold recovery"
    );
    assert_eq!(
        first["data"]["sessions"][0]["last_prompt_state"], "pending",
        "cold recovery must be distinguishable from an authoritative empty prompt"
    );
    let initial_continuity = first["data"]["sessions"][0]["last_prompt_continuity"]
        .as_str()
        .expect("eligible prompt state must include an opaque continuity token")
        .to_string();
    let cold = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_, body) = call(router(state.clone()), get("/sessions")).await;
            if let Some(text) = body["data"]["sessions"][0]["last_prompt"]["text"].as_str() {
                assert_eq!(
                    body["data"]["sessions"][0]["last_prompt_state"], "current",
                    "a caught-up preview must be marked current"
                );
                break text.to_string();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cold prompt warmup");
    assert_eq!(cold, "cold prompt");

    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("append transcript")
        .write_all(assistant_line("output without a new user prompt").as_bytes())
        .expect("append output-only record");
    let (_, output_catching_up) = call(router(state.clone()), get("/sessions")).await;
    assert_eq!(
        output_catching_up["data"]["sessions"][0]["last_prompt_state"],
        "current"
    );
    assert_eq!(
        output_catching_up["data"]["sessions"][0]["last_message"]["role"],
        "assistant"
    );
    assert_eq!(
        output_catching_up["data"]["sessions"][0]["last_message"]["text"],
        "output without a new user prompt"
    );
    let unchanged = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_, body) = call(router(state.clone()), get("/sessions")).await;
            if body["data"]["sessions"][0]["last_prompt_state"] == "current" {
                break body["data"]["sessions"][0]["last_prompt"]["text"]
                    .as_str()
                    .map(ToString::to_string);
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("output-only catch-up");
    assert_eq!(unchanged.as_deref(), Some("cold prompt"));

    for text in ["mid-turn request", "later request"] {
        OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap()
            .write_all(prompt_line(text).as_bytes())
            .unwrap();
        let (_, body) = call(router(state.clone()), get("/sessions")).await;
        let session = &body["data"]["sessions"][0];
        assert_eq!(body["data"]["capabilities"]["last_message"], true);
        assert_eq!(session["last_prompt_state"], "current");
        assert_eq!(session["last_prompt"]["text"], text);
        assert_eq!(session["last_message"]["text"], text);
        assert_eq!(session["last_message"]["role"], "user");
    }

    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("append transcript")
        .write_all(
            format!(
                "{}\n{}",
                " ".repeat(1024 * 1024),
                prompt_line("appended prompt")
            )
            .as_bytes(),
        )
        .expect("append backlog and prompt");
    let (_, catching_up) = call(router(state.clone()), get("/sessions")).await;
    assert!(
        catching_up["data"]["sessions"][0]
            .as_object()
            .is_some_and(|session| !session.contains_key("last_prompt")),
        "known unread backlog must omit rather than expose the stale cached prompt"
    );
    assert_eq!(
        catching_up["data"]["sessions"][0]["last_prompt_state"], "pending",
        "known append backlog must preserve the explicit pending state"
    );
    let appended = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_, body) = call(router(state.clone()), get("/sessions")).await;
            if let Some(text) = body["data"]["sessions"][0]["last_prompt"]["text"].as_str() {
                assert_eq!(
                    body["data"]["sessions"][0]["last_prompt_state"], "current",
                    "the refreshed preview must return to current"
                );
                break text.to_string();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("append catch-up");
    assert_eq!(appended, "appended prompt");

    let recovery_hold = state
        .provider_prompt_discovery
        .recovery_permits
        .clone()
        .acquire_many_owned(PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS as u32)
        .await
        .expect("hold recovery permits");
    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("append transcript")
        .write_all(
            format!(
                "{}\n{}",
                " ".repeat(1024 * 1024),
                assistant_line("output before replacement")
            )
            .as_bytes(),
        )
        .expect("append pre-replacement output");
    let (_, replacement_pending) = call(router(state.clone()), get("/sessions")).await;
    assert_eq!(
        replacement_pending["data"]["sessions"][0]["last_prompt_state"], "pending",
        "the controlled background catch-up must start pending"
    );
    std::fs::write(&transcript, &metadata).expect("replace transcript without a prompt");
    drop(recovery_hold);
    let key = ProviderPromptDiscoveryKey::from_record(&persisted).expect("discovery key");
    let slot = state
        .provider_prompt_discovery
        .entries
        .lock()
        .await
        .get(&key)
        .cloned()
        .expect("discovery slot");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if slot.lock().await.last_prompt_invalidated {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("background invalidation");
    let rediscovery_hold = state
        .provider_prompt_discovery
        .recovery_permits
        .clone()
        .acquire_many_owned(PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS as u32)
        .await
        .expect("hold rediscovery permits");
    let (_, unavailable) = call(router(state.clone()), get("/sessions")).await;
    assert_eq!(
        unavailable["data"]["sessions"][0]["last_prompt_state"], "unavailable",
        "continuity loss must invalidate the old preview before rediscovery"
    );
    let invalidated_continuity = unavailable["data"]["sessions"][0]["last_prompt_continuity"]
        .as_str()
        .expect("unavailable projection must retain the opaque fence");
    assert_ne!(
        invalidated_continuity, initial_continuity,
        "continuity loss must rotate the consumer retention fence"
    );
    assert!(
        unavailable["data"]["sessions"][0]
            .as_object()
            .is_some_and(|session| !session.contains_key("last_prompt")
                && !session.contains_key("last_message")),
        "an unavailable source must not expose the old prompt"
    );
    let (_, still_unavailable) = call(router(state.clone()), get("/sessions")).await;
    assert_eq!(
        still_unavailable["data"]["sessions"][0]["last_prompt_state"], "unavailable",
        "every caller must see unavailable until authoritative recovery completes"
    );
    drop(rediscovery_hold);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let state = slot.lock().await;
            if !state.last_prompt_in_flight && state.last_prompt_invalidated {
                break;
            }
            drop(state);
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("background recovery before an authoritative response");
    OpenOptions::new()
        .append(true)
        .open(&transcript)
        .expect("append transcript")
        .write_all(assistant_line("output after unobserved recovery").as_bytes())
        .expect("append after unobserved recovery");
    let (_, appended_before_current_response) = call(router(state.clone()), get("/sessions")).await;
    assert_eq!(
        appended_before_current_response["data"]["sessions"][0]["last_prompt_state"], "current",
        "bounded warm catch-up can authoritatively expose recovered state"
    );
    let current_empty = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_, body) = call(router(state.clone()), get("/sessions")).await;
            if body["data"]["sessions"][0]["last_prompt_state"] == "current" {
                break body;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("empty transcript recovery");
    assert!(
        current_empty["data"]["sessions"][0]
            .as_object()
            .is_some_and(|session| !session.contains_key("last_prompt")),
        "current without a prompt authoritatively represents an empty transcript"
    );
}
