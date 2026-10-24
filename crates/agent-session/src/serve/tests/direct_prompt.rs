//! Direct structured replies must preserve an owner draft before any dispatch.
use super::*;
use pretty_assertions::assert_eq;
use std::sync::atomic::{AtomicUsize, Ordering};

fn codex_pane_after_backspace(working: bool) -> String {
    format!(
        "{}› \u{1b}[2mAsk Codex to do anything\u{1b}[22m   \n\n  status",
        if working {
            "• Working (esc to interrupt)\n\n"
        } else {
            ""
        }
    )
}

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
        let script = fs::read_to_string(&tmux).unwrap().replacen(
            "print('────────────────────────────\\n❯ ' + text + '\\n────────────────────────────')",
            "if record['agent'] == 'codex': print('› ' + text + '\\n\\n  status')\n    else: print('────────────────────────────\\n❯ ' + text + '\\n────────────────────────────')",
            1,
        ).replacen(
            "if args[0] == 'capture-pane':\n",
            "if args[0] == 'capture-pane':\n    if (root / 'capture-fails').exists(): sys.exit(1)\n    if (root / 'pane-override').exists(): print((root / 'pane-override').read_text()); sys.exit(0)\n",
            1,
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

    fn arm_recovery(&self) -> Vec<u8> {
        crate::auto_resume::rearm_usage_exhaustion_for_runtime(
            &self.state.context,
            "direct-reply",
            &self.launch,
            "interrupted-turn".into(),
            1,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        let record = load_session_record(&self.state.context, "direct-reply").unwrap();
        let recovery = crate::auto_resume::view_for_record(&self.state.context, &record);
        assert!(recovery.enabled);
        assert_eq!(recovery.state, "armed");
        fs::read(
            self.tmp
                .path()
                .join("sessions/direct-reply/auto-resume.json"),
        )
        .unwrap()
    }

    fn assert_recovery_unchanged(&self, expected: &[u8]) {
        assert_eq!(
            fs::read(
                self.tmp
                    .path()
                    .join("sessions/direct-reply/auto-resume.json")
            )
            .unwrap(),
            expected
        );
        let record = load_session_record(&self.state.context, "direct-reply").unwrap();
        let recovery = crate::auto_resume::view_for_record(&self.state.context, &record);
        assert!(recovery.enabled);
        assert_eq!(recovery.state, "armed");
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
        let recovery = fixture.arm_recovery();
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
        fixture.assert_recovery_unchanged(&recovery);
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
            fixture.assert_recovery_unchanged(&recovery);
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
        if provider == "codex" {
            fs::write(
                fixture.tmp.path().join("pane-override"),
                codex_pane_after_backspace(false),
            )
            .unwrap();
        }
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
            let recovery = fixture.arm_recovery();
            fs::write(fixture.tmp.path().join("capture-fails"), "").unwrap();
            fs::write(fixture.tmp.path().join("composer"), "owner draft").unwrap();
            let (status, result) = fixture.submit(route, fixture.body(route)).await;
            assert_eq!(status, StatusCode::CONFLICT, "{provider} {route}: {result}");
            assert_eq!(result["error"]["code"], "composer-unobservable");
            fixture.assert_recovery_unchanged(&recovery);
            assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 0);
            assert_eq!(
                fs::read_to_string(fixture.tmp.path().join("composer")).unwrap(),
                "owner draft"
            );
            assert!(!fixture.tmp.path().join("buffer").exists());
        }
    }
}

#[tokio::test]
async fn direct_codex_numbered_dialog_refuses_until_observably_empty() {
    for (provider, pane) in [(
        "codex",
        "Select an option\n› 1. Default\n  2. Alternate\n\n  enter to confirm · esc to cancel",
    )] {
        for route in ["prompt", "prompt/v2"] {
            let fixture = DirectFixture::new(provider);
            let recovery = fixture.arm_recovery();
            let pane_path = fixture.tmp.path().join("pane-override");
            fs::write(&pane_path, pane).unwrap();
            let mut body = fixture.body(route);
            let (status, refusal) = fixture.submit(route, body.clone()).await;
            assert_eq!(
                status,
                StatusCode::CONFLICT,
                "{provider} {route}: {refusal}"
            );
            assert_eq!(refusal["error"]["code"], "composer-unobservable");
            assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 0);
            assert_eq!(
                fs::read_to_string(fixture.tmp.path().join("composer")).unwrap(),
                ""
            );
            assert!(!fixture.tmp.path().join("buffer").exists());
            assert!(!fixture.tmp.path().join("submitted").exists());
            fixture.assert_recovery_unchanged(&recovery);
            // Resolve the provider UI without typing into or clearing its input.
            fs::remove_file(pane_path).unwrap();
            if route == "prompt/v2" {
                assert_eq!(refusal["error"]["details"]["receipt"]["outcome"], "refused");
                let (_, replay) = fixture.submit(route, body.clone()).await;
                assert_eq!(replay, refusal);
                fixture.assert_recovery_unchanged(&recovery);
                body["request_id"] = json!("direct-attempt-2");
            }
            let (status, result) = fixture.submit(route, body).await;
            assert_eq!(status, StatusCode::OK, "{provider} {route}: {result}");
            assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn direct_working_codex_with_empty_composer_delivers() {
    for route in ["prompt", "prompt/v2"] {
        let fixture = DirectFixture::new("codex");
        fs::write(
            fixture.tmp.path().join("pane-override"),
            codex_pane_after_backspace(true),
        )
        .unwrap();
        let (status, result) = fixture.submit(route, fixture.body(route)).await;
        assert_eq!(status, StatusCode::OK, "{route}: {result}");
        assert_eq!(result["data"]["submitted"], true);
        assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn direct_claude_second_reply_after_queued_hint_delivers() {
    for route in ["prompt", "prompt/v2"] {
        let fixture = DirectFixture::new("claude");
        crate::activity::ingest_event(
            &fixture.state.context,
            "direct-reply",
            claude_turn_started_event(&fixture.launch, "earlier-start", "earlier-turn"),
        )
        .unwrap();
        fs::write(fixture.tmp.path().join("pane-override"),
            "earlier queued reply\n────────────────────────────\n❯ \x1b[2mPress up to edit queued messages\x1b[22m\n────────────────────────────\n  status").unwrap();
        let (status, result) = fixture.submit(route, fixture.body(route)).await;
        assert_eq!(status, StatusCode::OK, "{route}: {result}");
        assert_eq!(result["data"]["submitted"], true);
        assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 1);
        assert_eq!(
            fs::read_to_string(fixture.tmp.path().join("submitted")).unwrap(),
            "inspect the workspace"
        );
    }
}

#[tokio::test]
async fn direct_claude_owner_draft_beside_queued_hint_refuses() {
    for (draft, displayed) in [
        (
            "owner draft",
            "owner draft \x1b[2mPress up to edit queued messages\x1b[22m",
        ),
        (
            "Press up to edit queued messages",
            "Press up to edit queued messages",
        ),
    ] {
        for route in ["prompt", "prompt/v2"] {
            let fixture = DirectFixture::new("claude");
            let recovery = fixture.arm_recovery();
            fs::write(fixture.tmp.path().join("composer"), draft).unwrap();
            fs::write(fixture.tmp.path().join("pane-override"), format!(
                "earlier queued reply\n────────────────────────────\n❯ {displayed}\n────────────────────────────\n  status")).unwrap();
            let (status, result) = fixture.submit(route, fixture.body(route)).await;
            assert_eq!(status, StatusCode::CONFLICT, "{route}: {result}");
            assert_eq!(result["error"]["code"], "composer-not-empty");
            assert_eq!(
                fs::read_to_string(fixture.tmp.path().join("composer")).unwrap(),
                draft
            );
            assert_eq!(fixture.dispatched.load(Ordering::SeqCst), 0);
            assert!(!fixture.tmp.path().join("buffer").exists());
            fixture.assert_recovery_unchanged(&recovery);
        }
    }
}
