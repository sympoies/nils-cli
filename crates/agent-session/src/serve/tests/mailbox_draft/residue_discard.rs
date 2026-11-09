use super::*;
use pretty_assertions::assert_eq;

fn age_and_clear(fixture: &WakeFixture, text: &str) {
    fs::write(fixture.temp.path().join("composer"), text).unwrap();
    fs::write(fixture.temp.path().join("cursor-x"), "2").unwrap();
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    crate::send_guard::drafts::observe_at(&fixture.state.context, &record, Some(text), 1, 0)
        .unwrap();
    fixture.check_draft().unwrap();
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        ""
    );
    assert!(!fixture.temp.path().join("submissions").exists());
}

#[tokio::test]
async fn runtime_injected_text_and_prefix_are_cleared_without_saving() {
    let _lock = GlobalStateLock::new();
    for text in ["a custom automation delivery", "a custom automation"] {
        let fixture = WakeFixture::new(AgentKind::Codex).await;
        let mut args = crate::send::unguarded_args(
            "beta".into(),
            Some("a custom automation delivery".into()),
            vec![],
            false,
            fixture.state.tmux_bin.clone(),
        );
        args.guards.notice_envelope = true;
        args.guards.require_empty_composer = true;
        crate::send::send_to_session(&fixture.state.context, args).unwrap();
        age_and_clear(&fixture, text);
        let record = load_session_record(&fixture.state.context, "beta").unwrap();
        assert!(
            crate::send_guard::drafts::list(&fixture.state.context, &record)
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn fallback_notice_residue_is_cleared_without_saving() {
    let _lock = GlobalStateLock::new();
    let mut samples = crate::notice::labels::NOTICE_LABELS
        .iter()
        .map(|entry| format!("{}: queued delivery", entry.label))
        .collect::<Vec<_>>();
    samples.extend([
        "Mailbox wake: queued delivery".into(),
        "Mailbox: 1 handoff, 2 report; agent-session message inbox".into(),
        "<!-- agent-notice:v1 kind=schedule.tick source=fixture -->".into(),
    ]);
    for text in samples {
        let fixture = WakeFixture::new(AgentKind::Claude).await;
        age_and_clear(&fixture, &text);
        let record = load_session_record(&fixture.state.context, "beta").unwrap();
        assert!(
            crate::send_guard::drafts::list(&fixture.state.context, &record)
                .unwrap()
                .is_empty(),
            "{text}"
        );
    }
}

#[tokio::test]
async fn user_text_with_title_mid_line_is_saved_exactly() {
    let _lock = GlobalStateLock::new();
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let text = "  Please explain Mailbox wake: behavior  ";
    age_and_clear(&fixture, text);
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    let drafts = crate::send_guard::drafts::list(&fixture.state.context, &record).unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts.into_iter().next().unwrap().text(), text);
}

fn discard_request(session: &str, id: &str, token: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/sessions/{session}/drafts/{id}/discard"));
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    request.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn discard_removes_saved_records_and_clears_projection() {
    let _lock = GlobalStateLock::new();
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    age_and_clear(&fixture, "first user draft");
    age_and_clear(&fixture, "second user draft");
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    let drafts = crate::send_guard::drafts::list(&fixture.state.context, &record).unwrap();
    let first = drafts[0].metadata("beta");
    let last = drafts[1].metadata("beta");
    let id = last["draft_id"].as_str().unwrap();
    let app = router(fixture.state.clone());
    let (status, _) = call(app.clone(), discard_request("beta", id, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, result) = call(app.clone(), discard_request("beta", id, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["data"]["discarded"], id);
    assert_eq!(result["data"]["remaining"], 1);
    assert_eq!(result["data"]["machine"], fixture.state.machine);
    let project = || {
        let board = crate::board::snapshot(
            &fixture.state.context,
            &fixture.state.tmux_bin,
            "fixture",
            false,
        )
        .unwrap();
        board["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["session_id"] == "beta")
            .unwrap()
            .clone()
    };
    let projected = project();
    assert_eq!(result["data"]["stale_draft"], projected["stale_draft"]);
    assert_eq!(projected["stale_draft"]["draft_id"], first["draft_id"]);
    let first_id = first["draft_id"].as_str().unwrap();
    let (status, result) = call(app.clone(), discard_request("beta", first_id, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["data"]["remaining"], 0);
    assert_eq!(result["data"]["stale_draft"], Value::Null);
    assert!(
        !session_dir(&fixture.state.context, "beta")
            .join("drafts/latest.json")
            .exists()
    );
    let (status, result) = call(app.clone(), get_auth("/sessions/beta/drafts", Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["data"]["drafts"], json!([]));
    assert!(project().get("stale_draft").is_none());
    let (status, result) = call(app, discard_request("beta", first_id, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(result["error"]["code"], "draft-not-found");
}

#[tokio::test]
async fn discard_unknown_and_invalid_ids_have_typed_errors() {
    let _lock = GlobalStateLock::new();
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let app = router(fixture.state.clone());
    for (session, id, status, code) in [
        (
            "beta",
            "00000000-0000-4000-8000-000000000000",
            StatusCode::NOT_FOUND,
            "draft-not-found",
        ),
        (
            "beta",
            "invalid",
            StatusCode::BAD_REQUEST,
            "invalid-draft-id",
        ),
        (
            "missing",
            "00000000-0000-4000-8000-000000000000",
            StatusCode::NOT_FOUND,
            "session-not-found",
        ),
    ] {
        let (actual, result) = call(app.clone(), discard_request(session, id, Some(TOKEN))).await;
        assert_eq!(actual, status);
        assert_eq!(result["error"]["code"], code);
    }
    let root = session_dir(&fixture.state.context, "beta").join("drafts");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("saved"), "unavailable store").unwrap();
    let (status, result) = call(
        app,
        discard_request("beta", "00000000-0000-4000-8000-000000000000", Some(TOKEN)),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(result["error"]["code"], "draft-store-unavailable");
}

#[tokio::test]
async fn edited_or_replaced_runtime_text_is_saved() {
    let _lock = GlobalStateLock::new();
    for replaced in [false, true] {
        let fixture = WakeFixture::new(AgentKind::Codex).await;
        let mut args = crate::send::unguarded_args(
            "beta".into(),
            Some("custom automation text".into()),
            vec![],
            false,
            fixture.state.tmux_bin.clone(),
        );
        args.guards.notice_envelope = true;
        args.guards.require_empty_composer = true;
        crate::send::send_to_session(&fixture.state.context, args).unwrap();
        let text = if replaced {
            let mut record = load_session_record(&fixture.state.context, "beta").unwrap();
            record.runtime.as_mut().unwrap().launch_id = "replacement-runtime".into();
            crate::write_session_record(&fixture.state.context, &record).unwrap();
            let path = fixture.temp.path().join("identities.json");
            let mut identity: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            identity["AGENT_SESSION_RUNTIME_ID"] = json!("replacement-runtime");
            fs::write(path, serde_json::to_vec(&identity).unwrap()).unwrap();
            "custom automation text"
        } else {
            "custom automation text with a user edit"
        };
        age_and_clear(&fixture, text);
        let record = load_session_record(&fixture.state.context, "beta").unwrap();
        let drafts = crate::send_guard::drafts::list(&fixture.state.context, &record).unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts.into_iter().next().unwrap().text(), text);
    }
}
