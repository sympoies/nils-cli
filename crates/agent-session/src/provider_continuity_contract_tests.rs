use super::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

fn record(provider: &str, incarnation: &str, conversation: &str) -> SessionRecord {
    serde_json::from_value(json!({
        "schema_version":"agent-session.session.v1", "id":"managed-session",
        "agent":provider, "mode":"interactive", "title":null, "cwd":"/fixture",
        "tmux_session":"fixture", "prompt_file":null, "log_file":null,
        "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z",
        "runtime":{"kind":"tmux","tmux_session":"fixture","generation":1,
            "started_at":"2026-01-01T00:00:00Z","launch_id":incarnation},
        "provider_resume":{"provider":provider,"session_id":conversation,
            "captured_at":"2026-01-01T00:00:00Z","capture_method":"fixture","resume_args":[]}
    }))
    .unwrap()
}

#[test]
fn provider_continuity_retains_sixteen_chained_links_and_clamps_clock() {
    for provider in ["codex", "claude"] {
        let mut current = record(provider, "runtime-0", "conversation-0");
        for index in 1..=20 {
            let mut next = current.clone();
            next.runtime.as_mut().unwrap().launch_id = format!("runtime-{index}");
            next.provider_resume.as_mut().unwrap().session_id = format!("conversation-{index}");
            append(&current, &mut next, Kind::Clear);
            current = next;
        }
        let mut proof = retained(&current).unwrap();
        assert_eq!(proof.links.len(), 16);
        assert_eq!(proof.links[0].from_incarnation, "runtime-4");
        assert_eq!(proof.links[15].to_incarnation, "runtime-20");
        proof.links.last_mut().unwrap().at_epoch = 9_007_199_254_740_991;
        current
            .extra
            .insert(KEY.into(), serde_json::to_value(proof).unwrap());
        assert!(projection(&current, "running").is_some());
        let mut next = current.clone();
        next.runtime.as_mut().unwrap().launch_id = "runtime-21".into();
        append(&current, &mut next, Kind::Restart);
        let proof = retained(&next).unwrap();
        assert_eq!(proof.links.last().unwrap().at_epoch, 9_007_199_254_740_991);
        let wire = serde_json::to_value(proof).unwrap();
        let keys = |value: &Value| {
            let mut keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        assert_eq!(keys(&wire), ["links", "provider", "schema_version"]);
        assert_eq!(
            keys(&wire["links"][0]),
            [
                "at_epoch",
                "from_conversation",
                "from_incarnation",
                "kind",
                "to_conversation",
                "to_incarnation"
            ]
        );
    }
}

#[test]
fn provider_continuity_omits_invalid_proofs_without_breaking_the_snapshot() {
    let first = record("claude", "runtime-0", "conversation-0");
    let mut current = record("claude", "runtime-1", "conversation-1");
    append(&first, &mut current, Kind::Clear);
    let mut next = record("claude", "runtime-2", "conversation-1");
    append(&current, &mut next, Kind::Resume);
    let valid = next.extra[KEY].clone();
    let mut invalids = vec![Value::Null, json!({})];
    for (pointer, value) in [
        ("/schema_version", json!("invalid")),
        ("/provider", json!("codex")),
        ("/links", json!([])),
        ("/links/0/kind", json!("rebind")),
        ("/links/0/kind", json!("resume")),
        ("/links/1/kind", json!("clear")),
        ("/links/0/from_incarnation", json!("")),
        ("/links/0/from_conversation", json!("x".repeat(257))),
        ("/links/0/to_conversation", json!("control\n")),
        ("/links/1/from_incarnation", json!("broken")),
        ("/links/1/from_conversation", json!("broken")),
        ("/links/1/to_incarnation", json!("wrong-current")),
        ("/links/1/to_conversation", json!("wrong-current")),
        ("/links/1/at_epoch", json!(0)),
        ("/links/0/at_epoch", json!(-1)),
        ("/links/1/at_epoch", json!(9_007_199_254_740_992_i64)),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        invalids.push(invalid);
    }
    let mut extra_field = valid.clone();
    extra_field["account"] = json!("must-not-project");
    invalids.push(extra_field);
    for invalid in invalids {
        next.extra.insert(KEY.into(), invalid);
        assert!(projection(&next, "running").is_none());
    }
    next.extra.insert(KEY.into(), valid);
    assert!(projection(&next, "running").is_some());
    assert!(projection(&next, "stopped").is_none());
    assert!(projection(&next, "unknown").is_none());
}

#[test]
fn provider_continuity_cannot_bridge_an_unowned_identity_change() {
    let first = record("claude", "runtime-0", "conversation-0");
    let mut current = record("claude", "runtime-0", "conversation-1");
    append(&first, &mut current, Kind::Clear);
    current.provider_resume.as_mut().unwrap().session_id = "unowned-conversation".into();
    assert!(projection(&current, "running").is_none());
    let mut next = current.clone();
    next.runtime.as_mut().unwrap().launch_id = "runtime-1".into();
    append(&current, &mut next, Kind::Resume);
    let proof = retained(&next).unwrap();
    assert_eq!(proof.links.len(), 1);
    assert_eq!(proof.links[0].from_conversation, "unowned-conversation");
    let mut other = next.clone();
    other.id = "another-managed-session".into();
    append(&next, &mut other, Kind::Resume);
    assert!(projection(&other, "running").is_none());
}

#[test]
fn provider_continuity_snapshot_rejects_only_the_seventeenth_valid_link() {
    for provider in ["claude", "codex"] {
        let mut current = record(provider, "runtime-17", "conversation-17");
        // Construct the persisted proof directly: append() already truncates
        // at sixteen and would hide a missing read-side bound.
        let links: Vec<_> = (1..=17)
            .map(|index| {
                json!({
                    "kind": "clear",
                    "from_incarnation": format!("runtime-{}", index - 1),
                    "to_incarnation": format!("runtime-{index}"),
                    "from_conversation": format!("conversation-{}", index - 1),
                    "to_conversation": format!("conversation-{index}"),
                    "at_epoch": index,
                })
            })
            .collect();
        current.extra.insert(
            KEY.into(),
            json!({
                "schema_version": SCHEMA, "provider": provider, "links": links,
            }),
        );
        assert!(projection(&current, "running").is_none());
        // The same valid suffix, ending at the same running endpoint, is
        // publishable at the exact boundary. Only its length changed.
        current.extra.get_mut(KEY).unwrap()["links"]
            .as_array_mut()
            .unwrap()
            .remove(0);
        assert_eq!(projection(&current, "running").unwrap().links.len(), 16);
    }
}
