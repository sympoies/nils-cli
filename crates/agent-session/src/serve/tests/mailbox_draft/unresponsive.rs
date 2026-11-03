use super::*;
use pretty_assertions::{assert_eq, assert_ne};

async fn waiting_fixture(agent: AgentKind) -> WakeFixture {
    let fixture = WakeFixture::new(agent).await;
    let path = fixture.temp.path().join("sessions/beta/activity.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["state"]["phase"] = json!("waiting");
    value["state"]["current_turn"] = Value::Null;
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(phase(&fixture), "waiting");
    fixture
}

fn age_key_observation(fixture: &WakeFixture) {
    let path = fixture.temp.path().join("sessions/beta/pane-response.json");
    // Before the implementation there is no response observation to age; the
    // observable regression remains the reported waiting phase below.
    if path.exists() {
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["started_at"] = json!("2000-01-01T00:00:00Z");
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
}

fn phase(fixture: &WakeFixture) -> Value {
    let record = load_session_record(&fixture.state.context, "beta").unwrap();
    serde_json::to_value(crate::activity::state_for_view(&fixture.state.context, &record))
        .unwrap()["phase"]
        .clone()
}

#[tokio::test]
async fn unresponsive_guarded_pane_reports_status_event_and_clears_on_change() {
    let _lock = GlobalStateLock::new();
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        let fixture = waiting_fixture(agent).await;
        fs::write(fixture.temp.path().join("swallow-every-enter"), "").unwrap();
        fs::write(fixture.temp.path().join("ignore-clear"), "").unwrap();
        assert!(fixture.start().is_err());
        age_key_observation(&fixture);
        assert_eq!(phase(&fixture), "unresponsive", "{agent:?}");
        let board = crate::board::snapshot(
            &fixture.state.context, &fixture.state.tmux_bin, "fixture", false,
        ).unwrap();
        let row = board["records"].as_array().unwrap().iter()
            .find(|row| row["session_id"] == "beta").unwrap();
        assert_eq!(row["turn_state"]["phase"], "unresponsive");
        assert_eq!(row["pane_response"]["event"], "pane_unresponsive");
        assert_eq!(row["pane_response"]["operation"], "clear");
        assert_eq!(row["pane_response"]["bound_seconds"], 4);
        assert_eq!(row["pane_response"]["session_id"], "beta");
        assert!(row["pane_response"]["observed_at"].is_string());
        assert!(!row["pane_response"].to_string().contains("Mailbox"));
        let calls = fs::read(fixture.temp.path().join("calls")).unwrap();
        fs::write(fixture.temp.path().join("footer"), "pane changed").unwrap();
        assert_eq!(phase(&fixture), "waiting");
        // Observation performs no recovery key or restart.
        let after = fs::read_to_string(fixture.temp.path().join("calls")).unwrap();
        let prior = String::from_utf8(calls).unwrap();
        assert!(!after[prior.len()..].contains("send-keys"));
    }
}

#[tokio::test]
async fn unresponsive_guarded_submit_uses_configured_bound_without_new_key_retries() {
    let lock = GlobalStateLock::new();
    let _bound = EnvGuard::set(&lock, "AGENT_SESSION_UNRESPONSIVE_SECONDS", "9");
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        let fixture = waiting_fixture(agent).await;
        fs::write(fixture.temp.path().join("composer"), "pending prompt").unwrap();
        fs::write(fixture.temp.path().join("swallow-every-enter"), "").unwrap();
        let mut args = crate::send::unguarded_args("beta".into(), None,
            vec![SpecialKey::Enter], false, fixture.state.tmux_bin.clone());
        args.guards.require_idle = true;
        crate::send::send_to_session(&fixture.state.context, args).unwrap();
        assert_eq!(phase(&fixture), "waiting");
        age_key_observation(&fixture);
        assert_eq!(phase(&fixture), "unresponsive");
        let event: Value = serde_json::from_slice(&fs::read(
            fixture.temp.path().join("sessions/beta/pane-response-event.json")
        ).unwrap()).unwrap();
        assert_eq!(event["operation"], "submit");
        assert_eq!(event["bound_seconds"], 9);
        assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions"))
            .unwrap().lines().count(), 1);
        fs::write(fixture.temp.path().join("composer"), "new text").unwrap();
        assert_eq!(phase(&fixture), "waiting");
    }
}

#[tokio::test]
async fn unresponsive_observation_excludes_responsive_unobservable_and_busy_panes() {
    let _lock = GlobalStateLock::new();
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        let responsive = waiting_fixture(agent).await;
        responsive.start().unwrap();
        age_key_observation(&responsive);
        assert_eq!(phase(&responsive), "waiting");

        let fixture = waiting_fixture(agent).await;
        fs::write(fixture.temp.path().join("swallow-every-enter"), "").unwrap();
        fs::write(fixture.temp.path().join("ignore-clear"), "").unwrap();
        assert!(fixture.start().is_err());
        age_key_observation(&fixture);
        fs::write(fixture.temp.path().join("capture-unavailable"), "").unwrap();
        assert_eq!(phase(&fixture), "waiting");
        fs::remove_file(fixture.temp.path().join("capture-unavailable")).unwrap();
        assert_eq!(phase(&fixture), "waiting", "lost observation discards evidence");

        let busy = waiting_fixture(agent).await;
        fs::write(busy.temp.path().join("swallow-every-enter"), "").unwrap();
        fs::write(busy.temp.path().join("ignore-clear"), "").unwrap();
        assert!(busy.start().is_err());
        age_key_observation(&busy);
        let path = busy.temp.path().join("sessions/beta/activity.json");
        let mut state: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        state["state"]["phase"] = json!("working");
        fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
        assert_ne!(phase(&busy), "unresponsive");
    }
}

#[tokio::test]
async fn observer_failure_does_not_block_guarded_submit() {
    let lock = GlobalStateLock::new();
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        for fail_write in [false, true] {
            let fixture = waiting_fixture(agent).await;
            let _bound = EnvGuard::set(&lock, "AGENT_SESSION_UNRESPONSIVE_SECONDS",
                if fail_write { "4" } else { "invalid" });
            if fail_write {
                fs::create_dir(fixture.temp.path().join("sessions/beta/pane-response.json")).unwrap();
            }
            fs::write(fixture.temp.path().join("composer"), "pending prompt").unwrap();
            let mut args = crate::send::unguarded_args("beta".into(), None,
                vec![SpecialKey::Enter], false, fixture.state.tmux_bin.clone());
            args.guards.require_idle = true;
            crate::send::send_to_session(&fixture.state.context, args)
                .expect("passive observer failure must not block the existing Enter");
            assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions"))
                .unwrap().lines().count(), 1);
            assert_eq!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap(), "");
            assert_eq!(phase(&fixture), "waiting");
            assert!(!fixture.temp.path().join("sessions/beta/pane-response-event.json").exists());
        }
    }
}

#[tokio::test]
async fn observer_failure_does_not_block_shared_clear() {
    let lock = GlobalStateLock::new();
    for agent in [AgentKind::Claude, AgentKind::Codex] {
        for fail_write in [false, true] {
            let fixture = waiting_fixture(agent).await;
            let _bound = EnvGuard::set(&lock, "AGENT_SESSION_UNRESPONSIVE_SECONDS",
                if fail_write { "4" } else { "invalid" });
            if fail_write {
                fs::create_dir(fixture.temp.path().join("sessions/beta/pane-response.json")).unwrap();
            }
            let text = "owned prompt";
            fs::write(fixture.temp.path().join("composer"), text).unwrap();
            fs::write(fixture.temp.path().join("cursor-x"), (2 + text.len()).to_string()).unwrap();
            let record = load_session_record(&fixture.state.context, "beta").unwrap();
            let args = crate::send_guard::GuardArgs { require_empty_composer: true, ..Default::default() };
            let admission = crate::send_guard::Admission {
                context: &fixture.state.context, record: &record, args: &args,
                tmux_bin: &fixture.state.tmux_bin, verify_composer: true,
                deadline: Instant::now() + Duration::from_secs(10),
            };
            let error = admission.report_pasted_input(
                CliError::runtime("fixture-refusal", "existing guard refusal", None), text, true,
            );
            assert_eq!(error.code(), "fixture-refusal");
            assert_eq!(error.0.details.as_ref().unwrap()["cleared"], true,
                "passive observer failure must not block the existing cleanup");
            let calls = fs::read_to_string(fixture.temp.path().join("calls")).unwrap();
            assert_eq!(calls.matches("\"BSpace\"").count(), 1);
            assert_eq!(fs::read_to_string(fixture.temp.path().join("composer")).unwrap(), "");
            assert_eq!(phase(&fixture), "waiting");
            assert!(!fixture.temp.path().join("sessions/beta/pane-response-event.json").exists());
        }
    }
}
