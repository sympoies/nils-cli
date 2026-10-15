use pretty_assertions::assert_eq;
use serde_json::json;

use super::super::{
    CliError, idempotency_replay, mailbox::StoredMessage, receipt_key, store_receipt,
};
use super::*;

const PRINCIPAL: &str = "remote:receive";
// Far enough ahead that replay, which reads the wall clock, sees live receipts.
const NOW: i64 = 4_000_000_000;

fn message(id: &str, state: &str) -> StoredMessage {
    serde_json::from_value(json!({
        "schema_version": "agent-session.message.v1",
        "message_id": id,
        "sender_session_id": "sender",
        "sender_incarnation": "sender-incarnation",
        "recipient_session_id": "recipient",
        "recipient_incarnation": "recipient-incarnation",
        "state": state,
        "revision": 1,
        "reply_depth": 0,
        "created_at": "1970-01-01T00:00:00Z",
        "created_at_epoch": 0,
        "expires_at": "1970-01-01T00:00:00Z",
        "expires_at_epoch": NOW + 86_400,
        "body": "",
        "body_bytes": 0
    }))
    .expect("message fixture")
}

fn key(principal: &str, index: usize) -> String {
    receipt_key(
        principal,
        "v1",
        "operation",
        &format!("seed-key-{index:05}"),
    )
}

/// Seed `count` receipts for `principal`, oldest first, each naming
/// `message-{index}` under idempotency key `seed-key-{index}`.
fn seed(registry: &mut Registry, principal: &str, count: usize) {
    for index in 0..count {
        registry.receipts.insert(
            key(principal, index),
            IdempotencyReceipt {
                principal: principal.into(),
                incarnation: "v1".into(),
                operation: "operation".into(),
                digest: "digest".into(),
                outcome: json!({"message_id": format!("message-{index:05}"), "state": "delivered"}),
                expires_at_epoch: NOW + 1 + index as i64,
            },
        );
    }
}

fn store(registry: &mut Registry, key: &str) -> Result<(), CliError> {
    store_receipt(
        registry,
        key.into(),
        PRINCIPAL.into(),
        "v1".into(),
        "operation".into(),
        "digest".into(),
        json!({"message_id": key, "state": "delivered"}),
        NOW,
    )
}

#[test]
fn principal_at_count_cap_with_acknowledged_receipts_still_admits() {
    let mut registry = Registry::default();
    seed(&mut registry, PRINCIPAL, MAX_RECEIPTS_PER_PRINCIPAL);
    for index in 0..MAX_RECEIPTS_PER_PRINCIPAL {
        registry
            .messages
            .push(message(&format!("message-{index:05}"), "acknowledged"));
    }

    store(&mut registry, "fresh-key-0001").expect("finished receipts must make room");

    assert_eq!(registry.receipts.len(), MAX_RECEIPTS_PER_PRINCIPAL);
    assert!(
        !registry.receipts.contains_key(&key(PRINCIPAL, 0)),
        "the oldest finished receipt is evicted first"
    );
    assert!(registry.receipts.contains_key(&key(PRINCIPAL, 1)));
    let record = &registry.receipt_evictions[PRINCIPAL];
    assert_eq!((record.evicted, record.last_evicted_at_epoch), (1, NOW));
    // Recent receipts keep idempotent replay.
    for (idempotency_key, message_id) in [
        ("seed-key-04095", "message-04095"),
        ("fresh-key-0001", "fresh-key-0001"),
    ] {
        let replay = idempotency_replay(
            &registry,
            idempotency_key,
            PRINCIPAL,
            "v1",
            "operation",
            "digest",
        )
        .unwrap()
        .expect("recent receipts replay");
        assert_eq!(replay["message_id"], message_id);
    }
}

#[test]
fn receipts_for_pruned_messages_are_finished() {
    let mut registry = Registry::default();
    seed(&mut registry, PRINCIPAL, MAX_RECEIPTS_PER_PRINCIPAL);

    store(&mut registry, "fresh-key-0001").expect("receipts of pruned messages make room");

    assert!(!registry.receipts.contains_key(&key(PRINCIPAL, 0)));
}

#[test]
fn unacknowledged_receipts_are_never_evicted() {
    let mut registry = Registry::default();
    seed(&mut registry, PRINCIPAL, MAX_RECEIPTS_PER_PRINCIPAL);
    for index in 0..MAX_RECEIPTS_PER_PRINCIPAL {
        let state = if index % 2 == 0 { "unread" } else { "read" };
        registry
            .messages
            .push(message(&format!("message-{index:05}"), state));
    }
    let before = serde_json::to_value(&registry.receipts).unwrap();

    let error = store(&mut registry, "fresh-key-0001").expect_err("in-flight receipts stay");

    assert_eq!(error.code(), "quota-exceeded");
    assert_eq!(serde_json::to_value(&registry.receipts).unwrap(), before);
    assert!(registry.receipt_evictions.is_empty());
}

#[test]
fn eviction_skips_in_flight_receipts_and_other_principals() {
    let mut registry = Registry::default();
    seed(&mut registry, PRINCIPAL, MAX_RECEIPTS_PER_PRINCIPAL);
    // The oldest receipts are in flight; only a newer one is finished.
    for index in 0..MAX_RECEIPTS_PER_PRINCIPAL {
        let state = if index == 10 {
            "acknowledged"
        } else {
            "unread"
        };
        registry
            .messages
            .push(message(&format!("message-{index:05}"), state));
    }
    registry.receipts.insert(
        "other".into(),
        IdempotencyReceipt {
            principal: "other".into(),
            incarnation: "v1".into(),
            operation: "operation".into(),
            digest: "digest".into(),
            outcome: json!({"message_id": "pruned"}),
            expires_at_epoch: NOW + 1,
        },
    );

    store(&mut registry, "fresh-key-0001").expect("one finished receipt makes room");

    assert!(registry.receipts.contains_key(&key(PRINCIPAL, 0)));
    assert!(!registry.receipts.contains_key(&key(PRINCIPAL, 10)));
    assert!(registry.receipts.contains_key("other"));
}

#[test]
fn receipts_without_a_message_are_never_evicted() {
    let mut registry = Registry::default();
    seed(&mut registry, PRINCIPAL, MAX_RECEIPTS_PER_PRINCIPAL);
    for receipt in registry.receipts.values_mut() {
        receipt.outcome = json!({"operation": "claim"});
    }

    store(&mut registry, "fresh-key-0001").expect_err("claim receipts are not finished");

    assert_eq!(registry.receipts.len(), MAX_RECEIPTS_PER_PRINCIPAL);
}

#[test]
fn usage_reports_principals_against_the_limit() {
    let mut registry = Registry::default();
    seed(&mut registry, PRINCIPAL, MAX_RECEIPTS_PER_PRINCIPAL);
    store(&mut registry, "fresh-key-0001").unwrap();
    seed(&mut registry, "worker", 2);
    let receipts: BTreeMap<String, ReceiptOwner> =
        serde_json::from_value(serde_json::to_value(&registry.receipts).unwrap()).unwrap();

    let usage = usage(&receipts, &registry.receipt_evictions);

    assert_eq!(
        usage,
        json!({
            "count": MAX_RECEIPTS_PER_PRINCIPAL + 2,
            "limit": MAX_RECEIPTS_GLOBAL,
            "principal_count": 2,
            "principals": [
                {"principal": PRINCIPAL, "count": MAX_RECEIPTS_PER_PRINCIPAL,
                 "limit": MAX_RECEIPTS_PER_PRINCIPAL, "evicted": 1,
                 "last_evicted_at": super::super::timestamp(NOW)},
                {"principal": "worker", "count": 2, "limit": MAX_RECEIPTS_PER_PRINCIPAL,
                 "evicted": 0, "last_evicted_at": null},
            ],
        })
    );
}

fn sized(registry: &mut Registry, index: usize, message_id: &str, filler: usize) {
    registry.receipts.insert(
        key(PRINCIPAL, index),
        IdempotencyReceipt {
            principal: PRINCIPAL.into(),
            incarnation: "v1".into(),
            operation: "operation".into(),
            digest: "digest".into(),
            outcome: json!({"message_id": message_id, "payload": "x".repeat(filler)}),
            expires_at_epoch: NOW + 1 + index as i64,
        },
    );
}

fn store_sized(registry: &mut Registry, filler: usize) -> Result<(), CliError> {
    store_receipt(
        registry,
        "fresh-key-0001".into(),
        PRINCIPAL.into(),
        "v1".into(),
        "operation".into(),
        "digest".into(),
        json!({"message_id": "fresh", "payload": "x".repeat(filler)}),
        NOW,
    )
}

#[test]
fn insufficient_finished_receipts_refuse_without_evicting() {
    // Two small finished receipts cannot free enough bytes next to three large
    // in-flight ones, so the refusal must leave every receipt in place.
    let mut registry = Registry::default();
    sized(&mut registry, 0, "done-0", 16);
    sized(&mut registry, 1, "done-1", 16);
    for index in 2..5 {
        let id = format!("live-{index}");
        sized(&mut registry, index, &id, 1_300 * 1024);
        registry.messages.push(message(&id, "unread"));
    }
    let before = serde_json::to_value(&registry.receipts).unwrap();

    let error = store_sized(&mut registry, 512 * 1024).expect_err("budget still exceeded");

    assert_eq!(error.code(), "quota-exceeded");
    assert_eq!(serde_json::to_value(&registry.receipts).unwrap(), before);
    assert!(registry.receipt_evictions.is_empty());
}

#[test]
fn an_oversized_candidate_refuses_without_evicting() {
    let mut registry = Registry::default();
    sized(&mut registry, 0, "done-0", 16);
    let before = serde_json::to_value(&registry.receipts).unwrap();

    store_sized(&mut registry, MAX_RECEIPT_BYTES_PER_PRINCIPAL as usize)
        .expect_err("a candidate over the whole budget never fits");

    assert_eq!(serde_json::to_value(&registry.receipts).unwrap(), before);
    assert!(registry.receipt_evictions.is_empty());
}

#[test]
fn a_full_registry_refuses_without_evicting_when_the_principal_cannot_free_enough() {
    let mut registry = Registry::default();
    seed(&mut registry, PRINCIPAL, MAX_RECEIPTS_PER_PRINCIPAL);
    // Only one finished receipt, but the registry is two over its global cap.
    for index in 1..MAX_RECEIPTS_PER_PRINCIPAL {
        registry
            .messages
            .push(message(&format!("message-{index:05}"), "unread"));
    }
    for index in 0..(MAX_RECEIPTS_GLOBAL - MAX_RECEIPTS_PER_PRINCIPAL + 1) {
        registry.receipts.insert(
            format!("other-{index}"),
            IdempotencyReceipt {
                principal: format!("other-{}", index % 64),
                incarnation: "v1".into(),
                operation: "operation".into(),
                digest: "digest".into(),
                outcome: json!({"operation": "claim"}),
                expires_at_epoch: NOW + 1,
            },
        );
    }
    let before = registry.receipts.len();

    let error = store(&mut registry, "fresh-key-0001").expect_err("global cap still exceeded");

    assert_eq!(error.code(), "quota-exceeded");
    assert_eq!(registry.receipts.len(), before);
    assert!(registry.receipts.contains_key(&key(PRINCIPAL, 0)));
    assert!(registry.receipt_evictions.is_empty());
}
