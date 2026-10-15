//! Bounded retention for idempotency receipts at their per-principal caps.
//!
//! Receipts are otherwise released only by TTL, so a long-lived principal that
//! keeps admitting work (most visibly the shared remote-receive principal of a
//! busy host) can sit at its count or byte cap and refuse every new delivery.
//! At the cap, admission first evicts that principal's oldest *finished*
//! receipts: those whose outcome names a message that is already terminal
//! (`acknowledged`, `expired`, `deleted`) or no longer retained. Receipts for
//! unread or read-but-unacknowledged messages, and receipts that do not name a
//! message (claims, operations), are never evicted, so in-flight work keeps its
//! idempotent replay and a principal with no finished receipts is still
//! refused exactly as before.
use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    IdempotencyReceipt, MAX_RECEIPT_BYTES_PER_PRINCIPAL, MAX_RECEIPTS_GLOBAL,
    MAX_RECEIPTS_PER_PRINCIPAL, RECEIPT_TTL_SECS, Registry, receipt_bytes,
};

/// Usage rows reported by `message audit`, largest principals first.
const MAX_USAGE_ROWS: usize = 16;

/// Per-principal record of receipts evicted to admit new work.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct ReceiptEviction {
    pub evicted: u64,
    pub last_evicted_at_epoch: i64,
}

fn finished(receipt: &IdempotencyReceipt, messages: &HashMap<&str, &str>, now: i64) -> bool {
    if receipt.expires_at_epoch <= now {
        return true;
    }
    let Some(message_id) = receipt.outcome.get("message_id").and_then(Value::as_str) else {
        return false;
    };
    messages
        .get(message_id)
        .is_none_or(|state| matches!(*state, "acknowledged" | "expired" | "deleted"))
}

/// Evict the oldest finished receipts of `principal` until admitting a receipt
/// of `candidate_bytes` under `receipt_key` fits both per-principal caps.
/// Stops early when only unfinished receipts remain; the caller's quota checks
/// then refuse as before.
pub(super) fn make_room(
    registry: &mut Registry,
    principal: &str,
    receipt_key: &str,
    candidate_bytes: u64,
    now: i64,
) {
    let admits_new = !registry.receipts.contains_key(receipt_key);
    let mut count = 0usize;
    let mut bytes = 0u64;
    for (key, receipt) in &registry.receipts {
        if receipt.principal == principal && key != receipt_key {
            count += 1;
            bytes = bytes.saturating_add(receipt_bytes(receipt).unwrap_or(0));
        }
    }
    let over = |count: usize, bytes: u64| {
        (admits_new && count >= MAX_RECEIPTS_PER_PRINCIPAL)
            || bytes.saturating_add(candidate_bytes) > MAX_RECEIPT_BYTES_PER_PRINCIPAL
    };
    if !over(count, bytes) {
        return;
    }
    let messages: HashMap<&str, &str> = registry
        .messages
        .iter()
        .map(|message| (message.message_id.as_str(), message.state.as_str()))
        .collect();
    let mut evictable: Vec<(i64, String, u64)> = registry
        .receipts
        .iter()
        .filter(|(key, receipt)| {
            receipt.principal == principal
                && key.as_str() != receipt_key
                && finished(receipt, &messages, now)
        })
        .map(|(key, receipt)| {
            (
                receipt.expires_at_epoch,
                key.clone(),
                receipt_bytes(receipt).unwrap_or(0),
            )
        })
        .collect();
    drop(messages);
    evictable.sort();
    let mut evicted = 0u64;
    for (_, key, size) in evictable {
        if !over(count, bytes) {
            break;
        }
        registry.receipts.remove(&key);
        count -= 1;
        bytes = bytes.saturating_sub(size);
        evicted += 1;
    }
    if evicted > 0 {
        let cutoff = now.saturating_sub(RECEIPT_TTL_SECS);
        registry
            .receipt_evictions
            .retain(|_, record| record.last_evicted_at_epoch > cutoff);
        let record = registry
            .receipt_evictions
            .entry(principal.to_string())
            .or_default();
        record.evicted = record.evicted.saturating_add(evicted);
        record.last_evicted_at_epoch = now;
    }
}

/// Retained receipt principal; the audit never reads receipt contents and
/// tolerates malformed receipts rather than hiding the rest of the snapshot.
#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct ReceiptOwner {
    principal: String,
}

/// Per-principal receipt usage against the count quota, for `message audit`.
pub(super) fn usage(
    receipts: &BTreeMap<String, ReceiptOwner>,
    evictions: &BTreeMap<String, ReceiptEviction>,
) -> Value {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for receipt in receipts
        .values()
        .filter(|receipt| !receipt.principal.is_empty())
    {
        *counts.entry(receipt.principal.as_str()).or_default() += 1;
    }
    for principal in evictions.keys() {
        counts.entry(principal.as_str()).or_default();
    }
    let principal_count = counts.len();
    let mut rows: Vec<_> = counts.into_iter().collect();
    rows.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.cmp(b)));
    let principals: Vec<_> = rows
        .into_iter()
        .take(MAX_USAGE_ROWS)
        .map(|(principal, count)| {
            let eviction = evictions.get(principal);
            json!({
                "principal": principal,
                "count": count,
                "limit": MAX_RECEIPTS_PER_PRINCIPAL,
                "evicted": eviction.map_or(0, |record| record.evicted),
                "last_evicted_at": eviction
                    .map(|record| super::timestamp(record.last_evicted_at_epoch)),
            })
        })
        .collect();
    json!({
        "count": receipts.len(),
        "limit": MAX_RECEIPTS_GLOBAL,
        "principal_count": principal_count,
        "principals": principals,
    })
}

#[cfg(test)]
#[path = "receipt_retention_tests.rs"]
mod tests;
