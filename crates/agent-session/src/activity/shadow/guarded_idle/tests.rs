use super::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

#[test]
fn freshness_has_an_exclusive_thirty_second_boundary() {
    let observed = "2000-01-01T00:00:01Z";
    for (at, expected) in [
        ("2000-01-01T00:00:30Z", true),
        ("2000-01-01T00:00:31Z", false),
        ("2000-01-01T00:00:32Z", false),
        ("2000-01-01T00:00:00Z", false),
    ] {
        assert_eq!(fresh(observed, at.parse().unwrap()), expected, "{at}");
    }
    assert!(!fresh(
        "unparsable",
        "2000-01-01T00:00:30Z".parse().unwrap()
    ));
}

fn document() -> Value {
    json!({
        "schema_version":"agent-session.activity.v1", "runtime_id":"runtime-one", "runtime_generation":1,
        "state":{"schema_version":"agent-session.turn-state.v1", "phase":"working", "revision":3,
            "phase_changed_at":"2000-01-01T00:00:00Z",
            "source":{"kind":"provider_hook", "confidence":"observed", "provider":"claude"},
            "current_turn":{"provider_turn_id":"turn-one", "started_at":"2000-01-01T00:00:00Z"},
            "diagnostic":{"reason":"completion_evidence_pending"}},
        "last_provider_event_kind":"stop_observed", "last_provider_event_provider":"claude",
        "last_provider_event_turn_id":"turn-one", "last_provider_event_at":"2000-01-01T00:00:00Z",
        "last_event_at":"2000-01-01T00:00:00Z"
    })
}

#[test]
fn stop_authorization_rejects_each_independent_conflict() {
    let baseline: ActivityDocument = serde_json::from_value(document()).unwrap();
    assert!(exact_stop(&baseline.state, &baseline));
    // A second document read can race the controller read. Keep the controller
    // fixed so these assertions isolate the raw document fences.
    for case in [
        "revision",
        "kind",
        "provider",
        "turn",
        "newer-event",
        "pending-attention",
        "overflow-attention",
        "turn-attention",
    ] {
        let mut value = document();
        let mut state = baseline.state.clone();
        match case {
            "revision" => value["state"]["revision"] = json!(4),
            "kind" => value["last_provider_event_kind"] = json!("progress"),
            "provider" => value["last_provider_event_provider"] = json!("codex"),
            "turn" => value["last_provider_event_turn_id"] = json!("other-turn"),
            "newer-event" => value["last_event_at"] = json!("2000-01-01T00:00:01Z"),
            "pending-attention" => value["pending_attention"] = json!([{"id":"question", "kind":"clarification", "requested_at":"2000-01-01T00:00:00Z"}]),
            "overflow-attention" => value["overflow_attention"] = json!({"kind":"clarification", "count":1, "requested_at":"2000-01-01T00:00:00Z"}),
            "turn-attention" => state.current_turn.as_mut().unwrap().attention = Some(serde_json::from_value(json!({"kind":"clarification", "requested_at":"2000-01-01T00:00:00Z", "pending_count":1})).unwrap()),
            _ => unreachable!(),
        }
        let changed: ActivityDocument = serde_json::from_value(value).unwrap();
        assert!(!exact_stop(&state, &changed), "{case}");
    }
}
