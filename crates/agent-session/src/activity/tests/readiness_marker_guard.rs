use super::*;
use pretty_assertions::assert_eq;

fn fixture(
    agent: AgentKind,
    marker: &str,
    phase: TurnPhase,
) -> (tempfile::TempDir, CliContext, SessionRecord, Value) {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session_for_agent(&tmp, agent);
    activate_runtime(&context, &created.record).unwrap();
    let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let mut document = read_document(&path).unwrap();
    document.state.phase = phase;
    document.state.last_turn = Some(LastTurn {
        provider_turn_id: Some("previous-turn".into()),
        started_at: Some("2026-07-10T00:00:00Z".into()),
        completed_at: "2026-07-10T00:00:01Z".into(),
        outcome: "failed".into(),
        extra: serde_json::from_value(
            json!({"provider_failure_kind": marker, "other": "preserved"}),
        )
        .unwrap(),
    });
    write_document(&path, &mut document).unwrap();
    let before = serde_json::to_value(&document).unwrap();
    (tmp, context, created.record, before)
}

fn snapshot(context: &CliContext, record: &SessionRecord) -> Value {
    let path = session_dir(context, &record.id).join(ACTIVITY_FILE);
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn claude_readiness_clears_only_authentication_marker() {
    for marker in ["authentication", "usage_exhausted"] {
        let (_tmp, context, record, before) =
            fixture(AgentKind::Claude, marker, TurnPhase::Starting);
        assert!(settle_starting(&context, &record, None).unwrap());
        let after = snapshot(&context, &record);
        let mut expected = before["state"]["last_turn"].clone();
        if marker == "authentication" {
            expected
                .as_object_mut()
                .unwrap()
                .remove("provider_failure_kind");
        }
        assert_eq!(after["state"]["last_turn"], expected);
        assert_eq!(after["state"]["phase"], "waiting");
    }
}

#[test]
fn non_claude_readiness_preserves_authentication_marker() {
    let (_tmp, context, record, before) =
        fixture(AgentKind::Codex, "authentication", TurnPhase::Starting);
    assert!(settle_starting(&context, &record, None).unwrap());
    let after = snapshot(&context, &record);
    assert_eq!(after["state"]["last_turn"], before["state"]["last_turn"]);
    assert_eq!(after["state"]["phase"], "waiting");
}

#[test]
fn claude_readiness_outside_starting_preserves_authentication_marker() {
    for phase in [
        TurnPhase::Waiting,
        TurnPhase::Working,
        TurnPhase::NeedsInput,
        TurnPhase::Unresponsive,
        TurnPhase::Unknown,
    ] {
        let (_tmp, context, record, before) = fixture(AgentKind::Claude, "authentication", phase);
        let settled = settle_starting(&context, &record, None).unwrap();
        assert_eq!(snapshot(&context, &record), before);
        assert!(!settled);
    }
}
