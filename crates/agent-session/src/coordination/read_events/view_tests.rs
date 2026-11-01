//! The producer must emit exactly the wire elements frozen in the fixture corpus.
use crate::coordination::read_events::store::Event;
use pretty_assertions::assert_eq;
use serde_json::Value;

fn millis(value: &Value) -> Option<i64> {
    let text = value.as_str()?;
    Some(text.parse::<jiff::Timestamp>().unwrap().as_millisecond())
}

#[test]
fn producer_output_matches_every_fixture_element_byte_for_byte() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/mailbox-read-events-v1.json"
    ))
    .unwrap();
    let history_id = corpus["history_id"].as_str().unwrap();
    for case in corpus["events"].as_array().unwrap() {
        let wire = &case["message"];
        let mailbox = &wire["mailbox"];
        let text = wire["text"].as_str().unwrap();
        let body = text.split_once("\n\n").unwrap().1;
        let body = match mailbox["body_prefix"].as_str().unwrap() {
            "lf" => body.strip_prefix('\n').unwrap(),
            _ => body,
        };
        let event = Event {
            message_id: mailbox["message_id"].as_str().unwrap().into(),
            sender: mailbox["sender"].clone(),
            category: mailbox["category"].as_str().unwrap().into(),
            body: body.into(),
            anchor: 0,
            created_at_ms: millis(&wire["timestamp"]).unwrap(),
            read_at_ms: millis(&mailbox["read_at"]),
            acknowledged_at_ms: millis(&mailbox["acknowledged_at"]),
            revision: mailbox["revision"].as_u64().unwrap(),
        };
        let with_parts = wire.get("parts").is_some();
        assert_eq!(
            super::message(history_id, &event, with_parts).as_ref(),
            Some(wire),
            "{}",
            case["name"]
        );
    }
}
