use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn stale_draft_is_saved_before_wake_for_both_providers() {
    let _lock = GlobalStateLock::new();
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        let fixture = WakeFixture::new(agent).await;
        let draft = "  café  owner draft\u{a0}";
        fs::write(fixture.temp.path().join("composer"), draft).unwrap();
        // The abandoned draft's caret is at its start, not the paste endpoint.
        fs::write(fixture.temp.path().join("cursor-x"), "2").unwrap();
        assert!(matches!(
            fixture.start(),
            Err(TerminalNotificationStartError::Known(
                "recipient-composer-not-empty"
            ))
        ));
        let observed = fixture
            .temp
            .path()
            .join("sessions/beta/drafts/observation.json");
        let mut observation: Value = serde_json::from_slice(
            &fs::read(&observed).expect("pending delivery observes the draft"),
        )
        .unwrap();
        observation["unchanged_since"] = json!(1);
        fs::write(&observed, serde_json::to_vec(&observation).unwrap()).unwrap();
        fixture.start().expect("stale draft permits the wake");
        assert_eq!(
            fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
            ""
        );
        let submissions = fs::read_to_string(fixture.temp.path().join("submissions")).unwrap();
        let submitted: Vec<String> = submissions
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            submitted,
            [crate::coordination::notification_prompt(&fixture.candidate)]
        );
        let drafts = fs::read_dir(fixture.temp.path().join("sessions/beta/drafts/saved"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(drafts.len(), 1);
        let saved: Value = serde_json::from_slice(&fs::read(&drafts[0]).unwrap()).unwrap();
        assert_eq!(saved["text"].as_str().unwrap().as_bytes(), draft.as_bytes());
        assert_eq!(saved["event"], "stale_draft_stashed");
        assert_eq!(saved["cleared"], true);
        let app = router(fixture.state.clone());
        let (status, _) = call(app.clone(), get("/sessions/beta/drafts")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, inventory) =
            call(app.clone(), get_auth("/sessions/beta/drafts", Some(TOKEN))).await;
        assert_eq!(status, StatusCode::OK);
        let listed = inventory["data"]["drafts"].as_array().unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].get("text").is_none());
        let id = listed[0]["draft_id"].as_str().unwrap();
        let board = crate::board::snapshot(
            &fixture.state.context,
            &fixture.state.tmux_bin,
            "fixture",
            false,
        )
        .unwrap();
        let row = board["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["session_id"] == "beta")
            .unwrap();
        assert_eq!(row["stale_draft"]["draft_id"], id);
        assert!(row["stale_draft"].get("text").is_none());
        let route = format!("/sessions/beta/drafts/{id}");
        let response = app.oneshot(get_auth(&route, Some(TOKEN))).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), draft.as_bytes());
    }
}

#[tokio::test]
async fn edited_draft_restarts_age_for_both_providers() {
    let _lock = GlobalStateLock::new();
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        let fixture = WakeFixture::new(agent).await;
        fs::write(fixture.temp.path().join("composer"), "first draft").unwrap();
        assert!(fixture.start().is_err());
        let observed = fixture
            .temp
            .path()
            .join("sessions/beta/drafts/observation.json");
        let mut observation: Value =
            serde_json::from_slice(&fs::read(&observed).expect("draft observation")).unwrap();
        observation["unchanged_since"] = json!(1);
        fs::write(&observed, serde_json::to_vec(&observation).unwrap()).unwrap();
        fs::write(fixture.temp.path().join("composer"), "edited draft").unwrap();
        assert!(fixture.start().is_err());
        assert_eq!(
            fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
            "edited draft"
        );
        assert!(!fixture.temp.path().join("submissions").exists());
    }
}

#[tokio::test]
async fn unobservable_pane_never_inherits_stale_draft_age() {
    let _lock = GlobalStateLock::new();
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    fs::write(fixture.temp.path().join("composer"), "owner draft").unwrap();
    assert!(fixture.start().is_err());
    let observed = fixture
        .temp
        .path()
        .join("sessions/beta/drafts/observation.json");
    let mut observation: Value =
        serde_json::from_slice(&fs::read(&observed).expect("draft observation")).unwrap();
    observation["unchanged_since"] = json!(1);
    fs::write(&observed, serde_json::to_vec(&observation).unwrap()).unwrap();
    fs::write(fixture.temp.path().join("capture-unavailable"), "").unwrap();
    assert!(matches!(
        fixture.start(),
        Err(TerminalNotificationStartError::Known(
            "recipient-composer-unobservable"
        ))
    ));
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        "owner draft"
    );
    assert!(!observed.exists());
    assert!(!fixture.temp.path().join("submissions").exists());
}

#[tokio::test]
async fn zero_bound_keeps_owner_draft_ineligible() {
    let lock = GlobalStateLock::new();
    let _bound = EnvGuard::set(&lock, "AGENT_SESSION_STALE_DRAFT_SECONDS", "0");
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        let fixture = WakeFixture::new(agent).await;
        let draft = "owner draft";
        fs::write(fixture.temp.path().join("composer"), draft).unwrap();
        let record = load_session_record(&fixture.state.context, "beta").unwrap();
        assert!(
            !crate::send_guard::drafts::observe_at(
                &fixture.state.context,
                &record,
                Some(draft),
                600,
                1
            )
            .unwrap()
        );
        assert!(matches!(
            fixture.start(),
            Err(TerminalNotificationStartError::Known(
                "recipient-composer-not-empty"
            ))
        ));
        assert_eq!(
            fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
            draft
        );
        assert!(
            !fixture
                .temp
                .path()
                .join("sessions/beta/drafts/observation.json")
                .exists()
        );
        assert!(!fixture.temp.path().join("submissions").exists());
        assert!(
            !fixture
                .temp
                .path()
                .join("sessions/beta/drafts/saved")
                .exists()
        );
    }
}

#[tokio::test]
async fn full_draft_store_refuses_before_cleanup_keys() {
    let _lock = GlobalStateLock::new();
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    fs::write(fixture.temp.path().join("composer"), "owner draft").unwrap();
    assert!(fixture.start().is_err());
    let root = fixture.temp.path().join("sessions/beta/drafts");
    let observed = root.join("observation.json");
    let mut observation: Value = serde_json::from_slice(&fs::read(&observed).unwrap()).unwrap();
    observation["unchanged_since"] = json!(1);
    fs::write(&observed, serde_json::to_vec(&observation).unwrap()).unwrap();
    fs::create_dir(root.join("saved")).unwrap();
    for _ in 0..128 {
        let id = uuid::Uuid::new_v4().to_string();
        let saved = json!({"schema_version":"agent-session.saved-draft.v1", "event":"stale_draft_stashed", "draft_id":id,
            "timestamp":"2000-01-01T00:00:00Z", "session_incarnation":"modeled-beta", "text":"prior draft", "cleared":true});
        fs::write(
            root.join("saved").join(format!("{id}.json")),
            serde_json::to_vec(&saved).unwrap(),
        )
        .unwrap();
    }
    assert!(fixture.start().is_err());
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        "owner draft"
    );
    assert!(!fixture.temp.path().join("submissions").exists());
    let calls = fs::read_to_string(fixture.temp.path().join("calls")).unwrap();
    assert!(
        calls
            .lines()
            .map(|line| serde_json::from_str::<Vec<String>>(line).unwrap())
            .all(|args| args[0] != "send-keys")
    );
}

#[tokio::test]
async fn changed_draft_after_stash_is_never_erased_or_submitted() {
    let _lock = GlobalStateLock::new();
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        let fixture = WakeFixture::new(agent).await;
        fs::write(fixture.temp.path().join("composer"), "owner draft").unwrap();
        assert!(fixture.start().is_err());
        let observed = fixture
            .temp
            .path()
            .join("sessions/beta/drafts/observation.json");
        let mut observation: Value = serde_json::from_slice(&fs::read(&observed).unwrap()).unwrap();
        observation["unchanged_since"] = json!(1);
        fs::write(&observed, serde_json::to_vec(&observation).unwrap()).unwrap();
        fs::write(fixture.temp.path().join("edit-after-stash"), "").unwrap();
        assert!(fixture.start().is_err());
        assert_eq!(
            fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
            "owner  draft"
        );
        assert!(!fixture.temp.path().join("submissions").exists());
        let calls = fs::read_to_string(fixture.temp.path().join("calls")).unwrap();
        assert!(
            calls
                .lines()
                .map(|line| serde_json::from_str::<Vec<String>>(line).unwrap())
                .all(|args| args[0] != "send-keys")
        );
        let record = load_session_record(&fixture.state.context, "beta").unwrap();
        let saved = crate::send_guard::drafts::list(&fixture.state.context, &record).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].metadata()["cleared"], false);
    }
}

#[tokio::test]
async fn direct_reply_and_guarded_send_share_stale_draft_admission() {
    let _lock = GlobalStateLock::new();
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        for direct in [true, false] {
            let fixture = WakeFixture::new(agent).await;
            let draft = "owner draft";
            fs::write(fixture.temp.path().join("composer"), draft).unwrap();
            fs::write(
                fixture.temp.path().join("cursor-x"),
                (2 + draft.chars().count()).to_string(),
            )
            .unwrap();
            let record = load_session_record(&fixture.state.context, "beta").unwrap();
            let args = crate::send_guard::GuardArgs {
                require_empty_composer: true,
                ..Default::default()
            };
            let check = || {
                if direct {
                    crate::send_guard::require_empty_composer(
                        &fixture.state.context,
                        &record,
                        &fixture.state.tmux_bin,
                    )
                } else {
                    crate::send_guard::Admission {
                        context: &fixture.state.context,
                        record: &record,
                        args: &args,
                        tmux_bin: &fixture.state.tmux_bin,
                        verify_composer: true,
                        deadline: Instant::now() + Duration::from_secs(10),
                    }
                    .check(None, "admission")
                }
            };
            let refusal = check().expect_err("fresh composer must refuse before stashing");
            assert_eq!(
                refusal.code(),
                if direct {
                    "composer-not-empty"
                } else {
                    "send-composer-not-empty"
                },
                "{refusal:?}"
            );
            let observed = fixture
                .temp
                .path()
                .join("sessions/beta/drafts/observation.json");
            let mut observation: Value =
                serde_json::from_slice(&fs::read(&observed).unwrap()).unwrap();
            observation["unchanged_since"] = json!(1);
            fs::write(&observed, serde_json::to_vec(&observation).unwrap()).unwrap();
            check().expect("stale draft cleared at existing admission boundary");
            assert_eq!(
                fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
                ""
            );
            assert!(!fixture.temp.path().join("submissions").exists());
            assert_eq!(
                crate::send_guard::drafts::list(&fixture.state.context, &record)
                    .unwrap()
                    .len(),
                1
            );
        }
    }
}

#[tokio::test]
async fn hidden_trailing_spaces_keep_the_composer_untouched() {
    let _lock = GlobalStateLock::new();
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    let draft = "owner draft  ";
    fs::write(fixture.temp.path().join("composer"), draft).unwrap();
    assert!(fixture.start().is_err());
    let observed = fixture
        .temp
        .path()
        .join("sessions/beta/drafts/observation.json");
    let mut observation: Value = serde_json::from_slice(&fs::read(&observed).unwrap()).unwrap();
    observation["unchanged_since"] = json!(1);
    fs::write(&observed, serde_json::to_vec(&observation).unwrap()).unwrap();
    assert!(fixture.start().is_err());
    assert_eq!(
        fs::read_to_string(fixture.temp.path().join("composer")).unwrap(),
        draft
    );
    assert!(!fixture.temp.path().join("submissions").exists());
}

#[tokio::test]
async fn default_bound_requires_ten_minutes_and_same_runtime() {
    let _lock = GlobalStateLock::new();
    let fixture = WakeFixture::new(AgentKind::Codex).await;
    let mut record = load_session_record(&fixture.state.context, "beta").unwrap();
    let bound = crate::send_guard::drafts::DEFAULT_BOUND;
    assert!(
        !crate::send_guard::drafts::observe_at(
            &fixture.state.context,
            &record,
            Some("draft"),
            bound,
            1000
        )
        .unwrap()
    );
    assert!(
        !crate::send_guard::drafts::observe_at(
            &fixture.state.context,
            &record,
            Some("draft"),
            bound,
            1600
        )
        .unwrap()
    );
    assert!(
        crate::send_guard::drafts::observe_at(
            &fixture.state.context,
            &record,
            Some("draft"),
            bound,
            1601
        )
        .unwrap()
    );
    record.runtime.as_mut().unwrap().launch_id = "replacement-runtime".to_string();
    assert!(
        !crate::send_guard::drafts::observe_at(
            &fixture.state.context,
            &record,
            Some("draft"),
            bound,
            1602
        )
        .unwrap()
    );
}
