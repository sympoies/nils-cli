use super::*;
use pretty_assertions::{assert_eq, assert_ne};
use std::time::{Duration, Instant};

const ID: &str = "native-receipt";
const PROMPT: &str = "/sessions/native-receipt/prompt/v2";
const CAPABILITY: &str = "agent-session.reply-delivery.idle-boundary.v1";

fn state_for_list(root: &Path, tmux: PathBuf, ids: &'static [&'static str]) -> Arc<ServeState> {
    let collector: SessionCollector = Arc::new(move |context, tmux| {
        ids.iter()
            .map(|id| {
                load_session_record(context, id).map(|record| {
                    crate::session_view(context, &record, Some("running".into()), Some(tmux))
                })
            })
            .collect()
    });
    state_with_session_collector(root, Some(TOKEN), tmux, collector)
}

fn queue_fixture(root: &Path) -> (Arc<ServeState>, String, fs::File, Provider) {
    let (state, launch, lease, provider) = native_fixture(root);
    (
        state_for_list(root, state.tmux_bin.clone(), &[ID]),
        launch,
        lease,
        provider,
    )
}

async fn list(app: &Router) -> Vec<Value> {
    let (status, response) = call(app.clone(), bearer_get("/sessions")).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    response["data"]["sessions"].as_array().unwrap().clone()
}

async fn boundary(app: &Router) -> Value {
    boundary_for_session(app, ID).await
}

async fn boundary_for_session(app: &Router, id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let sessions = list(app).await;
        let view = sessions.iter().find(|session| session["id"] == id).unwrap();
        assert!(
            view["capabilities"]
                .as_array()
                .is_some_and(|caps| caps.iter().any(|cap| cap == CAPABILITY)),
            "missing reply-delivery capability: {view}"
        );
        if let Some(value) = view.get("idle_ready_boundary") {
            assert!(!value.to_string().contains("private-prior-turn"));
            return value.clone();
        }
        assert!(
            Instant::now() < deadline,
            "readiness did not become available"
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

fn request(incarnation: &str, key: &str, token: &Value) -> Value {
    json!({
        "text": "inspect the workspace",
        "expected_session_incarnation": incarnation,
        "request_id": key,
        "require_idle": true,
        "expected_idle_ready_boundary": token,
    })
}

async fn submit(app: &Router, body: Value) -> (StatusCode, Value) {
    call(app.clone(), post_json(PROMPT, Some(TOKEN), body)).await
}

async fn receipt(app: &Router, incarnation: &str, key: &str) -> Value {
    let path = format!("{PROMPT}/receipts/{key}?expected_session_incarnation={incarnation}");
    let (status, response) = call(app.clone(), bearer_get(&path)).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    response["data"]["receipt"].clone()
}

fn assert_refused(result: &(StatusCode, Value), code: &str) -> Value {
    assert_eq!(result.0, StatusCode::CONFLICT, "{}", result.1);
    assert_eq!(result.1["error"]["code"], code);
    let receipt = &result.1["error"]["details"]["receipt"];
    assert_eq!(receipt["outcome"], "refused");
    assert_eq!(receipt["code"], code);
    assert!(receipt.get("delivery_kind").is_none());
    receipt.clone()
}

#[tokio::test]
async fn idle_boundary_concurrent_requests_replay_and_conflict() {
    let temp = tempfile::TempDir::new().unwrap();
    let (state, launch, _lease, provider) = queue_fixture(temp.path());
    provider.busy.store(false, Ordering::SeqCst);
    provider
        .queue
        .confirm_idle_boundary_input
        .store(true, Ordering::SeqCst);
    let app = router(state);
    let boundary = boundary(&app).await;
    let one = request(&launch, "one", &boundary["token"]);
    let two = request(&launch, "two", &boundary["token"]);
    let (first, second) = tokio::join!(submit(&app, one.clone()), submit(&app, two.clone()));
    let (winner, submitted, loser) = if first.0 == StatusCode::OK {
        (one, first, second)
    } else {
        (two, second, first)
    };
    assert_eq!(submitted.0, StatusCode::OK, "{}", submitted.1);
    let success = &submitted.1["data"]["receipt"];
    assert_eq!(success["outcome"], "submitted");
    assert_eq!(success["delivery_kind"], "idle_boundary");
    assert!(!submitted.1.to_string().contains("accepted-turn"));
    assert!(!success.to_string().contains("inspect the workspace"));
    let refused = assert_refused(&loser, "stale-idle-ready-boundary");

    let winning_key = success["request_id"].as_str().unwrap();
    let losing_key = refused["request_id"].as_str().unwrap();
    assert_ne!(winning_key, losing_key);
    assert_eq!(receipt(&app, &launch, winning_key).await, *success);
    assert_eq!(receipt(&app, &launch, losing_key).await, refused);
    assert_eq!(submit(&app, winner.clone()).await, submitted);

    let mut different = winner.clone();
    different["expected_idle_ready_boundary"] = json!("b03afb5f-e527-4671-8274-64349bb5ecde");
    let (status, conflict) = submit(&app, different).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"]["code"], "request-id-conflict");
    let mut ordinary = winner;
    ordinary
        .as_object_mut()
        .unwrap()
        .remove("expected_idle_ready_boundary");
    let (status, conflict) = submit(&app, ordinary).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"]["code"], "request-id-conflict");
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn idle_boundary_rejects_an_intervening_turn_between_list_reads() {
    let temp = tempfile::TempDir::new().unwrap();
    let (state, launch, _lease, provider) = queue_fixture(temp.path());
    provider.busy.store(false, Ordering::SeqCst);
    let app = router(state);
    let before = boundary(&app).await;

    provider
        .queue
        .closed_idle_boundary_turn
        .store(true, Ordering::SeqCst);
    let result = submit(&app, request(&launch, "stale", &before["token"])).await;
    let refused = assert_refused(&result, "stale-idle-ready-boundary");
    assert_eq!(receipt(&app, &launch, "stale").await, refused);
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 0);
    let after = boundary(&app).await;
    assert_ne!(after["token"], before["token"]);
    let generation = |value: &Value| {
        value["generation"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    assert!(generation(&after) > generation(&before));
}

#[tokio::test]
async fn idle_boundary_rejects_a_replaced_incarnation() {
    let temp = tempfile::TempDir::new().unwrap();
    let (state, launch, _lease, provider) = queue_fixture(temp.path());
    provider.busy.store(false, Ordering::SeqCst);
    let app = router(state.clone());
    let before = boundary(&app).await;
    let replacement = "fixture-replacement-runtime";
    {
        let _lock = crate::acquire_session_record_lock(&state.context, ID).unwrap();
        let mut record = load_session_record(&state.context, ID).unwrap();
        let runtime = record.runtime.as_mut().unwrap();
        runtime.launch_id = replacement.into();
        runtime.generation += 1;
        crate::write_session_record(&state.context, &record).unwrap();
    }
    let old = submit(&app, request(&launch, "old", &before["token"])).await;
    assert_refused(&old, "session-incarnation-conflict");
    let rebound = submit(&app, request(replacement, "rebound", &before["token"])).await;
    assert_refused(&rebound, "stale-idle-ready-boundary");
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn idle_boundary_list_does_not_wait_on_a_slow_provider() {
    let temp = tempfile::TempDir::new().unwrap();
    let (state, _launch, _lease, provider) = native_fixture(temp.path());
    seed_session_with_runtime(temp.path(), "ordinary-one", "codex", "ordinary-one");
    seed_session_with_runtime(temp.path(), "ordinary-two", "claude", "ordinary-two");
    provider.busy.store(false, Ordering::SeqCst);
    provider
        .queue
        .hold_idle_boundary_read
        .store(true, Ordering::SeqCst);
    let app = router(state_for_list(
        temp.path(),
        state.tmux_bin.clone(),
        &[ID, "ordinary-one", "ordinary-two"],
    ));
    let initial = tokio::time::timeout(Duration::from_secs(1), list(&app))
        .await
        .expect("a slow provider must not hold the list");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !provider
        .queue
        .entered_idle_boundary_read
        .load(Ordering::SeqCst)
    {
        assert!(
            Instant::now() < deadline,
            "observation did not reach the provider"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let observed = tokio::time::timeout(Duration::from_secs(1), list(&app))
        .await
        .expect("a held read must not hold unrelated sessions");
    assert_eq!(observed.len(), 3);
    for id in ["ordinary-one", "ordinary-two"] {
        let original = initial.iter().find(|view| view["id"] == id).unwrap();
        let current = observed.iter().find(|view| view["id"] == id).unwrap();
        assert_eq!(
            current["session_incarnation"],
            original["session_incarnation"]
        );
        assert_eq!(current["status"], "running");
        assert_eq!(current["capabilities"], original["capabilities"]);
        assert!(current.get("idle_ready_boundary").is_none());
    }
    let slow = observed.iter().find(|view| view["id"] == ID).unwrap();
    assert!(slow.get("idle_ready_boundary").is_none());
    provider
        .queue
        .hold_idle_boundary_read
        .store(false, Ordering::SeqCst);
    let _ = boundary(&app).await;
}

#[tokio::test]
async fn idle_boundary_rejects_invalid_combinations_before_creating_a_claim() {
    let temp = tempfile::TempDir::new().unwrap();
    let (state, launch, _lease, provider) = native_fixture(temp.path());
    let app = router(state);
    let mut body = request(
        &launch,
        "invalid",
        &json!("b03afb5f-e527-4671-8274-64349bb5ecde"),
    );
    body["require_idle"] = json!(false);
    let (status, result) = submit(&app, body.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{result}");
    assert_eq!(result["error"]["code"], "invalid-idle-ready-boundary");
    body["require_idle"] = json!(true);
    body["expected_idle_ready_boundary"] = Value::Null;
    let (status, result) = submit(&app, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{result}");
    assert_eq!(result["error"]["code"], "invalid-idle-ready-boundary");

    let lookup = format!("{PROMPT}/receipts/invalid?expected_session_incarnation={launch}");
    let (status, result) = call(app, bearer_get(&lookup)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{result}");
    assert_eq!(result["error"]["code"], "prompt-receipt-not-found");
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 0);
}

const CLAUDE: &str = "claude-boundary-fixture";

fn claude_fixture(root: &Path) -> (Arc<ServeState>, String, SessionRecord) {
    const CONVERSATION: &str = "fixture-provider-conversation";
    let launch = seed_claude_prompt_session(root, CLAUDE);
    let tmux = guard_tmux(root, CLAUDE);
    let state = state_for_list(root, tmux, &[CLAUDE]);
    let context = &state.context;
    let mut record = load_session_record(context, CLAUDE).unwrap();
    record.provider_resume = Some(
        serde_json::from_value(json!({
            "provider":"claude", "session_id":CONVERSATION,
            "captured_at":"2000-01-01T00:00:00Z", "capture_method":"fixture", "resume_args":[]
        }))
        .unwrap(),
    );
    record.extra.insert(
        "claude_account_binding".into(),
        json!({
            "schema_version":"agent-session.claude-account-binding.v1",
            "selected_account":"fixture-account", "revision":1, "state":"bound",
            "applied_runtime_id":launch, "updated_at":"2000-01-01T00:00:00Z"
        }),
    );
    crate::write_session_record(context, &record).unwrap();
    activity::activate_runtime(context, &record).unwrap();
    for (event, kind) in [
        ("fixture-start", "turn_started"),
        ("fixture-done", "turn_completed"),
    ] {
        activity::ingest_event(
            context,
            CLAUDE,
            serde_json::from_value(json!({
                "schema_version":activity::TURN_EVENT_VERSION, "event_id":event,
                "runtime_id":launch, "provider":"claude", "provider_session_id":CONVERSATION,
                "provider_turn_id":"fixture-private-turn", "kind":kind, "confidence":"authoritative"
            }))
            .unwrap(),
        )
        .unwrap();
    }
    (state, launch, record)
}

async fn observe_claude(
    state: &Arc<ServeState>,
    record: &SessionRecord,
) -> prompt_receipts::idle_boundary::Boundary {
    let context = &state.context;
    let app = router(state.clone());
    let _ = list(&app).await;
    // Wait for this one batch to publish without starting another observer.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(Some(boundary)) = prompt_receipts::reply_delivery::projection(context, record) {
            return boundary;
        }
        assert!(
            Instant::now() < deadline,
            "Claude readiness did not become available"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn idle_boundary_claude_checks_the_real_composer_before_paste() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path();
    let (state, launch, record) = claude_fixture(root);
    let available = observe_claude(&state, &record).await;
    let app = router(state);
    let uri = format!("/sessions/{CLAUDE}/prompt/v2");
    fs::write(root.join("composer"), "owner draft").unwrap();
    let request_with_draft = request(&launch, "draft-refused", &json!(available.token));
    let refusal = call(
        app.clone(),
        post_json(&uri, Some(TOKEN), request_with_draft),
    )
    .await;
    assert_refused(&refusal, "busy");
    assert_eq!(
        fs::read_to_string(root.join("composer")).unwrap(),
        "owner draft"
    );
    assert!(!root.join("enter-count").exists());
    assert!(!root.join("submitted").exists());
    assert!(
        !fs::read_to_string(root.join("transport-calls"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|call| call[0] == "paste-buffer" || call[0] == "send-keys")
    );

    fs::write(root.join("composer"), "").unwrap();
    let renewed = boundary_for_session(&app, CLAUDE).await;
    assert_ne!(renewed["token"], json!(available.token));
    let send = request(&launch, "fresh-attempt", &renewed["token"]);
    let (status, response) = call(app, post_json(&uri, Some(TOKEN), send)).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(
        response["data"]["receipt"]["delivery_kind"],
        "idle_boundary"
    );
    assert_eq!(
        fs::read_to_string(root.join("submitted")).unwrap(),
        "inspect the workspace"
    );
    assert_eq!(fs::read_to_string(root.join("enter-count")).unwrap(), "1");
}

#[tokio::test]
async fn idle_boundary_distinguishes_literal_paths_from_owner_commands() {
    for literal in ["/workspace/file", "/ns/cmd explain this"] {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path();
        let (state, launch, record) = claude_fixture(root);
        let available = observe_claude(&state, &record).await;
        let app = router(state);
        let uri = format!("/sessions/{CLAUDE}/prompt/v2");
        let token = json!(available.token);

        let mut command = request(&launch, "command", &token);
        command["text"] = json!("/status");
        let refusal = call(app.clone(), post_json(&uri, Some(TOKEN), command)).await;
        let refused = assert_refused(&refusal, "idle-boundary-command-unsupported");
        assert!(!root.join("buffer").exists());
        assert!(!root.join("enter-count").exists());

        let mut prompt = request(&launch, "literal", &token);
        prompt["text"] = json!(literal);
        let (status, response) = call(app.clone(), post_json(&uri, Some(TOKEN), prompt)).await;
        assert_eq!(status, StatusCode::OK, "{literal}: {response}");
        let submitted = &response["data"]["receipt"];
        assert_eq!(submitted["outcome"], "submitted");
        assert_eq!(submitted["delivery_kind"], "idle_boundary");
        assert_eq!(fs::read_to_string(root.join("submitted")).unwrap(), literal);
        assert_eq!(fs::read_to_string(root.join("enter-count")).unwrap(), "1");

        for (key, expected) in [("command", &refused), ("literal", submitted)] {
            let lookup = format!("{uri}/receipts/{key}?expected_session_incarnation={launch}");
            let (status, response) = call(app.clone(), bearer_get(&lookup)).await;
            assert_eq!(status, StatusCode::OK, "{response}");
            assert_eq!(&response["data"]["receipt"], expected);
        }
    }
}

#[tokio::test]
async fn idle_boundary_receipt_recovers_lost_acknowledgement_on_the_original_key() {
    let temp = tempfile::TempDir::new().unwrap();
    let (state, launch, _lease, provider) = queue_fixture(temp.path());
    provider.busy.store(false, Ordering::SeqCst);
    provider.acknowledge.store(false, Ordering::SeqCst);
    let app = router(state);
    let available = boundary(&app).await;
    let body = request(&launch, "lost-ack", &available["token"]);
    let (_, response) = submit(&app, body.clone()).await;
    let unknown = &response["error"]["details"]["receipt"];
    assert_eq!(unknown["outcome"], "unknown");
    assert!(unknown.get("delivery_kind").is_none());
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);

    provider.history_kind.store(5, Ordering::SeqCst);
    provider.visible.store(true, Ordering::SeqCst);
    let recovered = receipt(&app, &launch, "lost-ack").await;
    assert_eq!(recovered["outcome"], "submitted");
    assert_eq!(recovered["delivery_kind"], "idle_boundary");
    assert_eq!(submit(&app, body).await.1["data"]["receipt"], recovered);
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn idle_boundary_repair_refuses_late_terminal_ingress_before_paste() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path();
    let (state, launch, record) = claude_fixture(root);
    let available = observe_claude(&state, &record).await;

    let ctx = &state.context;
    let lock = crate::acquire_session_record_lock(ctx, CLAUDE).unwrap();
    let ingress =
        fs::File::open(crate::session_dir(ctx, CLAUDE).join(".activity-ingress.lock")).unwrap();
    // Model the writer's advertised shared intent while it waits for the record lock.
    // SAFETY: ingress is an open test fixture descriptor and remains held until refusal.
    assert_eq!(
        unsafe { libc::flock(ingress.as_raw_fd(), libc::LOCK_SH) },
        0
    );
    assert!(activity::pending::event_pending(ctx, CLAUDE).unwrap());
    let guards = crate::send_guard::GuardArgs {
        expect_runtime: Some(launch),
        expected_idle_ready_boundary: Some(available.token),
        require_idle: true,
        require_empty_composer: true,
        ..Default::default()
    };
    let prompt = root.join("fixture-prompt");
    fs::write(&prompt, "inspect the workspace").unwrap();
    let result = crate::send::load_and_paste_guarded(
        &state.tmux_bin,
        "fixture-buffer",
        &format!("{}:0.0", record.tmux_session),
        &prompt,
        crate::PANE_INPUT_COMMAND_TIMEOUT,
        crate::PasteMode::Bracketed,
        || {
            prompt_receipts::reply_delivery::consume(
                ctx,
                &record,
                &guards,
                &lock,
                prompt_receipts::reply_delivery::AdmittedInput::Terminal {
                    tmux: &state.tmux_bin,
                    text: "inspect the workspace",
                    stopped: false,
                },
            )
        },
    );
    assert_eq!(result.unwrap_err().code(), "send-provider-event-pending");
    assert!(matches!(
        prompt_receipts::reply_delivery::projection(ctx, &record),
        Some(None)
    ));
    assert!(!root.join("enter-count").exists());
    assert!(
        !fs::read_to_string(root.join("transport-calls"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|call| call[0] == "paste-buffer" || call[0] == "send-keys")
    );
}

#[tokio::test]
async fn idle_boundary_repair_admits_a_replaced_native_conversation() {
    let temp = tempfile::TempDir::new().unwrap();
    let (state, launch, _lease, provider) = queue_fixture(temp.path());
    provider.busy.store(false, Ordering::SeqCst);
    provider
        .queue
        .closed_idle_boundary_turn
        .store(true, Ordering::SeqCst);
    provider
        .queue
        .confirm_idle_boundary_input
        .store(true, Ordering::SeqCst);
    let app = router(state.clone());
    let before = boundary(&app).await;
    let (status, response) = submit(&app, request(&launch, "before-new", &before["token"])).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);

    {
        let _lock = crate::acquire_session_record_lock(&state.context, ID).unwrap();
        let mut record = load_session_record(&state.context, ID).unwrap();
        record.provider_resume.as_mut().unwrap().session_id = "replacement-thread".into();
        codex_app_server::replace_thread_binding(&record, "replacement-thread").unwrap();
        crate::write_session_record(&state.context, &record).unwrap();
        provider
            .queue
            .replaced_idle_boundary_thread
            .store(true, Ordering::SeqCst);
    }
    let stale = submit(&app, request(&launch, "stale-old-thread", &before["token"])).await;
    assert_refused(&stale, "stale-idle-ready-boundary");
    let after = boundary(&app).await;
    assert_ne!(after["token"], before["token"]);
    let generation = |value: &Value| {
        value["generation"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap()
    };
    assert!(generation(&after) > generation(&before));
    let (status, response) = submit(&app, request(&launch, "after-new", &after["token"])).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn idle_boundary_repair_keeps_a_spent_watermark_after_account_rebind() {
    let temp = tempfile::TempDir::new().unwrap();
    let (state, launch, _lease, provider) = queue_fixture(temp.path());
    provider.busy.store(false, Ordering::SeqCst);
    provider
        .queue
        .closed_idle_boundary_turn
        .store(true, Ordering::SeqCst);
    provider
        .queue
        .confirm_idle_boundary_input
        .store(true, Ordering::SeqCst);
    let app = router(state.clone());
    let before = boundary(&app).await;
    let (status, response) =
        submit(&app, request(&launch, "before-rebind", &before["token"])).await;
    assert_eq!(status, StatusCode::OK, "{response}");

    {
        let _lock = crate::acquire_session_record_lock(&state.context, ID).unwrap();
        let mut record = load_session_record(&state.context, ID).unwrap();
        let binding = record.extra.get_mut("codex_account_binding").unwrap();
        binding["selected_account"] = json!("fixture-next-account");
        binding["revision"] = json!(2);
        crate::write_session_record(&state.context, &record).unwrap();
        // The provider exposes the same watermark as the spent grant until the
        // accepted turn is independently visible in the provider history.
        provider
            .queue
            .confirm_idle_boundary_input
            .store(false, Ordering::SeqCst);
    }
    let stale = submit(&app, request(&launch, "stale-account", &before["token"])).await;
    assert_refused(&stale, "stale-idle-ready-boundary");

    let initial_reads = provider.queue.watermark_reads.load(Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let sessions = list(&app).await;
        let view = sessions.iter().find(|session| session["id"] == ID).unwrap();
        assert!(view.get("idle_ready_boundary").is_none(), "{view}");
        // Only one observation batch runs at a time. A second watermark read
        // proves that at least one unchanged watermark was processed.
        if provider.queue.watermark_reads.load(Ordering::SeqCst) >= initial_reads + 2 {
            break;
        }
        assert!(Instant::now() < deadline, "no fresh watermark observation");
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    provider
        .queue
        .confirm_idle_boundary_input
        .store(true, Ordering::SeqCst);
    assert_ne!(boundary(&app).await["token"], before["token"]);
    assert_eq!(provider.mutations.load(Ordering::SeqCst), 1);
}
