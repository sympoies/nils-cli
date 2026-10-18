use super::*;
use pretty_assertions::assert_eq;

#[path = "direct_prompt.rs"]
mod direct_prompt;

#[path = "claude_prompt_ack.rs"]
mod claude_prompt_ack;

fn keyed_body(incarnation: &str) -> Value {
    json!({"text":"inspect the workspace", "expected_session_incarnation":incarnation,
        "request_id":"attempt-1"})
}

#[tokio::test]
async fn keyed_submission_replays_without_dispatch_and_survives_restart() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_codex_app_server_session(tmp.path(), "keyed");
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        "keyed".into(),
        CodexControlEntry {
            launch_id: launch.clone(),
            handle,
        },
    );
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Prompt { response, .. }) = commands.recv().await
        else {
            panic!("missing prompt command");
        };
        response.send(Ok("turn-keyed".into())).unwrap();
        commands
    });
    let uri = "/sessions/keyed/prompt/v2";
    let (status, first) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), keyed_body(&launch)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["data"]["receipt"]["outcome"], "submitted");
    let mut commands = responder.await.unwrap();
    let (status, repeat) = call(router(st), post_json(uri, Some(TOKEN), keyed_body(&launch))).await;
    assert_eq!(status, StatusCode::OK, "{repeat}");
    assert_eq!(repeat, first);
    assert!(commands.try_recv().is_err());
    let mut replaced = load_session_record(
        &state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path())).context,
        "keyed",
    )
    .unwrap();
    replaced.runtime.as_mut().unwrap().launch_id = "replacement-launch".into();
    crate::write_session_record(
        &CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        },
        &replaced,
    )
    .unwrap();
    let restarted = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let lookup = format!("{uri}/receipts/attempt-1?expected_session_incarnation={launch}");
    let (status, receipt) = call(router(restarted), bearer_get(&lookup)).await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["data"]["receipt"], first["data"]["receipt"]);
}

fn bearer_get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn unknown_submission_is_recoverable_via_authenticated_lookup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_codex_app_server_session(tmp.path(), "unknown-keyed");
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        "unknown-keyed".into(),
        CodexControlEntry {
            launch_id: launch.clone(),
            handle,
        },
    );
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::Prompt { response, .. }) = commands.recv().await
        else {
            panic!("missing prompt command");
        };
        response
            .send(Err("unknown provider outcome".into()))
            .unwrap();
    });
    let uri = "/sessions/unknown-keyed/prompt/v2";
    let (status, body) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), keyed_body(&launch)),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert_eq!(body["error"]["details"]["receipt"]["outcome"], "unknown");
    responder.await.unwrap();
    let lookup = format!("{uri}/receipts/attempt-1?expected_session_incarnation={launch}");
    let (status, _) = call(router(st), get(&lookup)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let restarted = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let (status, body) = call(router(restarted), bearer_get(&lookup)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["receipt"]["outcome"], "unknown");
}

#[tokio::test]
async fn require_idle_busy_refusal_is_typed_and_terminal() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_claude_prompt_session(tmp.path(), "busy-keyed");
    let st = state(
        tmp.path(),
        Some(TOKEN),
        guard_tmux(tmp.path(), "busy-keyed"),
    );
    let record = load_session_record(&st.context, "busy-keyed").unwrap();
    activity::activate_runtime(&st.context, &record).unwrap();
    activity::ingest_event(
        &st.context,
        &record.id,
        claude_turn_started_event(&launch, "busy-start", "busy-turn"),
    )
    .unwrap();
    let mut body = keyed_body(&launch);
    body["require_idle"] = json!(true);
    let uri = "/sessions/busy-keyed/prompt/v2";
    let (status, first) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{first}");
    assert_eq!(first["error"]["code"], "busy");
    assert_eq!(first["error"]["details"]["receipt"]["outcome"], "refused");
    let (status, repeat) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(repeat, first);
}

// The actual guard reads exact identity and composer state from this modeled
// transport. No live terminal or provider is used.
fn guard_tmux(root: &Path, id: &str) -> PathBuf {
    let path = root.join("guard-tmux");
    fs::write(root.join("guard-id"), id).unwrap();
    fs::write(root.join("composer"), "").unwrap();
    fs::write(&path, r#"#!/usr/bin/env python3
import json, pathlib, sys, time
root = pathlib.Path(__file__).parent
if (root / 'transport-delay-ms').exists(): time.sleep(int((root / 'transport-delay-ms').read_text()) / 1000)
args = sys.argv[1:]
with (root / 'transport-calls').open('a') as out: out.write(json.dumps(args) + '\n')
record = json.loads((root / 'sessions' / (root / 'guard-id').read_text() / 'session.json').read_text())
if args[0] == 'show-environment':
    values = {'AGENT_SESSION_ID': record['id'], 'AGENT_SESSION_STATE_DIR': str(root), 'AGENT_SESSION_RUNTIME_ID': record['runtime']['launch_id']}
    keys = [key for key in args if key in values]
    if keys:
        for key in keys: print(key + '=' + values[key])
        sys.exit(0)
    sys.exit(1)
if args[0] == 'capture-pane':
    text = (root / 'composer').read_text()
    print('────────────────────────────\n❯ ' + text + '\n────────────────────────────')
    sys.exit(0)
if args[0] == 'load-buffer': (root / 'buffer').write_bytes(pathlib.Path(args[-1]).read_bytes())
if args[0] == 'paste-buffer':
    (root / 'composer').write_bytes((root / 'buffer').read_bytes())
    if (root / 'owner-addition').exists():
        with (root / 'composer').open('a') as out: out.write(' owner addition')
if args[0] == 'send-keys':
    if args[-1] == 'BSpace':
        count = int(args[args.index('-N') + 1])
        text = (root / 'composer').read_text()
        (root / 'composer').write_text(text[:-count]); sys.exit(0)
    count_file = root / 'enter-count'
    count = int(count_file.read_text()) + 1 if count_file.exists() else 1
    count_file.write_text(str(count))
    if (root / 'stuck').exists() or ((root / 'swallow-first').exists() and count == 1): sys.exit(0)
    (root / 'submitted').write_text((root / 'composer').read_text())
    (root / 'composer').write_text('')
sys.exit(0)
"#).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[tokio::test]
async fn concurrent_duplicate_observes_unknown_until_owner_finishes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_codex_app_server_session(tmp.path(), "concurrent");
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        "concurrent".into(),
        CodexControlEntry {
            launch_id: launch.clone(),
            handle,
        },
    );
    let uri = "/sessions/concurrent/prompt/v2";
    let app = router(st.clone());
    let body = keyed_body(&launch);
    let first = tokio::spawn(call(app, post_json(uri, Some(TOKEN), body.clone())));
    let Some(codex_app_server::ControlCommand::Prompt { response, .. }) = commands.recv().await
    else {
        panic!("no dispatch")
    };
    let (status, pending) = call(router(st.clone()), post_json(uri, Some(TOKEN), body)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{pending}");
    assert_eq!(pending["error"]["details"]["receipt"]["outcome"], "unknown");
    let lookup = format!("{uri}/receipts/attempt-1?expected_session_incarnation={launch}");
    let (status, pending) = call(router(st.clone()), bearer_get(&lookup)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pending["data"]["receipt"]["outcome"], "unknown");
    assert!(commands.try_recv().is_err());
    response.send(Ok("turn-concurrent".into())).unwrap();
    assert_eq!(first.await.unwrap().0, StatusCode::OK);
    let (_, completed) = call(router(st), bearer_get(&lookup)).await;
    assert_eq!(completed["data"]["receipt"]["outcome"], "submitted");
}

#[tokio::test]
async fn keyed_validation_conflicts_and_missing_lookup_are_typed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_codex_app_server_session(tmp.path(), "validation");
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let uri = "/sessions/validation/prompt/v2";
    for invalid in [
        "".to_string(),
        "x".repeat(129),
        "invalid/id".into(),
        "has space".into(),
        "非ascii".into(),
    ] {
        let mut body = keyed_body(&launch);
        body["request_id"] = json!(invalid);
        let (status, body) = call(router(st.clone()), post_json(uri, Some(TOKEN), body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"]["code"], "invalid-request-id");
    }
    let lookup = format!("{uri}/receipts/attempt-1?expected_session_incarnation={launch}");
    let (status, body) = call(router(st.clone()), bearer_get(&lookup)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"]["code"], "prompt-receipt-not-found");
    let mut body = keyed_body(&launch);
    body["text"] = json!("");
    let (status, refused) = call(router(st.clone()), post_json(uri, Some(TOKEN), body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"]["details"]["receipt"]["code"],
        "empty-prompt"
    );
    let (status, body) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), keyed_body(&launch)),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["code"], "request-id-conflict");
    let (_, original) = call(router(st), bearer_get(&lookup)).await;
    assert_eq!(original["data"]["receipt"]["code"], "empty-prompt");
}

#[tokio::test]
async fn stale_incarnation_receipt_remains_readable_after_runtime_replacement() {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_codex_app_server_session(tmp.path(), "replaced");
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));
    let uri = "/sessions/replaced/prompt/v2";
    let body = keyed_body("previous-launch");
    let (status, first) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{first}");
    assert_eq!(
        first["error"]["details"]["receipt"]["code"],
        "session-incarnation-conflict"
    );
    let lookup = format!("{uri}/receipts/attempt-1?expected_session_incarnation=previous-launch");
    let (_, historical) = call(router(st.clone()), bearer_get(&lookup)).await;
    assert_eq!(
        historical["data"]["receipt"],
        first["error"]["details"]["receipt"]
    );
    let (_, repeat) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
    assert_eq!(repeat, first);
}

fn make_idle(context: &CliContext, id: &str, launch: &str) {
    let record = load_session_record(context, id).unwrap();
    activity::activate_runtime(context, &record).unwrap();
    for (event_id, kind) in [
        ("idle-start", "turn_started"),
        ("idle-done", "turn_completed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version":activity::TURN_EVENT_VERSION, "event_id":event_id,
            "runtime_id":launch, "provider":"claude", "provider_turn_id":"idle-turn",
            "kind":kind, "confidence":"authoritative"
        }))
        .unwrap();
        activity::ingest_event(context, id, event).unwrap();
    }
}

#[tokio::test]
async fn idle_guard_preserves_draft_and_can_submit_with_a_fresh_attempt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_claude_prompt_session(tmp.path(), "idle-keyed");
    let st = state(
        tmp.path(),
        Some(TOKEN),
        guard_tmux(tmp.path(), "idle-keyed"),
    );
    make_idle(&st.context, "idle-keyed", &launch);
    fs::write(tmp.path().join("composer"), "existing user draft").unwrap();
    let uri = "/sessions/idle-keyed/prompt/v2";
    let mut body = keyed_body(&launch);
    body["require_idle"] = json!(true);
    let (status, refusal) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refusal}");
    assert_eq!(refusal["error"]["code"], "busy");
    assert_eq!(
        fs::read_to_string(tmp.path().join("composer")).unwrap(),
        "existing user draft"
    );
    assert!(!tmp.path().join("submitted").exists());
    fs::write(tmp.path().join("composer"), "").unwrap();
    let (_, still_refused) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), body.clone()),
    )
    .await;
    assert_eq!(still_refused, refusal);
    body["request_id"] = json!("attempt-2");
    let (status, submitted) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    assert_eq!(submitted["data"]["receipt"]["outcome"], "submitted");
    assert_eq!(
        fs::read_to_string(tmp.path().join("submitted")).unwrap(),
        "inspect the workspace"
    );
}

#[tokio::test]
async fn unobservable_idle_fails_closed_without_pasting() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_claude_prompt_session(tmp.path(), "unobservable");
    let st = state(
        tmp.path(),
        Some(TOKEN),
        guard_tmux(tmp.path(), "unobservable"),
    );
    let mut body = keyed_body(&launch);
    body["require_idle"] = json!(true);
    let (status, refusal) = call(
        router(st),
        post_json("/sessions/unobservable/prompt/v2", Some(TOKEN), body),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refusal}");
    assert_eq!(refusal["error"]["details"]["receipt"]["outcome"], "refused");
    assert_eq!(refusal["error"]["code"], "send-idle-unobservable");
    assert!(!tmp.path().join("submitted").exists());
    assert!(!tmp.path().join("buffer").exists());
}

#[tokio::test]
async fn fenced_structured_prompt_requires_exact_body_shape() {
    let tmp = tempfile::TempDir::new().unwrap();
    let st = state(tmp.path(), Some(TOKEN), empty_composer_tmux(tmp.path()));

    for body in [
        json!({ "text": "missing the required fence" }),
        json!({
            "text": "misspelled fence",
            "expected_session_incarnaton": "launch-missing",
        }),
        json!({
            "text": "unexpected field",
            "expected_session_incarnation": "launch-missing",
            "unexpected": true,
        }),
    ] {
        let (status, body) = call(
            router(st.clone()),
            post_json("/sessions/missing/prompt/v2", Some(TOKEN), body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
        assert_eq!(body["error"]["code"], "invalid-json-body");
    }
}

#[tokio::test]
async fn idle_claude_guard_retries_swallowed_enter_without_residue() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_claude_prompt_session(tmp.path(), "swallowed");
    let st = state(tmp.path(), Some(TOKEN), guard_tmux(tmp.path(), "swallowed"));
    make_idle(&st.context, "swallowed", &launch);
    fs::write(tmp.path().join("swallow-first"), "").unwrap();
    // Exercise admission and retry round trips on a slower transport.
    fs::write(tmp.path().join("transport-delay-ms"), "120").unwrap();
    let mut body = keyed_body(&launch);
    body["require_idle"] = json!(true);
    let uri = "/sessions/swallowed/prompt/v2";
    let (status, result) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["receipt"]["outcome"], "submitted");
    assert_eq!(
        fs::read_to_string(tmp.path().join("submitted")).unwrap(),
        "inspect the workspace"
    );
    assert_eq!(fs::read_to_string(tmp.path().join("composer")).unwrap(), "");
    assert_eq!(
        fs::read_to_string(tmp.path().join("enter-count")).unwrap(),
        "2"
    );
    let (_, replay) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
    assert_eq!(replay, result);
    assert_eq!(
        fs::read_to_string(tmp.path().join("enter-count")).unwrap(),
        "2"
    );
}

#[tokio::test]
async fn idle_claude_guard_cleans_stuck_prompt_and_returns_terminal_receipt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_claude_prompt_session(tmp.path(), "stuck");
    let st = state(tmp.path(), Some(TOKEN), guard_tmux(tmp.path(), "stuck"));
    make_idle(&st.context, "stuck", &launch);
    fs::write(tmp.path().join("stuck"), "").unwrap();
    let mut body = keyed_body(&launch);
    body["require_idle"] = json!(true);
    let uri = "/sessions/stuck/prompt/v2";
    let (status, result) = call(
        router(st.clone()),
        post_json(uri, Some(TOKEN), body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{result}");
    assert_eq!(result["error"]["details"]["receipt"]["outcome"], "unknown");
    assert_eq!(
        result["error"]["details"]["receipt"]["composer_cleanup"],
        json!({"cleared":true,"remaining_input":"empty"})
    );
    assert_eq!(fs::read_to_string(tmp.path().join("composer")).unwrap(), "");
    assert!(!tmp.path().join("submitted").exists());
    let (_, replay) = call(router(st), post_json(uri, Some(TOKEN), body)).await;
    assert_eq!(replay, result);
}

#[tokio::test]
async fn idle_claude_guard_reports_changed_composer_without_erasing_owner_input() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_claude_prompt_session(tmp.path(), "changed");
    let st = state(tmp.path(), Some(TOKEN), guard_tmux(tmp.path(), "changed"));
    make_idle(&st.context, "changed", &launch);
    fs::write(tmp.path().join("owner-addition"), "").unwrap();
    let mut body = keyed_body(&launch);
    body["require_idle"] = json!(true);
    let (status, result) = call(
        router(st),
        post_json("/sessions/changed/prompt/v2", Some(TOKEN), body),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{result}");
    assert_eq!(result["error"]["details"]["receipt"]["outcome"], "unknown");
    assert_eq!(
        result["error"]["details"]["receipt"]["composer_cleanup"],
        json!({"cleared":false,"remaining_input":"different_text"})
    );
    assert_eq!(
        fs::read_to_string(tmp.path().join("composer")).unwrap(),
        "inspect the workspace owner addition"
    );
    assert!(!tmp.path().join("submitted").exists());
}

#[tokio::test]
async fn unkeyed_idle_claude_guard_returns_unknown_with_cleanup() {
    let tmp = tempfile::TempDir::new().unwrap();
    let launch = seed_claude_prompt_session(tmp.path(), "unkeyed-stuck");
    let st = state(
        tmp.path(),
        Some(TOKEN),
        guard_tmux(tmp.path(), "unkeyed-stuck"),
    );
    make_idle(&st.context, "unkeyed-stuck", &launch);
    fs::write(tmp.path().join("stuck"), "").unwrap();
    let body = json!({"text":"inspect the workspace", "expected_session_incarnation":launch,
        "require_idle":true});
    let (status, result) = call(
        router(st),
        post_json("/sessions/unkeyed-stuck/prompt/v2", Some(TOKEN), body),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{result}");
    assert_eq!(result["error"]["code"], "structured-prompt-outcome-unknown");
    assert_eq!(result["error"]["details"]["cleared"], true);
    assert_eq!(result["error"]["details"]["remaining_input"], "empty");
    assert!(result["error"]["details"].get("receipt").is_none());
    assert_eq!(fs::read_to_string(tmp.path().join("composer")).unwrap(), "");
    assert!(!tmp.path().join("submitted").exists());
}
