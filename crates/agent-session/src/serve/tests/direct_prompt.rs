//! Direct structured replies must preserve an owner draft before any dispatch.
use super::*;
use pretty_assertions::assert_eq;
use std::sync::atomic::{AtomicUsize, Ordering};

struct DirectFixture {
    tmp: tempfile::TempDir,
    state: Arc<ServeState>,
    launch: String,
    dispatched: Arc<AtomicUsize>,
}

impl DirectFixture {
    fn new(provider: &str) -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        let id = "direct-reply";
        let launch = if provider == "claude" {
            seed_claude_prompt_session(tmp.path(), id)
        } else {
            seed_codex_app_server_session(tmp.path(), id)
        };
        let tmux = guard_tmux(tmp.path(), id);
        let script = fs::read_to_string(&tmux).unwrap().replace(
            "print('────────────────────────────\\n❯ ' + text + '\\n────────────────────────────')",
            "if record['agent'] == 'codex': print('› ' + text + '\\n\\n  status')\n    else: print('────────────────────────────\\n❯ ' + text + '\\n────────────────────────────')",
        ).replace(
            "text = (root / 'composer').read_text()",
            "if (root / 'capture-fails').exists(): sys.exit(1)\n    text = (root / 'composer').read_text()",
        );
        fs::write(&tmux, script).unwrap();
        let state = state(tmp.path(), Some(TOKEN), tmux);
        let dispatched = Arc::new(AtomicUsize::new(0));
        if provider == "codex" {
            let (handle, mut commands) = codex_app_server::control_channel();
            state.codex_controls.lock().unwrap().insert(
                id.into(),
                CodexControlEntry {
                    launch_id: launch.clone(),
                    handle,
                },
            );
            let count = dispatched.clone();
            tokio::spawn(async move {
                while let Some(command) = commands.recv().await {
                    let codex_app_server::ControlCommand::Prompt { response, .. } = command else {
                        panic!("unexpected native command");
                    };
                    count.fetch_add(1, Ordering::SeqCst);
                    response.send(Ok("turn-direct".into())).unwrap();
                }
            });
        } else {
            make_idle(&state.context, id, &launch);
            let context = state.context.clone();
            let launch = launch.clone();
            let root = tmp.path().to_path_buf();
            let count = dispatched.clone();
            tokio::spawn(async move {
                while !root.join("submitted").exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                count.fetch_add(1, Ordering::SeqCst);
                activity::ingest_event(
                    &context,
                    id,
                    claude_turn_started_event(&launch, "direct-start", "direct-turn"),
                )
                .unwrap();
            });
        }
        Self {
            tmp,
            state,
            launch,
            dispatched,
        }
    }

    fn body(&self, route: &str) -> Value {
        let mut body =
            json!({"text":"inspect the workspace", "expected_session_incarnation":self.launch});
        if route == "prompt/v2" {
            body["request_id"] = json!("direct-attempt-1");
        }
        body
    }

    async fn submit(&self, route: &str, body: Value) -> (StatusCode, Value) {
        call(
            router(self.state.clone()),
            post_json(
                &format!("/sessions/direct-reply/{route}"),
                Some(TOKEN),
                body,
            ),
        )
        .await
    }
}

async fn preserves_draft_then_retries(provider: &str) {
    for route in ["prompt", "prompt/v2"] {
        let fixture = DirectFixture::new(provider);
        let composer = fixture.tmp.path().join("composer");
        fs::write(&composer, "owner draft").unwrap();
        let mut body = fixture.body(route);
        let (status, refusal) = fixture.submit(route, body.clone()).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{provider} {route}: {refusal}"
        );
        assert_eq!(refusal["error"]["code"], "composer-not-empty");
        assert_eq!(fs::read_to_string(&composer).unwrap(), "owner draft");
        assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 0);
        assert!(!fixture.tmp.path().join("buffer").exists());
        assert!(!fixture.tmp.path().join("submitted").exists());
        // Model the explicit owner clear, then exercise the real submit route.
        fs::write(&composer, "").unwrap();
        if route == "prompt/v2" {
            assert_eq!(refusal["error"]["details"]["receipt"]["outcome"], "refused");
            let (_, replay) = fixture.submit(route, body.clone()).await;
            assert_eq!(replay, refusal);
            body["request_id"] = json!("direct-attempt-2");
        }
        let (status, sent) = fixture.submit(route, body).await;
        assert_eq!(status, StatusCode::OK, "{provider} {route}: {sent}");
        assert_eq!(sent["data"]["submitted"], true);
        assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 1);
        assert_eq!(fs::read_to_string(&composer).unwrap(), "");
    }
}

async fn empty_composer_delivers(provider: &str) {
    for route in ["prompt", "prompt/v2"] {
        let fixture = DirectFixture::new(provider);
        let (status, sent) = fixture.submit(route, fixture.body(route)).await;
        assert_eq!(status, StatusCode::OK, "{provider} {route}: {sent}");
        assert_eq!(sent["data"]["submitted"], true);
        assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn direct_claude_preserves_draft_then_retries() {
    preserves_draft_then_retries("claude").await;
}
#[tokio::test]
async fn direct_codex_preserves_draft_then_retries() {
    preserves_draft_then_retries("codex").await;
}
#[tokio::test]
async fn direct_claude_empty_composer_delivers() {
    empty_composer_delivers("claude").await;
}
#[tokio::test]
async fn direct_codex_empty_composer_delivers() {
    empty_composer_delivers("codex").await;
}

#[tokio::test]
async fn direct_unobservable_composer_refuses_without_dispatch() {
    for provider in ["claude", "codex"] {
        for route in ["prompt", "prompt/v2"] {
            let fixture = DirectFixture::new(provider);
            fs::write(fixture.tmp.path().join("capture-fails"), "").unwrap();
            fs::write(fixture.tmp.path().join("composer"), "owner draft").unwrap();
            let (status, result) = fixture.submit(route, fixture.body(route)).await;
            assert_eq!(status, StatusCode::CONFLICT, "{provider} {route}: {result}");
            assert_eq!(result["error"]["code"], "composer-unobservable");
            assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 0);
            assert_eq!(
                fs::read_to_string(fixture.tmp.path().join("composer")).unwrap(),
                "owner draft"
            );
            assert!(!fixture.tmp.path().join("buffer").exists());
        }
    }
}
