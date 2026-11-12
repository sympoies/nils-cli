use super::*;
use pretty_assertions::assert_eq;

pub(super) fn bind_idle(fixture: &WakeFixture) {
    bind_transcript(fixture);
    let path = fixture.temp.path().join("sessions/beta/activity.json");
    let mut activity: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    activity["state"]["phase"] = json!("waiting");
    activity["state"]["current_turn"] = Value::Null;
    fs::write(path, serde_json::to_vec(&activity).unwrap()).unwrap();
}

#[tokio::test]
async fn claude_configuration_fallback_preserves_drafts_and_delivers_once() {
    use crate::send::claude_notice::attach::{self, Reason};
    let lock = GlobalStateLock::new();
    for reason in [Reason::WritersDisabled, Reason::ProviderVersionUnsupported, Reason::ProviderVersionUnreadable] {
        let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
        let fixture = WakeFixture::new(AgentKind::Claude).await;
        bind_idle(&fixture);
        let record = load_session_record(&fixture.state.context, "beta").unwrap();
        attach::record(&fixture.state.context.state_dir, &record, reason, None);
        // A launch without writers stays unattached even if the daemon now enables them.
        let detail = match reason {
            Reason::WritersDisabled => "notice-writers-disabled",
            Reason::ProviderVersionUnsupported | Reason::ProviderVersionUnreadable => "notice-provider-unsupported",
        };
        let draft = "generic operator draft";
        fs::write(fixture.temp.path().join("composer"), draft).unwrap();
        assert_eq!(fixture.check_draft().unwrap_err().code(), "composer-not-empty");
        let observation_path = fixture.temp.path().join("sessions/beta/drafts/observation.json");
        let mut observation: Value = serde_json::from_slice(&fs::read(&observation_path).unwrap()).unwrap();
        observation["unchanged_since"] = json!(1);
        fs::write(observation_path, serde_json::to_vec(&observation).unwrap()).unwrap();
        drain_coordination_notifications(fixture.state.clone()).await;
        fixture.assert_no_input(draft);
        assert!(!fixture.temp.path().join("sessions/beta/drafts/saved").exists());
        let receipt = notification_fixture(fixture.temp.path());
        assert_eq!(receipt["state"], "queued");
        assert_eq!(receipt["last_reason_detail"], detail, "{receipt}");
        fs::write(fixture.temp.path().join("composer"), "").unwrap();
        // A draft deferral has a retry deadline; make it due without changing the generation.
        let registry_path = fixture.temp.path().join("coordination/registry.json");
        let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
        for receipt in registry["notifications"].as_object_mut().unwrap().values_mut() {
            receipt["next_attempt_at_epoch"] = json!(0);
        }
        fs::write(registry_path, serde_json::to_vec(&registry).unwrap()).unwrap();
        drain_coordination_notifications(fixture.state.clone()).await;
        let receipt = notification_fixture(fixture.temp.path());
        assert_eq!(receipt["state"], "prompt_submitted", "{receipt}");
        assert_eq!(receipt["last_reason_detail"], detail);
        assert_eq!(receipt["notified_generation"], 1);
        crate::serve::terminal_notification::dispatch_coordination_notification(fixture.state.clone(), fixture.candidate.clone()).await;
        assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions")).unwrap().lines().count(), 1);
    }
}

#[tokio::test]
async fn claude_configuration_fallback_with_daemon_writers_disabled() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    drain_coordination_notifications(fixture.state.clone()).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "prompt_submitted", "{receipt}");
    assert_eq!(receipt["last_reason_detail"], "notice-writers-disabled");
    crate::serve::terminal_notification::dispatch_coordination_notification(fixture.state.clone(), fixture.candidate.clone()).await;
    assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions")).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn claude_configuration_fallback_unknown_submission_never_resends() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    fs::write(fixture.temp.path().join("capture-unavailable-after-enter"), "").unwrap();
    drain_coordination_notifications(fixture.state.clone()).await;
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "attempt_unknown", "{receipt}");
    assert_eq!(receipt["attempted_generation"], 1);
    assert_eq!(receipt["last_reason_detail"], "notice-writers-disabled");
    crate::serve::terminal_notification::dispatch_coordination_notification(fixture.state.clone(), fixture.candidate.clone()).await;
    assert_eq!(fs::read_to_string(fixture.temp.path().join("submissions")).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn claude_configuration_fallback_preserves_a_draft_typed_during_load() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::remove(&lock, crate::notice::WRITERS_ENV);
    let fixture = WakeFixture::new(AgentKind::Claude).await;
    bind_idle(&fixture);
    fs::write(fixture.temp.path().join("draft-on-load"), "").unwrap();
    drain_coordination_notifications(fixture.state.clone()).await;
    fixture.assert_no_input("later operator draft");
    let receipt = notification_fixture(fixture.temp.path());
    assert_eq!(receipt["state"], "queued", "{receipt}");
    assert_eq!(receipt["attempted_generation"], 0);
    assert_eq!(receipt["last_reason_detail"], "notice-writers-disabled");
}
