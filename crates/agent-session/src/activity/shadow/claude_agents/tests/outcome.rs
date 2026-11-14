use super::*;
use pretty_assertions::assert_eq;

const OUTCOMES: &str =
    include_str!("../../../../../tests/fixtures/claude-agents/unresolved-outcomes.json");

fn pending_approval(sampled: &Sampled) {
    let event: activity::TurnEvent = serde_json::from_value(json!({
        "schema_version":"agent-session.turn-event.v1", "event_id":"approval",
        "runtime_id":"launch-a", "provider":"claude", "provider_turn_id":"turn-1",
        "provider_session_id":IDLE_ID, "kind":"attention_requested",
        "attention_id":"approval-1", "attention_kind":"approval",
        "confidence":"observed", "source_kind":"provider_hook"
    }))
    .unwrap();
    activity::ingest_event(&sampled.context, &sampled.record.id, event).unwrap();
}

fn unresolved_outcome(name: &str) {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let fixtures: serde_json::Value = serde_json::from_str(OUTCOMES).unwrap();
    let fixture = fixtures["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .find(|fixture| fixture["name"] == name)
        .unwrap();
    let (bin, _) = counting_claude_printing(tmp.path(), &fixture["listing"].to_string());
    let _bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", bin.to_str().unwrap());
    let _config = EnvGuard::remove(&lock, "CLAUDE_CONFIG_DIR");
    let sampled = running_turn(IDLE_ID);
    if fixture["approval_pending"] == true {
        pending_approval(&sampled);
    }
    let durable = activity::controller_state_for_record(&sampled.context, &sampled.record).unwrap();
    if fixture["approval_pending"] == true {
        // Feed the documented first idle observation directly so the regression
        // asserts the wrong outcome, independently of sampler eligibility.
        let document = ShadowDocument {
            schema_version: super::super::super::SHADOW_DOCUMENT_VERSION.into(),
            runtime_id: "launch-a".into(),
            runtime_generation: 1,
            activity_revision: Some(durable.revision),
            interrupt_since: None,
            observation: observation(
                parse(fixture["listing"].to_string().as_bytes())
                    .unwrap()
                    .state(IDLE_ID)
                    .unwrap(),
                &durable.phase,
            ),
        };
        fs::write(sampled.path(), serde_json::to_vec(&document).unwrap()).unwrap();
    } else {
        sampled.collect(true);
    }
    let document = sampled.wait_for_document(|_| true);
    assert_eq!(document.observation.rule_id, "claude_agents_idle");
    let view = sampled.collect(false);
    assert_eq!(
        view.phase,
        activity::TurnPhase::Unknown,
        "{name}: first idle sample"
    );
    assert_eq!(
        view.diagnostic.as_ref().unwrap().reason,
        "claude_turn_outcome_unknown"
    );
    assert_eq!(view.current_turn, durable.current_turn);
    assert_eq!(view.last_turn, durable.last_turn);
    assert_eq!(view.revision, durable.revision);
    assert_eq!(view.source.kind, activity::SourceKind::ConsoleObservation);
    assert_eq!(view.source.confidence, activity::Confidence::Observed);
    assert!(!sampled.captures.exists());
    let status = activity::state_for_view(&sampled.context, &sampled.record).unwrap();
    assert_eq!(status.phase, activity::TurnPhase::Unknown);
    assert_eq!(status.diagnostic, view.diagnostic);
    assert_eq!(
        activity::controller_state_for_record(&sampled.context, &sampled.record).unwrap(),
        durable,
        "presentation uncertainty must not change control or persisted attention"
    );
}

#[test]
fn interrupted_turn_is_unknown_on_first_structured_idle() {
    unresolved_outcome("interrupted_turn");
}

#[test]
fn manually_denied_approval_is_unknown_on_first_structured_idle() {
    unresolved_outcome("manually_denied_approval");
}

#[test]
fn long_genuine_idle_does_not_become_suspected_interrupt() {
    let sampled = running_turn(IDLE_ID);
    let state = activity::controller_state_for_record(&sampled.context, &sampled.record).unwrap();
    let earlier = Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_secs(20))
        .unwrap()
        .to_string();
    let cached = json!({
        "schema_version":super::super::super::SHADOW_DOCUMENT_VERSION,
        "runtime_id":"launch-a", "runtime_generation":1, "activity_revision":state.revision,
        "interrupt_since":earlier,
        "observation":{"observer_version":OBSERVER_VERSION,"rule_id":"claude_agents_idle",
            "observed_at":super::super::super::now(),"projection":"waiting","disagrees":true}
    });
    fs::write(sampled.path(), serde_json::to_vec(&cached).unwrap()).unwrap();
    let view = sampled.collect(false);
    assert_eq!(view.phase, activity::TurnPhase::Unknown);
    assert_eq!(
        view.diagnostic.as_ref().unwrap().reason,
        "claude_turn_outcome_unknown"
    );
    assert_eq!(view.current_turn, state.current_turn);
    assert_eq!(view.last_turn, state.last_turn);
    let mut resampled = cached;
    resampled["observation"]["observed_at"] = json!(super::super::super::now());
    fs::write(sampled.path(), serde_json::to_vec(&resampled).unwrap()).unwrap();
    assert_eq!(
        activity::stream_projection(&sampled.collect(false)),
        activity::stream_projection(&view),
        "resampling unknown must not create another stream transition"
    );
}

#[test]
fn idle_before_stop_does_not_prevent_later_completion() {
    for ending in ["stop_observed", "turn_failed", "turn_completed"] {
        let sampled = running_turn(IDLE_ID);
        let durable =
            activity::controller_state_for_record(&sampled.context, &sampled.record).unwrap();
        let cached = |revision| ShadowDocument {
            schema_version: super::super::super::SHADOW_DOCUMENT_VERSION.into(),
            runtime_id: "launch-a".into(),
            runtime_generation: 1,
            activity_revision: Some(revision),
            interrupt_since: None,
            observation: observation(AgentState::Idle, &activity::TurnPhase::Working),
        };
        let first = cached(durable.revision);
        fs::write(sampled.path(), serde_json::to_vec(&first).unwrap()).unwrap();
        assert_eq!(sampled.collect(false).phase, activity::TurnPhase::Unknown);
        let event: activity::TurnEvent = serde_json::from_value(json!({
            "schema_version":"agent-session.turn-event.v1", "event_id":"late-end",
            "runtime_id":"launch-a", "provider":"claude", "provider_turn_id":"turn-1",
            "provider_session_id":IDLE_ID, "kind":ending, "completion_candidate":ending == "stop_observed",
            "confidence":"observed", "source_kind":"provider_hook"
        })).unwrap();
        let accepted = activity::ingest_event(&sampled.context, &sampled.record.id, event).unwrap();
        if ending == "stop_observed" {
            assert!(
                activity::state_for_view(&sampled.context, &sampled.record)
                    .unwrap()
                    .current_turn
                    .is_some(),
                "pre-Stop idle cannot prove post-Stop completion"
            );
            fs::write(
                sampled.path(),
                serde_json::to_vec(&cached(accepted.turn_state.revision)).unwrap(),
            )
            .unwrap();
        }
        let settled = activity::state_for_view(&sampled.context, &sampled.record).unwrap();
        assert_eq!(settled.phase, activity::TurnPhase::Waiting, "{ending}");
        assert!(settled.current_turn.is_none());
        if ending != "turn_failed" {
            assert!(
                activity::claude_notification_waiting(
                    &sampled.context,
                    &sampled.record,
                    Duration::ZERO
                ),
                "a late completion hook must restore mailbox eligibility"
            );
        }
        assert_eq!(
            settled.last_turn.unwrap().outcome,
            if ending == "turn_failed" {
                "failed"
            } else {
                "completed"
            }
        );
    }
}

#[test]
fn stream_retains_fixed_claude_outcome_diagnostic() {
    let diagnostic = activity::ActivityDiagnosticView {
        reason: "claude_turn_outcome_unknown".into(),
        extra: serde_json::Map::new(),
    };
    let projected =
        activity::stream_diagnostic::project(&diagnostic).expect("stream outcome diagnostic");
    assert_eq!(projected.reason, diagnostic.reason);
    let board = activity::diagnostic_for_board(&serde_json::Map::from_iter([(
        "reason".into(),
        json!(diagnostic.reason),
    )]))
    .expect("board outcome diagnostic");
    assert_eq!(
        board["reason"].as_str(),
        Some("claude_turn_outcome_unknown")
    );
}

#[test]
fn pending_approval_is_sampled_without_waiting_for_semantic_staleness() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (bin, _) = counting_claude(tmp.path());
    let _bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", bin.to_str().unwrap());
    let _config = EnvGuard::remove(&lock, "CLAUDE_CONFIG_DIR");
    let sampled = running_turn(IDLE_ID);
    pending_approval(&sampled);
    sampled.collect(true);
    let document = sampled.wait_for_document(|_| true);
    assert!(
        document.interrupt_since.is_none(),
        "Claude must not retain an interrupt timer"
    );
    let view = sampled.collect(false);
    assert_eq!(view.phase, activity::TurnPhase::Unknown);
    assert_eq!(
        view.current_turn.unwrap().attention.unwrap().kind,
        "approval"
    );
    assert!(
        !activity::claude_notification_waiting(&sampled.context, &sampled.record, Duration::ZERO),
        "unknown cannot create a mailbox idle wake"
    );
}
