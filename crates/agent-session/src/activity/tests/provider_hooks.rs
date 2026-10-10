use super::*;
use pretty_assertions::assert_eq;

#[test]
fn native_claude_clear_rebinds_resume_and_resets_completed_turn() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    let runtime_id = &created.record.runtime.as_ref().unwrap().launch_id;
    for (kind, id) in [
        (TurnEventKind::TurnStarted, "old-start"),
        (TurnEventKind::TurnCompleted, "old-complete"),
    ] {
        let mut old = event(kind, id);
        old.provider = "claude".into();
        old.runtime_id = runtime_id.clone();
        ingest_event(&context, &created.record.id, old).unwrap();
    }
    assert!(
        state_for_view(&context, &created.record)
            .unwrap()
            .last_turn
            .is_some()
    );
    let payload = serde_json::to_vec(&json!({
        "hook_event_name": "SessionStart", "source": "clear", "session_id": "fresh-session"
    }))
    .unwrap();
    ingest_provider_hook_input(
        &context,
        AgentKind::Claude,
        None,
        ProviderHookInput {
            id: &created.record.id,
            runtime_id,
            payload: &payload,
            attention_authority: None,
        },
    )
    .unwrap();
    let rebound = load_session_record(&context, &created.record.id).unwrap();
    assert_eq!(
        rebound.provider_resume.as_ref().unwrap().session_id,
        "fresh-session"
    );
    let turn = state_for_view(&context, &rebound).unwrap();
    assert_eq!(turn.phase, TurnPhase::Waiting);
    assert!(turn.current_turn.is_none());
    assert!(turn.last_turn.is_none());
}

#[test]
fn raw_stop_keeps_working_and_projects_pending_completion_evidence() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    activate_runtime(&context, &created.record).expect("activate runtime");
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let mut started = event(TurnEventKind::TurnStarted, "started");
    started.runtime_id = runtime_id.clone();
    ingest_event(&context, &created.record.id, started).expect("turn start");
    let mut stop = event(TurnEventKind::StopObserved, "raw-stop");
    stop.runtime_id = runtime_id;
    let state = ingest_event(&context, &created.record.id, stop)
        .expect("raw stop")
        .turn_state;

    assert_eq!(state.phase, TurnPhase::Working);
    assert_eq!(
        state
            .semantic_event
            .as_ref()
            .map(|event| event.kind.as_str()),
        Some("stop_observed")
    );
    assert_eq!(
        state
            .diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.reason.as_str()),
        Some("completion_evidence_pending")
    );
}

#[test]
fn confirmed_codex_system_ephemeral_hooks_are_noops_but_foreign_sessions_still_fail() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, mut created) = test_session(&tmp);
    let runtime = created.record.runtime.as_mut().expect("runtime");
    runtime.kind = crate::codex_app_server::RUNTIME_KIND.to_string();
    runtime.extra = BTreeMap::from([
        (
            crate::codex_app_server::PROTOCOL_KEY.to_string(),
            json!(crate::codex_app_server::PROTOCOL_VERSION),
        ),
        (
            crate::codex_app_server::SOCKET_KEY.to_string(),
            json!(tmp.path().join("app-server.sock")),
        ),
        (
            crate::codex_app_server::PROXY_KEY.to_string(),
            json!(tmp.path().join("proxy.sock")),
        ),
        (
            crate::codex_app_server::THREAD_HANDOFF_KEY.to_string(),
            json!(tmp.path().join("thread-handoff")),
        ),
        (
            crate::codex_app_server::THREAD_ATTACHED_KEY.to_string(),
            json!(tmp.path().join("thread-attached")),
        ),
    ]);
    let runtime_id = runtime.launch_id.clone();
    let auxiliary_session_id = concat!(
        "local:v1:",
        "bbbbbbbbbbbbbbbb",
        "bbbbbbbbbbbbbbbb",
        "bbbbbbbbbbbbbbbb",
        "bbbbbbbbbbbbbbbb"
    );
    write_session_record(&context, &created.record).expect("persist app-server runtime");
    activate_runtime(&context, &created.record).expect("activate runtime");
    mutate_session_record(&context, &created.record.id, |record| {
        record.provider_resume = None;
        Ok(())
    })
    .expect("clear primary identity to model the fresh-session hook race");
    crate::codex_app_server::register_system_ephemeral_thread(
        &context,
        &created.record,
        auxiliary_session_id,
    )
    .expect("register confirmed system-ephemeral thread");
    provider_resume_from_user_prompt_hook(
        &context,
        &created.record.id,
        AgentKind::Codex,
        &runtime_id,
        None,
        &json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": auxiliary_session_id
        }),
    )
    .expect("system-ephemeral prompt hook must not change provider resume identity");
    assert!(
        load_session_record(&context, &created.record.id)
            .expect("record after auxiliary prompt hook")
            .provider_resume
            .is_none(),
        "the auxiliary prompt hook must not capture the fresh session"
    );

    let activity_path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let before = fs::read(&activity_path).expect("activity before auxiliary hook");
    let auxiliary = normalize_provider_hook(
        AgentKind::Codex,
        None,
        &runtime_id,
        &json!({
            "hook_event_name": "Stop",
            "session_id": auxiliary_session_id,
            "turn_id": "system-ephemeral-turn"
        }),
    )
    .expect("normalize deceptive raw auxiliary hook identity")
    .expect("recognized auxiliary stop hook");
    let accepted = ingest_event(&context, &created.record.id, auxiliary)
        .expect("registered auxiliary hook must be accepted as a no-op");
    assert!(accepted.duplicate);
    assert_eq!(
        fs::read(&activity_path).expect("activity after auxiliary hook"),
        before,
        "the system-ephemeral hook must not mutate primary activity"
    );

    provider_resume_from_user_prompt_hook(
        &context,
        &created.record.id,
        AgentKind::Codex,
        &runtime_id,
        None,
        &json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "session-1"
        }),
    )
    .expect("the primary prompt hook must capture the fresh session");
    assert_eq!(
        load_session_record(&context, &created.record.id)
            .expect("record after primary prompt hook")
            .provider_resume
            .expect("primary provider resume")
            .session_id,
        "session-1"
    );
    let mut primary_binding = event(TurnEventKind::Progress, "primary-binding");
    primary_binding.runtime_id = runtime_id.clone();
    ingest_event(&context, &created.record.id, primary_binding)
        .expect("the primary activity hook must bind after the auxiliary no-op");

    let mut foreign = event(TurnEventKind::StopObserved, "foreign-stop");
    foreign.runtime_id = runtime_id.clone();
    foreign.provider_session_id = Some("foreign-thread".to_string());
    let error = ingest_event(&context, &created.record.id, foreign)
        .expect_err("an unregistered provider session must still fail closed");
    assert_eq!(error.code(), "provider-session-id-mismatch");

    fs::write(
        session_dir(&context, &created.record.id)
            .join(".codex-app-server-system-ephemeral-threads.json"),
        b"not-json",
    )
    .expect("corrupt auxiliary registry fixture");
    provider_resume_from_user_prompt_hook(
        &context,
        &created.record.id,
        AgentKind::Codex,
        &runtime_id,
        None,
        &json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "session-1"
        }),
    )
    .expect("the primary prompt hook must not read the auxiliary registry");
    let mut primary = event(TurnEventKind::Progress, "primary-progress");
    primary.runtime_id = runtime_id;
    ingest_event(&context, &created.record.id, primary)
        .expect("the primary activity hook must not read the auxiliary registry");
}

#[test]
fn claude_notification_waiting_requires_stop_and_no_reactivation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).expect("activate runtime");
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let mut stop = event(TurnEventKind::StopObserved, "claude-stop");
    stop.runtime_id = runtime_id.clone();
    stop.provider = AgentKind::Claude.as_str().to_string();
    stop.provider_turn_id = None;
    ingest_event(&context, &created.record.id, stop).expect("ingest stop");

    assert!(claude_notification_waiting(
        &context,
        &created.record,
        Duration::ZERO
    ));
    // Whole-second timestamps make a one-second debounce race the clock;
    // an hour cannot elapse during the test.
    assert!(!claude_notification_waiting(
        &context,
        &created.record,
        Duration::from_secs(3600)
    ));

    let mut reactivated = event(TurnEventKind::Progress, "claude-reactivated");
    reactivated.runtime_id = runtime_id;
    reactivated.provider = AgentKind::Claude.as_str().to_string();
    reactivated.provider_turn_id = None;
    ingest_event(&context, &created.record.id, reactivated).expect("ingest reactivation");
    assert!(!claude_notification_waiting(
        &context,
        &created.record,
        Duration::ZERO
    ));
}

/// Claude reports an idle composer only as an observed `idle_prompt`
/// completion about a minute after its last Stop. That completion replaces
/// the Stop as the last provider event, so it must keep the recipient
/// deliverable: otherwise queued guidance for a worker whose turn ended
/// minutes ago waits forever (sympoies/nils-cli#1886).
#[test]
fn claude_notification_waiting_accepts_an_idle_prompt_completion_after_stop() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).expect("activate runtime");
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let claude_event = |kind, id: &str| {
        let mut event = event(kind, id);
        event.runtime_id = runtime_id.clone();
        event.provider = AgentKind::Claude.as_str().to_string();
        event.provider_turn_id = None;
        event
    };
    ingest_event(
        &context,
        &created.record.id,
        claude_event(TurnEventKind::TurnStarted, "claude-turn"),
    )
    .expect("ingest turn start");
    ingest_event(
        &context,
        &created.record.id,
        claude_event(TurnEventKind::StopObserved, "claude-stop"),
    )
    .expect("ingest stop");
    ingest_event(
        &context,
        &created.record.id,
        claude_event(TurnEventKind::TurnCompleted, "claude-idle-prompt"),
    )
    .expect("ingest idle prompt");

    assert!(
        claude_notification_waiting(&context, &created.record, Duration::ZERO),
        "an idle_prompt completion is the idle composer Claude reports"
    );
    assert!(
        !claude_notification_waiting(&context, &created.record, Duration::from_secs(3600)),
        "the idle completion obeys the same debounce as a Stop"
    );

    ingest_event(
        &context,
        &created.record.id,
        claude_event(TurnEventKind::TurnStarted, "claude-next-turn"),
    )
    .expect("ingest next turn");
    assert!(
        !claude_notification_waiting(&context, &created.record, Duration::ZERO),
        "a newer turn reactivates the recipient"
    );
}

#[test]
fn codex_permission_hook_obeys_the_immutable_runtime_authority() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (_, created) = test_session(&tmp);
    assert_eq!(
        codex_hook_attention_disposition(&created.record, Some("hook")),
        CodexHookAttentionDisposition::Accept
    );
    assert_eq!(
        codex_hook_attention_disposition(&created.record, None),
        CodexHookAttentionDisposition::Accept
    );
    assert_eq!(
        codex_hook_attention_disposition(&created.record, Some("protocol")),
        CodexHookAttentionDisposition::Breach
    );

    let mut protocol = created.record.clone();
    let runtime = protocol.runtime.as_mut().unwrap();
    runtime.kind = crate::codex_app_server::RUNTIME_KIND.to_string();
    runtime.extra.insert(
        crate::codex_app_server::ATTENTION_AUTHORITY_KEY.to_string(),
        json!("protocol"),
    );
    assert_eq!(
        codex_hook_attention_disposition(&protocol, Some("protocol")),
        CodexHookAttentionDisposition::Suppress
    );
    for injected in [None, Some("hook"), Some("future-mode")] {
        assert_eq!(
            codex_hook_attention_disposition(&protocol, injected),
            CodexHookAttentionDisposition::Breach
        );
    }
}

#[test]
fn unhealthy_runtime_cannot_recover_until_a_new_generation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, mut created) = test_session(&tmp);
    created.record.updated_at = "2000-01-01T00:00:00Z".to_string();
    write_session_record(&context, &created.record).unwrap();
    let runtime_id = created.record.runtime.as_ref().unwrap().launch_id.clone();
    let mut started = event(TurnEventKind::TurnStarted, "start");
    started.runtime_id = runtime_id.clone();
    let working = ingest_event(&context, &created.record.id, started)
        .unwrap()
        .turn_state;
    mark_runtime_unhealthy(
        &context,
        &created.record.id,
        &runtime_id,
        "fixture_projection_loss",
    )
    .unwrap();
    let unknown = activity_status(&context, &created.record.id)
        .unwrap()
        .turn_state;
    assert_eq!(unknown.phase, TurnPhase::Unknown);
    assert!(unknown.revision > working.revision);
    assert_ne!(unknown.phase_changed_at, created.record.updated_at);
    let revision = unknown.revision;
    let changed_at = unknown.phase_changed_at.clone();

    let mut progress = event(TurnEventKind::Progress, "late-progress");
    progress.runtime_id = runtime_id.clone();
    let error = ingest_event(&context, &created.record.id, progress)
        .expect_err("same-runtime evidence cannot recover degraded activity");
    assert_eq!(error.code(), "activity-runtime-unhealthy");
    assert_eq!(
        activate_runtime(&context, &created.record).unwrap().phase,
        TurnPhase::Unknown
    );
    assert_eq!(
        (
            activity_status(&context, &created.record.id)
                .unwrap()
                .turn_state
                .revision,
            activity_status(&context, &created.record.id)
                .unwrap()
                .turn_state
                .phase_changed_at,
        ),
        (revision, changed_at)
    );

    fs::write(
        session_dir(&context, &created.record.id).join(ACTIVITY_UNHEALTHY_FILE),
        b"{malformed",
    )
    .unwrap();
    assert!(runtime_is_unhealthy(&context, &created.record));
    assert_eq!(
        activity_status(&context, &created.record.id)
            .unwrap()
            .turn_state
            .phase,
        TurnPhase::Unknown
    );
    let auto_resume = crate::auto_resume::view_for_record(&context, &created.record);
    assert_eq!(auto_resume.state, "terminal_failure");
    assert_eq!(
        auto_resume.failure_reason.as_deref(),
        Some("state_unavailable")
    );

    let mut false_healthy = unknown.clone();
    false_healthy.phase = TurnPhase::Working;
    false_healthy.source = provider_source(&event(TurnEventKind::Progress, "false-healthy"));
    fs::write(
        session_dir(&context, &created.record.id).join(ACTIVITY_UNHEALTHY_FILE),
        serde_json::to_vec_pretty(&RuntimeUnhealthyMarker {
            schema_version: "agent-session.activity-unhealthy.v1".to_string(),
            runtime_id: runtime_id.clone(),
            runtime_generation: created.record.runtime.as_ref().unwrap().generation,
            reason: "invalid_state_fixture".to_string(),
            marked_at: "2030-01-01T00:00:00Z".to_string(),
            state: Some(false_healthy),
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        activity_status(&context, &created.record.id)
            .unwrap()
            .turn_state
            .phase,
        TurnPhase::Unknown,
        "a parseable marker cannot publish a non-degraded state"
    );

    let runtime = created.record.runtime.as_mut().unwrap();
    runtime.generation += 1;
    runtime.launch_id = "runtime-next".to_string();
    runtime.started_at = "2030-01-01T00:00:00Z".to_string();
    write_session_record(&context, &created.record).unwrap();
    assert_eq!(
        activate_runtime(&context, &created.record).unwrap().phase,
        TurnPhase::Starting
    );
}

#[test]
fn exact_attention_binds_an_unidentified_open_turn_before_rejecting_mismatch() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session(&tmp);
    let runtime_id = created.record.runtime.as_ref().unwrap().launch_id.clone();

    let mut started = event(TurnEventKind::TurnStarted, "start-without-turn");
    started.runtime_id = runtime_id.clone();
    started.provider_turn_id = None;
    ingest_event(&context, &created.record.id, started).unwrap();

    let mut first = event(TurnEventKind::AttentionRequested, "attention-turn-a");
    first.runtime_id = runtime_id.clone();
    first.provider_turn_id = Some("turn-a".to_string());
    first.attention_id = Some("local:v1:attention-a".to_string());
    first.attention_kind = Some("approval".to_string());
    let bound = ingest_event(&context, &created.record.id, first)
        .unwrap()
        .turn_state;
    let bound_turn = bound
        .current_turn
        .as_ref()
        .and_then(|turn| turn.provider_turn_id.as_deref())
        .expect("first exact request must bind the open turn")
        .to_string();

    let mut second = event(TurnEventKind::AttentionRequested, "attention-turn-b");
    second.runtime_id = runtime_id;
    second.provider_turn_id = Some("turn-b".to_string());
    second.attention_id = Some("local:v1:attention-b".to_string());
    second.attention_kind = Some("approval".to_string());
    let error = ingest_event(&context, &created.record.id, second)
        .expect_err("a later exact request cannot change the bound turn");
    assert_eq!(error.code(), "provider-turn-id-mismatch");
    assert_eq!(
        activity_status(&context, &created.record.id)
            .unwrap()
            .turn_state
            .current_turn
            .and_then(|turn| turn.provider_turn_id),
        Some(bound_turn)
    );
}

#[test]
fn codex_user_prompt_hook_persists_exact_runtime_provider_identity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).expect("repo dir");
    let mut created = create_record(RecordRequest {
        context: &context,
        agent: AgentKind::Codex,
        mode: "interactive",
        coordination_mode: crate::cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        explicit_id: Some("hook-identity"),
        cwd: &cwd,
        prompt: None,
        log_file_name: None,
        provider_resume: None,
        agent_args: Vec::new(),
        agent_bin: None,
    })
    .expect("test session");
    created.release_lifecycle_lock();
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let raw = json!({
        "hook_event_name":"UserPromptSubmit",
        "session_id":"exact-codex-session",
        "turn_id":"turn-1"
    });

    provider_resume_from_user_prompt_hook(
        &context,
        &created.record.id,
        AgentKind::Codex,
        &runtime_id,
        None,
        &raw,
    )
    .expect("capture identity");
    let record = load_session_record(&context, &created.record.id).expect("session record");
    let resume = record.provider_resume.expect("provider resume");
    assert_eq!(resume.provider, "codex");
    assert_eq!(resume.session_id, "exact-codex-session");
    assert_eq!(resume.capture_method, "codex-user-prompt-submit-hook");
    assert_eq!(
        &resume.resume_args[..2],
        &["resume".to_string(), "exact-codex-session".to_string()]
    );
    assert!(resume.resume_args.iter().any(|arg| arg == "--cd"));

    let error = provider_resume_from_user_prompt_hook(
        &context,
        &created.record.id,
        AgentKind::Codex,
        "different-runtime",
        None,
        &json!({
            "hook_event_name":"UserPromptSubmit",
            "session_id":"other-session"
        }),
    )
    .expect_err("wrong runtime must fail closed");
    assert_eq!(error.code(), "provider-hook-runtime-mismatch");
    let record = load_session_record(&context, &created.record.id).expect("session record");
    assert_eq!(
        record.provider_resume.expect("provider resume").session_id,
        "exact-codex-session"
    );
}

#[test]
fn codex_user_prompt_hook_promotes_matching_heuristic_identity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).expect("repo dir");
    let mut created = create_record(RecordRequest {
        context: &context,
        agent: AgentKind::Codex,
        mode: "interactive",
        coordination_mode: crate::cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        explicit_id: Some("hook-promotes-heuristic"),
        cwd: &cwd,
        prompt: None,
        log_file_name: None,
        provider_resume: Some(ProviderResume {
            provider: "codex".to_string(),
            session_id: "exact-codex-session".to_string(),
            captured_at: "2026-07-10T00:00:00Z".to_string(),
            capture_method: "codex-session-meta".to_string(),
            resume_args: Vec::new(),
            extra: BTreeMap::new(),
        }),
        agent_args: Vec::new(),
        agent_bin: None,
    })
    .expect("test session");
    created.release_lifecycle_lock();
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();

    provider_resume_from_user_prompt_hook(
        &context,
        &created.record.id,
        AgentKind::Codex,
        &runtime_id,
        None,
        &json!({
            "hook_event_name":"UserPromptSubmit",
            "session_id":"exact-codex-session"
        }),
    )
    .expect("promote matching identity");

    let record = load_session_record(&context, &created.record.id).expect("session record");
    let resume = record.provider_resume.expect("provider resume");
    assert_eq!(resume.capture_method, "codex-user-prompt-submit-hook");
    assert_eq!(
        &resume.resume_args[..2],
        &["resume".to_string(), "exact-codex-session".to_string()]
    );
}

#[test]
fn provider_identity_and_title_updates_are_serialized_without_lost_fields() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).expect("repo dir");
    let mut created = create_record(RecordRequest {
        context: &context,
        agent: AgentKind::Codex,
        mode: "interactive",
        coordination_mode: crate::cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        explicit_id: Some("concurrent-session-mutations"),
        cwd: &cwd,
        prompt: None,
        log_file_name: None,
        provider_resume: None,
        agent_args: Vec::new(),
        agent_bin: None,
    })
    .expect("test session");
    created.release_lifecycle_lock();
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let barrier = Arc::new(Barrier::new(3));
    let hook = {
        let context = context.clone();
        let id = created.record.id.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            provider_resume_from_user_prompt_hook(
                &context,
                &id,
                AgentKind::Codex,
                &runtime_id,
                None,
                &json!({
                    "hook_event_name":"UserPromptSubmit",
                    "session_id":"concurrent-provider-session"
                }),
            )
        })
    };
    let title = {
        let context = context.clone();
        let id = created.record.id.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            crate::update_session_title(
                &context,
                &id,
                Some("Concurrent title".to_string()),
                Path::new("/bin/false"),
            )
        })
    };
    barrier.wait();
    hook.join().expect("hook thread").expect("hook update");
    title.join().expect("title thread").expect("title update");

    let record = load_session_record(&context, &created.record.id).expect("session record");
    assert_eq!(record.title.as_deref(), Some("Concurrent title"));
    assert_eq!(
        record
            .provider_resume
            .as_ref()
            .expect("provider resume")
            .session_id,
        "concurrent-provider-session"
    );
    assert!(
        session_dir(&context, &created.record.id)
            .join(crate::SESSION_RESUME_FILE)
            .is_file()
    );
}

#[test]
fn reducer_preserves_parallel_attention_until_each_correlated_request_clears() {
    let mut document = document();
    reduce(
        &mut document,
        &event(TurnEventKind::TurnStarted, "start"),
        "2026-07-10T00:00:01Z",
    );
    for (event_id, attention_id) in [("ask-1", "request-1"), ("ask-2", "request-2")] {
        let mut request = event(TurnEventKind::AttentionRequested, event_id);
        request.attention_id = Some(attention_id.to_string());
        request.attention_kind = Some("approval".to_string());
        reduce(&mut document, &request, "2026-07-10T00:00:02Z");
    }
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);
    assert_eq!(
        document
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.attention.as_ref())
            .map(|attention| attention.pending_count),
        Some(2)
    );

    reduce(
        &mut document,
        &event(TurnEventKind::Progress, "unrelated-progress"),
        "2026-07-10T00:00:03Z",
    );
    assert_eq!(
        serde_json::to_value(&document.state).expect("state json")["current_turn"]["last_progress_at"],
        "2026-07-10T00:00:03Z",
        "accepted provider progress should expose a safe monotonic timestamp"
    );
    reduce(
        &mut document,
        &event(TurnEventKind::StopObserved, "raw-stop"),
        "2026-07-10T00:00:04Z",
    );
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);

    let mut clear = event(TurnEventKind::AttentionCleared, "clear-1");
    clear.attention_id = Some("request-1".to_string());
    reduce(&mut document, &clear, "2026-07-10T00:00:05Z");
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);
    assert_eq!(
        serde_json::to_value(&document.state).expect("state json")["current_turn"]["last_progress_at"],
        "2026-07-10T00:00:05Z",
        "an exact response is both a correlated clear and safe provider progress"
    );
    assert_eq!(
        document
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.attention.as_ref())
            .map(|attention| attention.pending_count),
        Some(1)
    );

    clear.event_id = "clear-2".to_string();
    clear.attention_id = Some("request-2".to_string());
    reduce(&mut document, &clear, "2026-07-10T00:00:06Z");
    assert_eq!(document.state.phase, TurnPhase::Working);
    assert_eq!(
        document
            .state
            .current_turn
            .as_ref()
            .map(|turn| turn.started_at.as_str()),
        Some("2026-07-10T00:00:01Z"),
        "clearing attention must not reset the original turn timer"
    );

    reduce(
        &mut document,
        &event(TurnEventKind::Progress, "late-progress"),
        "2026-07-10T00:00:04Z",
    );
    assert_eq!(
        document
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.last_progress_at.as_deref()),
        Some("2026-07-10T00:00:06Z"),
        "out-of-order evidence must not regress the monotonic progress timestamp"
    );
}

#[test]
fn reducer_progress_metadata_requires_provider_evidence_and_matching_clear() {
    let mut document = document();
    reduce(
        &mut document,
        &event(TurnEventKind::TurnStarted, "start"),
        "2026-07-10T00:00:01Z",
    );

    for (index, source_kind) in [
        SourceKind::ConsoleObservation,
        SourceKind::TerminalHeuristic,
        SourceKind::Runtime,
    ]
    .into_iter()
    .enumerate()
    {
        let mut progress = event(TurnEventKind::Progress, &format!("progress-{index}"));
        progress.source_kind = source_kind;
        reduce(
            &mut document,
            &progress,
            &format!("2026-07-10T00:00:0{}Z", index + 2),
        );
    }
    assert_eq!(
        document
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.last_progress_at.as_deref()),
        None,
        "non-provider observations must not create safe progress metadata"
    );

    let mut request = event(TurnEventKind::AttentionRequested, "ask-1");
    request.attention_id = Some("request-1".to_string());
    request.attention_kind = Some("clarification".to_string());
    reduce(&mut document, &request, "2026-07-10T00:00:05Z");

    let mut non_provider_clear = event(TurnEventKind::AttentionCleared, "clear-local");
    non_provider_clear.attention_id = Some("request-1".to_string());
    non_provider_clear.source_kind = SourceKind::TerminalHeuristic;
    reduce(&mut document, &non_provider_clear, "2026-07-10T00:00:06Z");
    assert_eq!(
        document
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.last_progress_at.as_deref()),
        None,
        "a non-provider clear must not count as safe provider progress"
    );

    request.event_id = "ask-2".to_string();
    request.attention_id = Some("request-2".to_string());
    reduce(&mut document, &request, "2026-07-10T00:00:07Z");

    let mut unmatched_clear = event(TurnEventKind::AttentionCleared, "clear-unmatched");
    unmatched_clear.attention_id = Some("different-request".to_string());
    reduce(&mut document, &unmatched_clear, "2026-07-10T00:00:08Z");
    assert_eq!(
        document
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.last_progress_at.as_deref()),
        None,
        "an unmatched clear must not count as correlated provider progress"
    );

    unmatched_clear.event_id = "clear-matched".to_string();
    unmatched_clear.attention_id = Some("request-2".to_string());
    reduce(&mut document, &unmatched_clear, "2026-07-10T00:00:09Z");
    assert_eq!(
        document
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.last_progress_at.as_deref()),
        Some("2026-07-10T00:00:09Z")
    );
}

#[test]
fn reducer_bounds_attention_and_keeps_overflow_conservatively_latched() {
    let mut document = document();
    reduce(
        &mut document,
        &event(TurnEventKind::TurnStarted, "start"),
        "2026-07-10T00:00:01Z",
    );
    for index in 0..(MAX_PENDING_ATTENTION + 20) {
        let mut request = event(
            TurnEventKind::AttentionRequested,
            &format!("request-event-{index}"),
        );
        request.attention_id = Some(format!("request-{index}"));
        request.attention_kind = Some("approval".to_string());
        reduce(&mut document, &request, "2026-07-10T00:00:02Z");
    }
    assert_eq!(document.pending_attention.len(), MAX_PENDING_ATTENTION);
    assert_eq!(
        document
            .overflow_attention
            .as_ref()
            .map(|overflow| overflow.count),
        Some(20)
    );
    assert_eq!(document.state.phase, TurnPhase::NeedsInput);
    assert_eq!(
        document
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.attention.as_ref())
            .map(|attention| attention.pending_count),
        Some(MAX_PENDING_ATTENTION + 20)
    );
}

#[test]
fn reducer_ignores_late_completion_after_a_newer_turn_completed() {
    let mut document = document();
    let mut turn_a = event(TurnEventKind::TurnStarted, "a-start");
    turn_a.provider_turn_id = Some("turn-a".to_string());
    reduce(&mut document, &turn_a, "2026-07-10T00:00:01Z");
    let mut turn_b = event(TurnEventKind::TurnStarted, "b-start");
    turn_b.provider_turn_id = Some("turn-b".to_string());
    reduce(&mut document, &turn_b, "2026-07-10T00:00:02Z");
    let mut complete_b = event(TurnEventKind::TurnCompleted, "b-complete");
    complete_b.provider_turn_id = Some("turn-b".to_string());
    reduce(&mut document, &complete_b, "2026-07-10T00:00:03Z");
    let last_b = document.state.last_turn.clone();

    let mut complete_a = event(TurnEventKind::TurnFailed, "a-late");
    complete_a.provider_turn_id = Some("turn-a".to_string());
    reduce(&mut document, &complete_a, "2026-07-10T00:00:04Z");

    assert_eq!(document.state.phase, TurnPhase::Waiting);
    assert_eq!(document.state.last_turn, last_b);
}

#[test]
fn codex_authoritative_completion_requires_an_exact_open_turn() {
    let mut completion = event(TurnEventKind::TurnCompleted, "completion");
    completion.confidence = Confidence::Authoritative;
    completion.provider_turn_id = Some("turn-a".to_string());

    let mut no_open_turn = document();
    reduce(&mut no_open_turn, &completion, "2026-07-10T00:00:01Z");
    assert_eq!(no_open_turn.state.phase, TurnPhase::Starting);
    assert!(no_open_turn.state.last_turn.is_none());

    let mut id_less_turn = document();
    let mut start_without_id = event(TurnEventKind::TurnStarted, "start-without-id");
    start_without_id.provider_turn_id = None;
    reduce(&mut id_less_turn, &start_without_id, "2026-07-10T00:00:01Z");
    reduce(&mut id_less_turn, &completion, "2026-07-10T00:00:02Z");
    assert_eq!(id_less_turn.state.phase, TurnPhase::Working);
    assert!(id_less_turn.state.current_turn.is_some());

    let mut exact_turn = document();
    let mut start = event(TurnEventKind::TurnStarted, "start");
    start.provider_turn_id = Some("turn-a".to_string());
    reduce(&mut exact_turn, &start, "2026-07-10T00:00:01Z");
    reduce(&mut exact_turn, &completion, "2026-07-10T00:00:02Z");
    assert_eq!(exact_turn.state.phase, TurnPhase::Waiting);
    assert!(exact_turn.state.current_turn.is_none());
}

#[test]
fn provider_mapping_uses_only_metadata_and_conservative_finality() {
    let codex = normalize_provider_hook(
        AgentKind::Codex,
        None,
        "runtime-1",
        &json!({
            "hook_event_name": "Stop",
            "session_id": "provider-session",
            "turn_id": "provider-turn",
            "last_assistant_message": "secret output",
            "transcript_path": "/secret/transcript"
        }),
    )
    .expect("codex mapping")
    .expect("codex event");
    assert_eq!(codex.kind, TurnEventKind::StopObserved);
    let serialized = serde_json::to_string(&codex).expect("event json");
    assert!(!serialized.contains("secret output"));
    assert!(!serialized.contains("transcript"));
    assert!(!serialized.contains("provider-session"));
    assert!(!serialized.contains("provider-turn"));

    let codex_completion = normalize_provider_notification(
        AgentKind::Codex,
        "runtime-1",
        &json!({
            "type": "agent-turn-complete",
            "thread-id": "provider-session",
            "turn-id": "provider-turn",
            "cwd": "/secret/cwd",
            "input-messages": ["secret prompt"],
            "last-assistant-message": "secret output"
        }),
    )
    .expect("Codex notification mapping")
    .expect("recognized completion");
    assert_eq!(codex_completion.kind, TurnEventKind::TurnCompleted);
    assert_eq!(codex_completion.confidence, Confidence::Authoritative);
    let serialized = serde_json::to_string(&codex_completion).expect("completion event json");
    for forbidden in [
        "provider-session",
        "provider-turn",
        "/secret/cwd",
        "secret prompt",
        "secret output",
        "input-messages",
        "last-assistant-message",
    ] {
        assert!(!serialized.contains(forbidden), "forbidden {forbidden}");
    }
    assert!(
        normalize_provider_notification(
            AgentKind::Codex,
            "runtime-1",
            &json!({"type": "future-notification"}),
        )
        .expect("future notification")
        .is_none()
    );
    let missing_turn = normalize_provider_notification(
        AgentKind::Codex,
        "runtime-1",
        &json!({
            "type": "agent-turn-complete",
            "thread-id": "provider-session"
        }),
    )
    .expect_err("completion requires turn-id correlation");
    assert_eq!(missing_turn.code(), "provider-notification-turn-id-missing");

    let claude = normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({
            "hook_event_name": "Notification",
            "notification_type": "idle_prompt",
            "message": "content is discarded"
        }),
    )
    .expect("claude mapping")
    .expect("claude event");
    assert_eq!(claude.kind, TurnEventKind::TurnCompleted);
    assert_eq!(claude.confidence, Confidence::Observed);

    let dsh = normalize_provider_hook(
        AgentKind::Dsh,
        Some("post_llm_call"),
        "runtime-1",
        &json!({
            "session_id": "provider-session",
            "assistant_response": "discarded"
        }),
    )
    .expect("DSH mapping")
    .expect("DSH event");
    assert_eq!(dsh.kind, TurnEventKind::TurnCompleted);
    assert_eq!(dsh.confidence, Confidence::Authoritative);
}

#[test]
fn claude_pre_tool_use_reactivates_working_without_admitting_stale_subagent_stop() {
    let pre_tool_use = normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({
            "hook_event_name": "PreToolUse",
            "session_id": "claude-session",
            "tool_name": "Task",
            "tool_use_id": "tool-secret",
            "tool_input": {"prompt": "discarded"}
        }),
    )
    .expect("pre-tool normalization")
    .expect("recognized pre-tool signal");
    assert_eq!(pre_tool_use.kind, TurnEventKind::Progress);
    assert_eq!(pre_tool_use.confidence, Confidence::Observed);
    assert_eq!(pre_tool_use.source_kind, SourceKind::ProviderHook);
    assert_eq!(pre_tool_use.provider, "claude");

    let serialized = serde_json::to_string(&pre_tool_use).expect("continuation event json");
    for forbidden in ["tool-secret", "discarded"] {
        assert!(!serialized.contains(forbidden), "forbidden {forbidden}");
    }

    let mut document = document();
    reduce(
        &mut document,
        &event(TurnEventKind::TurnStarted, "turn-started"),
        "2026-07-18T00:00:01Z",
    );
    reduce(
        &mut document,
        &event(TurnEventKind::TurnCompleted, "idle-prompt"),
        "2026-07-18T00:00:02Z",
    );
    assert_eq!(document.state.phase, TurnPhase::Waiting);
    assert!(document.state.current_turn.is_none());

    let stale_subagent_stop = normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({
            "hook_event_name": "SubagentStop",
            "session_id": "claude-session",
            "agent_id": "subagent-secret",
            "agent_transcript_path": "/secret/transcript"
        }),
    )
    .expect("stale subagent-stop normalization");
    assert!(
        stale_subagent_stop.is_none(),
        "an uncorrelated completed subagent must not resurrect a waiting parent turn"
    );
    assert_eq!(document.state.phase, TurnPhase::Waiting);
    assert!(document.state.current_turn.is_none());

    reduce(&mut document, &pre_tool_use, "2026-07-18T00:00:03Z");
    assert_eq!(document.state.phase, TurnPhase::Working);
    assert!(document.state.current_turn.is_some());
    assert_eq!(document.state.source.kind, SourceKind::ProviderHook);
    assert_eq!(document.state.source.provider.as_deref(), Some("claude"));
    assert_eq!(document.state.source.confidence, Confidence::Observed);
}

#[test]
fn claude_bypass_permissions_prompt_is_latched() {
    // PermissionRequest and permission_prompt hooks mean Claude is showing a
    // real dialog, including the root/home deletion circuit breaker that remains
    // active in bypass mode. Preserve that attention signal.
    for payload in [
        json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "claude-session",
            "tool_name": "Bash",
            "tool_input": {"command": "rm -rf /"},
            "permission_mode": "bypassPermissions"
        }),
        json!({
            "hook_event_name": "Notification",
            "notification_type": "permission_prompt",
            "permission_mode": "bypassPermissions"
        }),
    ] {
        let mapped = normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &payload)
            .expect("bypass mapping")
            .expect("bypass prompt");
        assert_eq!(mapped.kind, TurnEventKind::AttentionRequested);
        assert_eq!(mapped.attention_kind.as_deref(), Some("approval"));
    }

    // Non-bypass modes (and a missing mode) keep the conservative approval latch.
    for mode in [Some("default"), Some("acceptEdits"), Some("plan"), None] {
        let mut payload = json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "claude-session",
            "tool_name": "Bash"
        });
        if let Some(mode) = mode {
            payload["permission_mode"] = json!(mode);
        }
        let mapped = normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &payload)
            .expect("approval mapping")
            .expect("recognized approval");
        assert_eq!(mapped.kind, TurnEventKind::AttentionRequested);
        assert_eq!(mapped.attention_kind.as_deref(), Some("approval"));
    }
}

#[test]
fn claude_ask_user_question_in_an_unannounced_turn_is_ingested() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    // Shaped as agent-hook sends Claude hooks: the turn id is the projected
    // `prompt_id`, and AskUserQuestion carries its projected `tool_use_id`.
    let claude = |kind: TurnEventKind, id: &str, prompt: &str| {
        let mut current = event(kind, id);
        current.runtime_id.clone_from(&runtime_id);
        current.provider = AgentKind::Claude.as_str().to_string();
        current.provider_turn_id = Some(format!("local:v1:{}", "a".repeat(63) + prompt));
        current
    };
    // Hooks can carry a new prompt_id whose UserPromptSubmit was never
    // recorded while the previously announced turn is still open.
    for current in [
        claude(TurnEventKind::TurnStarted, "prompt-1-start", "1"),
        claude(TurnEventKind::Progress, "prompt-1-tool", "1"),
        claude(TurnEventKind::Progress, "prompt-2-tool", "2"),
    ] {
        ingest_event(&context, &created.record.id, current).expect("turn setup");
    }
    let attention_id = format!("local:v1:{}", "b".repeat(64));
    let mut asked = claude(TurnEventKind::AttentionRequested, "prompt-2-ask", "2");
    asked.attention_kind = Some("clarification".to_string());
    asked.attention_id = Some(attention_id.clone());
    asked.attention_correlation_exact = true;
    let requested = ingest_event(&context, &created.record.id, asked)
        .expect("a clarification request in an unannounced turn is ingested");
    assert_eq!(requested.turn_state.phase, TurnPhase::NeedsInput);
    let turn_id = |prompt: &str| format!("local:v1:{}", "a".repeat(63) + prompt);
    assert_eq!(
        requested
            .turn_state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.clone()),
        Some(turn_id("2")),
        "the requesting turn is now current"
    );
    let superseded = requested.turn_state.last_turn.as_ref().expect("last turn");
    assert_eq!(superseded.provider_turn_id, Some(turn_id("1")));
    assert_eq!(superseded.outcome, "interrupted");

    // A late request from the superseded turn must not reopen it.
    let mut late = claude(TurnEventKind::AttentionRequested, "prompt-1-late-ask", "1");
    late.attention_kind = Some("clarification".to_string());
    late.attention_id = Some(format!("local:v1:{}", "c".repeat(64)));
    late.attention_correlation_exact = true;
    let ignored = ingest_event(&context, &created.record.id, late)
        .expect("a late request from the closed turn is accepted as metadata");
    assert!(ignored.duplicate);
    assert_eq!(ignored.turn_state, requested.turn_state);

    let mut cleared = claude(TurnEventKind::AttentionCleared, "prompt-2-answer", "2");
    cleared.attention_id = Some(attention_id);
    cleared.attention_correlation_exact = true;
    let answered = ingest_event(&context, &created.record.id, cleared)
        .expect("the answered clarification clears");
    assert_ne!(answered.turn_state.phase, TurnPhase::NeedsInput);
}

#[test]
fn claude_attention_after_a_completed_same_prompt_turn_needs_input() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    let turn_id = format!("local:v1:{}", "a".repeat(64));
    let claude = |kind: TurnEventKind, id: &str| {
        let mut current = event(kind, id);
        current.runtime_id.clone_from(&runtime_id);
        current.provider = AgentKind::Claude.as_str().to_string();
        current.provider_turn_id = Some(turn_id.clone());
        current
    };
    // Claude keeps stamping hooks with a prompt that `idle_prompt` already
    // closed while a background subagent is still working under it, so a
    // request for that prompt with no other turn open is live, not late.
    for current in [
        claude(TurnEventKind::TurnStarted, "prompt-start"),
        claude(TurnEventKind::TurnCompleted, "prompt-idle"),
    ] {
        ingest_event(&context, &created.record.id, current).expect("turn setup");
    }
    let mut asked = claude(TurnEventKind::AttentionRequested, "prompt-approval");
    asked.attention_kind = Some("approval".to_string());
    asked.attention_id = Some(format!("local:v1:{}", "b".repeat(64)));
    let requested = ingest_event(&context, &created.record.id, asked)
        .expect("a request under the completed prompt is ingested");
    assert!(!requested.duplicate, "the request must not be dropped");
    assert_eq!(requested.turn_state.phase, TurnPhase::NeedsInput);
    assert_eq!(
        requested
            .turn_state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.clone()),
        Some(turn_id.clone()),
        "the completed prompt is reopened by its own request"
    );

    let closed = ingest_event(
        &context,
        &created.record.id,
        claude(TurnEventKind::TurnCompleted, "prompt-idle-again"),
    )
    .expect("the reopened turn completes again");
    assert!(!closed.duplicate);
    assert_eq!(closed.turn_state.phase, TurnPhase::Waiting);
}

#[test]
fn claude_ask_user_question_uses_exact_runtime_scoped_correlation() {
    let request = normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({
            "hook_event_name": "PreToolUse",
            "session_id": "claude-session",
            "tool_name": "AskUserQuestion",
            "tool_use_id": "tool-use-secret-1",
            "tool_input": {"questions": [{"question": "discarded"}]}
        }),
    )
    .expect("request mapping")
    .expect("recognized request");
    assert_eq!(request.kind, TurnEventKind::AttentionRequested);
    assert_eq!(request.attention_kind.as_deref(), Some("clarification"));
    let correlation = request.attention_id.as_deref().expect("correlation id");
    assert!(correlation.starts_with("local:v1:"));
    assert!(!correlation.contains("tool-use-secret-1"));

    for event_name in ["PostToolUse", "PostToolUseFailure"] {
        let response = normalize_provider_hook(
            AgentKind::Claude,
            None,
            "runtime-1",
            &json!({
                "hook_event_name": event_name,
                "session_id": "claude-session",
                "tool_name": "AskUserQuestion",
                "tool_use_id": "tool-use-secret-1",
                "tool_response": {"answers": "discarded"},
                "error": "discarded"
            }),
        )
        .expect("response mapping")
        .expect("recognized response");
        assert_eq!(response.kind, TurnEventKind::AttentionCleared);
        assert_eq!(response.attention_id.as_deref(), Some(correlation));
        let serialized = serde_json::to_string(&response).expect("response json");
        assert!(!serialized.contains("tool-use-secret-1"));
        assert!(!serialized.contains("answers"));
        assert!(!serialized.contains("discarded"));
    }

    let permission = normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "claude-session",
            "tool_name": "Bash"
        }),
    )
    .expect("permission mapping")
    .expect("recognized permission");
    assert_eq!(permission.kind, TurnEventKind::AttentionRequested);
    assert_ne!(permission.attention_id.as_deref(), Some(correlation));

    let unrelated = normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({
            "hook_event_name": "PostToolUse",
            "session_id": "claude-session",
            "tool_name": "Bash",
            "tool_use_id": "other-tool"
        }),
    )
    .expect("progress mapping")
    .expect("recognized progress");
    assert_eq!(unrelated.kind, TurnEventKind::Progress);
    assert!(unrelated.attention_id.is_none());

    let missing_correlation = normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({
            "hook_event_name": "PreToolUse",
            "session_id": "claude-session",
            "tool_name": "AskUserQuestion"
        }),
    )
    .expect_err("missing correlation should surface safe drift diagnostics");
    assert_eq!(
        missing_correlation.code(),
        "provider-hook-correlation-missing"
    );
}

#[test]
fn claude_ask_user_question_shadows_do_not_pin_attention() {
    let mapped = normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({
            "hook_event_name": "PermissionRequest",
            "session_id": "claude-session",
            "tool_name": "AskUserQuestion",
            "tool_input": {"questions": [{"question": "discarded"}]}
        }),
    )
    .expect("shadow signal should be safely recognized");
    assert!(
        mapped.is_none(),
        "AskUserQuestion exact PreToolUse/PostToolUse correlation must be the sole attention occurrence"
    );
    assert!(
        !provider_specs(AgentKind::Claude).iter().any(|spec| {
            spec.event == "Notification" && spec.matcher == Some("permission_prompt")
        })
    );
}

#[test]
fn claude_elicitation_uses_exact_id_when_present_and_never_manufactures_a_clear() {
    for (mode, expected_kind) in [("form", "clarification"), ("url", "authentication")] {
        let request = normalize_provider_hook(
            AgentKind::Claude,
            None,
            "runtime-1",
            &json!({
                "hook_event_name": "Elicitation",
                "session_id": "claude-session",
                "mcp_server_name": "fixture-server",
                "mode": mode,
                "elicitation_id": "elicit-secret-1",
                "message": "must-not-leave-normalizer",
                "url": "https://must-not-leave.invalid",
                "requested_schema": {"secret": true}
            }),
        )
        .expect("request mapping")
        .expect("recognized elicitation request");
        assert_eq!(request.kind, TurnEventKind::AttentionRequested);
        assert_eq!(request.attention_kind.as_deref(), Some(expected_kind));
        let correlation = request.attention_id.as_deref().expect("exact id");
        assert!(correlation.starts_with("local:v1:"));

        let response = normalize_provider_hook(
            AgentKind::Claude,
            None,
            "runtime-1",
            &json!({
                "hook_event_name": "ElicitationResult",
                "session_id": "claude-session",
                "mcp_server_name": "fixture-server",
                "mode": mode,
                "elicitation_id": "elicit-secret-1",
                "action": "accept",
                "content": {"answer": "must-not-leave-normalizer"}
            }),
        )
        .expect("response mapping")
        .expect("recognized elicitation response");
        assert_eq!(response.kind, TurnEventKind::AttentionCleared);
        assert_eq!(response.attention_id.as_deref(), Some(correlation));

        for event in [request, response] {
            let wire = serde_json::to_string(&event).unwrap();
            for forbidden in [
                "elicit-secret-1",
                "fixture-server",
                "must-not-leave-normalizer",
                "must-not-leave.invalid",
                "answer",
            ] {
                assert!(!wire.contains(forbidden), "forbidden {forbidden}");
            }
        }
    }

    let conservative = normalize_provider_hook(
        AgentKind::Claude,
        None,
        "runtime-1",
        &json!({
            "hook_event_name": "Elicitation",
            "session_id": "claude-session",
            "mode": "form",
            "message": "identifier omitted"
        }),
    )
    .expect("missing-id request remains safe")
    .expect("missing-id request is conservatively latched");
    assert_eq!(conservative.kind, TurnEventKind::AttentionRequested);
    assert_eq!(
        conservative.attention_kind.as_deref(),
        Some("clarification")
    );
    assert!(conservative.attention_id.is_some());

    assert!(
        normalize_provider_hook(
            AgentKind::Claude,
            None,
            "runtime-1",
            &json!({
                "hook_event_name": "ElicitationResult",
                "session_id": "claude-session",
                "mode": "form",
                "action": "decline"
            }),
        )
        .expect("identifier-less result is safely ignored")
        .is_none(),
        "identifier-less results must never clear a conservative latch"
    );
}

#[test]
fn exact_codex_approval_ids_survive_semantic_deduplication() {
    let mut first = event(TurnEventKind::AttentionRequested, "first");
    first.attention_kind = Some("approval".to_string());
    first.attention_id = Some(
        "local:v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
    );
    let mut second = first.clone();
    second.event_id = "second".to_string();
    second.attention_id = Some(
        "local:v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
    );

    assert_ne!(
        semantic_event_key(&first),
        semantic_event_key(&second),
        "independent exact approvals must not collapse inside the semantic dedupe window"
    );
}

#[test]
fn frozen_provider_fixtures_replay_through_each_adapter() {
    let cases = [
        (
            AgentKind::Codex,
            include_str!("../../../tests/fixtures/activity/codex-events.jsonl"),
            vec![
                TurnEventKind::TurnStarted,
                TurnEventKind::AttentionRequested,
                TurnEventKind::Progress,
                TurnEventKind::StopObserved,
            ],
        ),
        (
            AgentKind::Claude,
            include_str!("../../../tests/fixtures/activity/claude-events.jsonl"),
            vec![
                TurnEventKind::TurnStarted,
                TurnEventKind::Progress,
                TurnEventKind::AttentionRequested,
                TurnEventKind::AttentionCleared,
                TurnEventKind::AttentionRequested,
                TurnEventKind::AttentionCleared,
                TurnEventKind::AttentionRequested,
                TurnEventKind::Progress,
                TurnEventKind::StopObserved,
                TurnEventKind::TurnCompleted,
                TurnEventKind::TurnFailed,
            ],
        ),
        (
            AgentKind::Dsh,
            include_str!("../../../tests/fixtures/activity/dsh-events.jsonl"),
            vec![TurnEventKind::TurnStarted, TurnEventKind::TurnCompleted],
        ),
    ];
    for (agent, fixture, expected) in cases {
        let normalized = fixture
            .lines()
            .map(|line| {
                let raw: Value = serde_json::from_str(line).expect("provider fixture");
                normalize_provider_hook(agent, None, "runtime-1", &raw)
                    .expect("provider fixture mapping")
                    .expect("recognized provider fixture")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            normalized
                .iter()
                .map(|event| event.kind.clone())
                .collect::<Vec<_>>(),
            expected
        );
        for event in normalized {
            validate_event(&event, EventAdmission::Generic).expect("normalized fixture event");
            let serialized = serde_json::to_string(&event).expect("normalized event json");
            assert!(!serialized.contains("codex-session"));
            assert!(!serialized.contains("claude-session"));
            assert!(!serialized.contains("dsh-session"));
            assert!(!serialized.contains("tool-1"));
            assert!(!serialized.contains("subagent-secret"));
            assert!(!serialized.contains("rate_limit"));
        }
    }
}

#[test]
fn frozen_codex_notification_fixture_projects_only_matching_completion_metadata() {
    let normalized = include_str!("../../../tests/fixtures/activity/codex-notifications.jsonl")
        .lines()
        .map(|line| {
            let raw: Value = serde_json::from_str(line).expect("Codex notification fixture");
            normalize_provider_notification(AgentKind::Codex, "runtime-1", &raw)
                .expect("Codex notification mapping")
        })
        .collect::<Vec<_>>();
    let completion = normalized[0].as_ref().expect("recognized completion");
    assert_eq!(completion.kind, TurnEventKind::TurnCompleted);
    assert_eq!(completion.confidence, Confidence::Authoritative);
    assert!(normalized[1].is_none());
    let serialized = serde_json::to_string(completion).expect("normalized completion");
    for forbidden in [
        "codex-session",
        "codex-turn",
        "<redacted>",
        "input-messages",
        "last-assistant-message",
    ] {
        assert!(!serialized.contains(forbidden), "forbidden {forbidden}");
    }
}
