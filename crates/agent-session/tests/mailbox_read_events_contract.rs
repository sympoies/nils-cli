//! Frozen consumer fixture for `docs/specs/mailbox-read-events-v1.md`.
use agent_session::notice::{NoticeEnvelope, parse_notice};
use pretty_assertions::assert_eq;
use serde_json::Value;

fn corpus() -> Value {
    serde_json::from_str(include_str!("fixtures/mailbox-read-events-v1.json")).unwrap()
}

#[test]
fn event_elements_carry_the_canonical_envelope_and_safe_metadata() {
    let corpus = corpus();
    let history_id = corpus["history_id"].as_str().unwrap();
    for case in corpus["events"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let message = &case["message"];
        let mailbox = &message["mailbox"];
        let message_id = mailbox["message_id"].as_str().unwrap();
        let text = message["text"].as_str().unwrap();

        let parsed = parse_notice(text).unwrap_or_else(|| panic!("{name}: unclassified"));
        assert_eq!(parsed.metadata.kind(), "mailbox.read", "{name}");
        assert_eq!(parsed.metadata.source(), "agent-session", "{name}");
        assert_eq!(parsed.metadata.id(), Some(message_id), "{name}");
        assert!(message_id.len() <= 96 && text.len() < 64 * 1024, "{name}");
        let header = text.split('\n').next().unwrap();
        assert!(header.len() <= 512, "{name}");
        assert_eq!(
            NoticeEnvelope::new("mailbox.read", "agent-session", Some(message_id))
                .unwrap()
                .encode(parsed.body)
                .unwrap(),
            text,
            "{name}"
        );

        assert_eq!(
            message["id"],
            format!("{history_id}-{message_id}-mailbox"),
            "{name}"
        );
        assert_eq!(message["role"], "notice", "{name}");
        assert_eq!(mailbox["availability"], "complete", "{name}");
        assert!(
            ["read", "acknowledged"].contains(&mailbox["state"].as_str().unwrap()),
            "{name}"
        );
        // Incarnation, generation and capability values are never exposed.
        let sender = mailbox["sender"].as_object().unwrap();
        let keys: Vec<_> = sender.keys().map(String::as_str).collect();
        match sender["kind"].as_str().unwrap() {
            "session" => assert_eq!(keys, ["kind", "machine", "session_id"], "{name}"),
            "service" => assert_eq!(keys, ["kind", "machine", "service_id"], "{name}"),
            other => panic!("{name}: sender kind {other}"),
        }
    }
}

/// The spec's partition rule: a page over raw interval `[lo, hi)` holds the
/// rows starting inside it and the events anchored in `(lo, hi]`; `lo == 0`
/// also takes anchor 0, and an open end takes every later anchor.
fn page(placement: &Value, lo: Option<u64>, hi: Option<u64>) -> Vec<String> {
    let mut items: Vec<(u64, u8, String)> = Vec::new();
    for row in placement["rows"].as_array().unwrap() {
        let offset = row["offset"].as_u64().unwrap();
        if lo.is_none_or(|lo| offset >= lo) && hi.is_none_or(|hi| offset < hi) {
            items.push((offset, 1, row["id"].as_str().unwrap().into()));
        }
    }
    for event in placement["events"].as_array().unwrap() {
        let anchor = event["anchor"].as_u64().unwrap();
        let after_lo = lo.is_none_or(|lo| lo == 0 && anchor == 0 || anchor > lo);
        if after_lo && hi.is_none_or(|hi| anchor <= hi) {
            items.push((anchor, 0, event["id"].as_str().unwrap().into()));
        }
    }
    items.sort();
    items.into_iter().map(|(_, _, id)| id).collect()
}

fn ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap().into())
        .collect()
}

#[test]
fn forward_and_reverse_pages_partition_one_total_order() {
    let corpus = corpus();
    let placement = &corpus["placement"];
    let total = ids(&placement["total_order"]);

    let mut forward = Vec::new();
    for case in placement["forward_pages"].as_array().unwrap() {
        let actual = page(
            placement,
            case["cursor"].as_u64(),
            case["next_cursor"].as_u64(),
        );
        assert_eq!(actual, ids(&case["ids"]), "forward {}", case["cursor"]);
        forward.extend(actual);
    }
    assert_eq!(forward, total);

    let mut reverse = Vec::new();
    for case in placement["reverse_pages"].as_array().unwrap() {
        let actual = page(
            placement,
            case["next_older_cursor"].as_u64().or(Some(0)),
            case["older_cursor"].as_u64(),
        );
        assert_eq!(
            actual,
            ids(&case["ids"]),
            "reverse {}",
            case["older_cursor"]
        );
        reverse.splice(0..0, actual);
    }
    assert_eq!(reverse, total);
}
