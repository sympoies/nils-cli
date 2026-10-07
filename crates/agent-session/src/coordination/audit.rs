//! Owner-only, observational mailbox metadata. Never deserialize message bodies,
//! hashes, capabilities, idempotency receipts, or forwarding content.
use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::{notification, remote};
use crate::{CliContext, CliError, cli::MessageCategory};

pub(crate) const ROUTE: &str = "/coordination/messages/audit/v1";

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Query {
    pub older_than: u64,
    pub limit: usize,
    pub cursor: Option<String>,
    pub include_healthy: bool,
}
impl Default for Query {
    fn default() -> Self {
        Self {
            older_than: 300,
            limit: 50,
            cursor: None,
            include_healthy: false,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Registry {
    schema_version: String,
    messages: Vec<Inbox>,
    brokers: BTreeMap<String, Broker>,
    notifications: BTreeMap<String, notification::NotificationReceipt>,
}
#[derive(Deserialize)]
struct Broker {
    incarnation: String,
    state: String,
    heartbeat_epoch: i64,
}
#[derive(Deserialize)]
struct Inbox {
    message_id: String,
    sender_session_id: String,
    sender_incarnation: String,
    recipient_session_id: String,
    recipient_incarnation: String,
    state: String,
    revision: u64,
    created_at_epoch: i64,
    #[serde(default)]
    remote_created_at_epoch: Option<i64>,
    expires_at_epoch: i64,
    #[serde(default)]
    category: Option<MessageCategory>,
}
#[derive(Default, Deserialize)]
#[serde(default)]
struct Journal {
    schema_version: String,
    remote_outbox: Vec<Outbox>,
    retained: Vec<Retained>,
}
#[derive(Deserialize)]
struct Envelope {
    message_id: String,
    from: remote::Origin,
    to: remote::Address,
    created_at_epoch: i64,
    expires_at_epoch: i64,
    #[serde(default)]
    category: Option<MessageCategory>,
}
#[derive(Deserialize)]
struct Outbox {
    envelope: Envelope,
    state: String,
    attempts: u64,
    next_attempt_epoch: i64,
    #[serde(default)]
    last_attempt_at_epoch: Option<i64>,
    #[serde(default)]
    state_changed_at_epoch: Option<i64>,
    reason: Option<String>,
    receipt: Option<DeliveryReceipt>,
}
#[derive(Deserialize)]
struct DeliveryReceipt {
    persisted_at_epoch: Option<i64>,
}
#[derive(Deserialize)]
struct Retained {
    message_id: String,
    sender: remote::Origin,
    recipient: remote::Address,
    expires_at_epoch: i64,
    state: String,
    attempts: u64,
    reason: Option<String>,
    persisted_at_epoch: Option<i64>,
    #[serde(default)]
    created_at_epoch: Option<i64>,
    #[serde(default)]
    last_attempt_at_epoch: Option<i64>,
    #[serde(default)]
    state_changed_at_epoch: Option<i64>,
    #[serde(default)]
    category: Option<MessageCategory>,
}

#[derive(Clone, Serialize)]
struct RecipientStatus {
    runtime: &'static str,
    broker: &'static str,
    current_incarnation: Option<String>,
    heartbeat_at: Option<String>,
    heartbeat_fresh: Option<bool>,
}
impl Default for RecipientStatus {
    fn default() -> Self {
        Self {
            runtime: "unknown",
            broker: "unknown",
            current_incarnation: None,
            heartbeat_at: None,
            heartbeat_fresh: None,
        }
    }
}
#[derive(Serialize)]
struct Record {
    source: &'static str,
    message_id: String,
    sender: Value,
    recipient: remote::Address,
    category: MessageCategory,
    sent_at: Option<String>,
    persisted_at: Option<String>,
    expires_at: Option<String>,
    unread_age_seconds: Option<i64>,
    end_to_end_latency_seconds: Option<i64>,
    mailbox_state: Option<&'static str>,
    mailbox_revision: Option<u64>,
    delivery_state: Option<&'static str>,
    attempts: Option<u64>,
    next_retry_at: Option<String>,
    last_attempt_at: Option<String>,
    state_changed_at: Option<String>,
    reason_code: Option<&'static str>,
    recipient_status: RecipientStatus,
    notification: Option<Value>,
    anomalies: Vec<&'static str>,
}
fn metadata_key(source: &str, id: &str, sender: &Value, recipient: &remote::Address) -> String {
    serde_json::to_string(&(source, id, sender, recipient)).expect("metadata key")
}

fn time(epoch: Option<i64>) -> Option<String> {
    epoch
        .filter(|t| *t > 0)
        .and_then(|t| jiff::Timestamp::from_second(t).ok())
        .map(|t| t.to_string())
}
fn age(now: i64, then: Option<i64>) -> Option<i64> {
    then.filter(|t| *t > 0)
        .map(|t| now.saturating_sub(t).max(0))
}
fn known<'a>(value: &str, allowed: &'a [&'static str]) -> &'a str {
    allowed
        .iter()
        .copied()
        .find(|v| *v == value)
        .unwrap_or("unknown")
}
fn reason(value: Option<&str>) -> Option<&'static str> {
    value.map(|v| {
        remote::relay_reason_code(v).unwrap_or_else(|| {
            known(
                v,
                &[
                    "remote-messaging-unavailable",
                    "coordination-unavailable",
                    "remote-receipt-invalid",
                    "delivery-unknown",
                ],
            )
        })
    })
}

fn read<T: DeserializeOwned>(path: &Path, cap: u64) -> Result<Option<T>, &'static str> {
    match super::read_private_file(path, cap) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "invalid"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("unavailable"),
    }
}
fn sender(message: &Inbox, machine: &str) -> Value {
    if remote::is_remote_sender(&message.sender_session_id) {
        return remote::sender_address_metadata(
            &message.sender_session_id,
            &message.sender_incarnation,
        )
        .map(|address| json!(address))
        .unwrap_or(Value::Null);
    }
    if super::service::is_service_sender(&message.sender_session_id) {
        return super::service::sender_origin_metadata(&message.sender_session_id,&message.sender_incarnation)
            .map(|origin| json!({"kind":"service","machine":origin.machine,"service_id":origin.service_id,"service_generation":origin.service_generation})).unwrap_or(Value::Null);
    }
    json!({"machine":machine,"session_id":message.sender_session_id,"session_incarnation":message.sender_incarnation})
}

#[cfg(test)]
thread_local! { static STATUS_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

fn recipient_status(context: &CliContext, registry: &Registry, id: &str) -> RecipientStatus {
    #[cfg(test)]
    STATUS_PROBES.with(|probes| probes.set(probes.get() + 1));
    // A failed read is uncertainty, never deletion. No runtime reconciliation,
    // broker renewal, capability read, or tmux input occurs on this path.
    if crate::validate_id(id).is_err() {
        return RecipientStatus::default();
    }
    // Mail addresses are exact IDs, never the interactive prefix resolver.
    let path = context
        .state_dir
        .join("sessions")
        .join(id)
        .join("session.json");
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return RecipientStatus {
                runtime: "missing",
                ..RecipientStatus::default()
            };
        }
        Err(_) => return RecipientStatus::default(),
        Ok(_) => {}
    }
    let record = match crate::load_session_record(context, id) {
        Ok(v) if v.id == id => v,
        Ok(_) => return RecipientStatus::default(),
        Err(e) if e.code() == "session-not-found" => {
            return RecipientStatus {
                runtime: "missing",
                ..RecipientStatus::default()
            };
        }
        Err(_) => return RecipientStatus::default(),
    };
    let current = super::incarnation(&record).ok();
    let runtime = match crate::coordination_runtime_evidence(context, &record)
        .ok()
        .map(|v| v.status)
    {
        Some(crate::CoordinationRuntimeStatus::Running) => "running",
        Some(crate::CoordinationRuntimeStatus::Stopped) => "stopped",
        _ => "unknown",
    };
    let broker = registry
        .brokers
        .get(id)
        .filter(|b| current.as_deref() == Some(&b.incarnation));
    RecipientStatus {
        runtime,
        current_incarnation: current,
        broker: broker
            .map(|b| known(&b.state, &["ready", "stopped", "lost", "starting"]))
            .unwrap_or("unknown"),
        heartbeat_at: broker.and_then(|b| time(Some(b.heartbeat_epoch))),
        heartbeat_fresh: broker.map(|b| {
            super::broker::heartbeat_fresh(context, id, &b.incarnation, b.heartbeat_epoch)
        }),
    }
}
fn target_anomalies(row: &mut Record) {
    if row.recipient_status.runtime == "missing" {
        row.anomalies.push("recipient-missing");
    }
    if row
        .recipient_status
        .current_incarnation
        .as_deref()
        .is_some_and(|i| i != row.recipient.session_incarnation)
    {
        row.anomalies.push("incarnation-mismatch");
    } else if row.recipient_status.runtime == "stopped" || row.recipient_status.broker == "stopped"
    {
        row.anomalies.push("recipient-stopped");
    }
}
fn invalid_query() -> CliError {
    CliError::usage(
        "mail-audit-query-invalid",
        "mail audit query or cursor is invalid",
        None,
    )
}

/// Independent atomic file reads, with an observation interval and explicit
/// partial source status. No locks/cursors are written and no maintenance runs.
pub(crate) fn snapshot(
    context: &CliContext,
    machine: &str,
    query: Query,
) -> Result<Value, CliError> {
    snapshot_at(context, machine, query, super::now_epoch())
}

fn snapshot_at(
    context: &CliContext,
    machine: &str,
    query: Query,
    now: i64,
) -> Result<Value, CliError> {
    if !(1..=100).contains(&query.limit) || query.older_than > i64::MAX as u64 {
        return Err(invalid_query());
    }
    let after = query
        .cursor
        .as_deref()
        .map(|v| {
            if v.len() > 8192 {
                return Err(invalid_query());
            }
            let c: Cursor = serde_json::from_str(v).map_err(|_| invalid_query())?;
            if c.version != 1
                || c.machine != machine
                || c.older_than != query.older_than
                || c.include_healthy != query.include_healthy
            {
                return Err(invalid_query());
            }
            Ok(c.after)
        })
        .transpose()?;
    let registry = read::<Registry>(
        &context.state_dir.join("coordination/registry.json"),
        nils_common::coordination_projection::MAX_REGISTRY_BYTES,
    )
    .and_then(|r| {
        let Some(r) = r else {
            return Ok(Registry::default());
        };
        if matches!(
            r.schema_version.as_str(),
            "agent-session.coordination-registry.v1" | "agent-session.coordination-registry.v2"
        ) {
            Ok(r)
        } else {
            Err("unsupported")
        }
    });
    let journal = read::<Journal>(
        &context
            .state_dir
            .join("coordination/federation-journal.json"),
        32 * 1024 * 1024,
    )
    .and_then(|j| {
        let Some(j) = j else {
            return Ok(Journal::default());
        };
        if matches!(
            j.schema_version.as_str(),
            "agent-session.federation-journal.v1"
                | "agent-session.federation-journal.v2"
                | "agent-session.federation-journal.v3"
                | "agent-session.federation-journal.v4"
        ) && (j.schema_version != "agent-session.federation-journal.v1" || j.retained.is_empty())
        {
            Ok(j)
        } else {
            Err("unsupported")
        }
    });
    let partial = registry.is_err() || journal.is_err();
    let sources = json!({"inbox":registry.as_ref().err().copied().unwrap_or("available"), "notifications":registry.as_ref().err().copied().unwrap_or("available"), "outbox":journal.as_ref().err().copied().unwrap_or("available")});
    let mut rows = Vec::new();
    let mut statuses = BTreeMap::new();
    if let Ok(registry) = &registry {
        let mut notifications = BTreeMap::new();
        for n in registry.notifications.values() {
            let key = (&n.target_session_id, &n.target_incarnation);
            let slot = notifications.entry(key).or_insert(n);
            if (n.generation, n.updated_at_epoch) > (slot.generation, slot.updated_at_epoch) {
                *slot = n;
            }
        }
        let mut candidates: Vec<_> = registry
            .messages
            .iter()
            .map(|m| {
                let from = sender(m, machine);
                let to = remote::Address {
                    machine: machine.into(),
                    session_id: m.recipient_session_id.clone(),
                    session_incarnation: m.recipient_incarnation.clone(),
                };
                (
                    metadata_key("inbox", &m.message_id, &from, &to),
                    m,
                    from,
                    to,
                )
            })
            .filter(|(key, _, _, _)| after.as_ref().is_none_or(|after| key > after))
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, m, from, to) in candidates {
            if rows.len() > query.limit {
                break;
            }
            let status = statuses
                .entry(m.recipient_session_id.clone())
                .or_insert_with(|| recipient_status(context, registry, &m.recipient_session_id))
                .clone();
            let receipt = notifications
                .get(&(&m.recipient_session_id, &m.recipient_incarnation))
                .copied();
            let notification = receipt.map(|n| json!({
                "state":known(&n.state, &["queued","attempting","prompt_submitted","attempt_unknown","undeliverable"]),
                "generation":n.generation,"notified_generation":n.notified_generation,
                "queued_at":time(Some(n.queued_at_epoch)),"attempted_at":time(Some(n.attempted_at_epoch)),
                "updated_at":time(Some(n.updated_at_epoch)),"next_retry_at":time(Some(n.next_attempt_at_epoch)),
                "reason_code":n.last_reason.as_deref().map(notification::safe_reason),
            }));
            let state = known(&m.state, &["unread", "read", "acknowledged", "expired"]);
            let unread = state == "unread" && m.expires_at_epoch > now;
            let mut row = Record {
                source: "inbox",
                message_id: m.message_id.clone(),
                sender: from,
                recipient: to,
                category: m.category.unwrap_or_default(),
                sent_at: time(m.remote_created_at_epoch.or(Some(m.created_at_epoch))),
                persisted_at: time(Some(m.created_at_epoch)),
                expires_at: time(Some(m.expires_at_epoch)),
                unread_age_seconds: unread.then(|| age(now, Some(m.created_at_epoch))).flatten(),
                end_to_end_latency_seconds: m
                    .remote_created_at_epoch
                    .map(|sent| m.created_at_epoch.saturating_sub(sent)),
                mailbox_state: Some(
                    if m.expires_at_epoch <= now && matches!(state, "unread" | "read") {
                        "expired"
                    } else {
                        state
                    },
                ),
                mailbox_revision: Some(m.revision),
                delivery_state: None,
                attempts: None,
                next_retry_at: None,
                last_attempt_at: None,
                state_changed_at: None,
                reason_code: None,
                recipient_status: status,
                notification,
                anomalies: vec![],
            };
            if row
                .unread_age_seconds
                .is_some_and(|age| age > query.older_than as i64)
            {
                row.anomalies.push("overdue-unread");
            }
            if m.expires_at_epoch > now && matches!(state, "unread" | "read") {
                target_anomalies(&mut row);
                if receipt.is_some_and(|n| {
                    n.state == "attempt_unknown"
                        || (n.state == "undeliverable"
                            && n.last_reason.as_deref()
                                != Some(notification::REASON_HOOK_DELIVERED))
                }) {
                    row.anomalies.push("notification-failed");
                }
            }
            if query.include_healthy || !row.anomalies.is_empty() {
                rows.push(row);
            }
        }
    }
    if let Ok(journal) = journal
        && rows.len() <= query.limit
    {
        let pending = journal.remote_outbox.into_iter().map(|o| {
            let persisted = o.receipt.and_then(|r| r.persisted_at_epoch);
            (
                Retained {
                    message_id: o.envelope.message_id,
                    sender: o.envelope.from,
                    recipient: o.envelope.to,
                    expires_at_epoch: o.envelope.expires_at_epoch,
                    state: o.state,
                    attempts: o.attempts,
                    reason: o.reason,
                    persisted_at_epoch: persisted,
                    created_at_epoch: Some(o.envelope.created_at_epoch),
                    last_attempt_at_epoch: o.last_attempt_at_epoch,
                    state_changed_at_epoch: o.state_changed_at_epoch,
                    category: o.envelope.category,
                },
                Some(o.next_attempt_epoch),
            )
        });
        let mut candidates: Vec<_> = pending
            .chain(journal.retained.into_iter().map(|r| (r, None)))
            .map(|(o, next)| {
                (
                    metadata_key(
                        "outbox",
                        &o.message_id,
                        &o.sender.projection(),
                        &o.recipient,
                    ),
                    o,
                    next,
                )
            })
            .filter(|(key, _, _)| after.as_ref().is_none_or(|after| key > after))
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, o, next) in candidates {
            if rows.len() > query.limit {
                break;
            }
            let state = known(
                &o.state,
                &["queued", "delivered", "rejected", "delivery-unknown"],
            );
            let mut row = Record {
                source: "outbox",
                message_id: o.message_id,
                sender: o.sender.projection(),
                recipient: o.recipient,
                category: o.category.unwrap_or_default(),
                sent_at: time(o.created_at_epoch),
                persisted_at: time(o.persisted_at_epoch),
                expires_at: time(Some(o.expires_at_epoch)),
                unread_age_seconds: None,
                end_to_end_latency_seconds: o
                    .persisted_at_epoch
                    .zip(o.created_at_epoch)
                    .map(|(p, s)| p.saturating_sub(s)),
                mailbox_state: None,
                mailbox_revision: None,
                delivery_state: Some(state),
                attempts: Some(o.attempts),
                next_retry_at: if state == "queued" { time(next) } else { None },
                last_attempt_at: time(o.last_attempt_at_epoch),
                state_changed_at: time(o.state_changed_at_epoch),
                reason_code: reason(o.reason.as_deref()),
                recipient_status: RecipientStatus::default(),
                notification: None,
                anomalies: vec![],
            };
            match state {
                "queued"
                    if age(now, o.created_at_epoch)
                        .is_some_and(|a| a > query.older_than as i64) =>
                {
                    row.anomalies.push("queued-overdue")
                }
                "rejected" => row.anomalies.push("rejected"),
                "delivery-unknown" | "unknown" => row.anomalies.push("delivery-unknown"),
                _ => {}
            }
            if query.include_healthy || !row.anomalies.is_empty() {
                rows.push(row);
            }
        }
    }
    let more = rows.len() > query.limit;
    rows.truncate(query.limit);
    let next_cursor = if more {
        rows.last().map(|r| {
            serde_json::to_string(&Cursor {
                version: 1,
                machine: machine.into(),
                older_than: query.older_than,
                include_healthy: query.include_healthy,
                after: metadata_key(r.source, &r.message_id, &r.sender, &r.recipient),
            })
            .expect("cursor")
        })
    } else {
        None
    };
    Ok(
        json!({"schema_version":"agent-session.mail-audit.v1","machine":machine,"snapshot_at":super::timestamp(now),"observation_finished_at":super::timestamp(super::now_epoch()),"older_than_seconds":query.older_than,"partial":partial,"sources":sources,"records":rows,"next_cursor":next_cursor}),
    )
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    machine: String,
    older_than: u64,
    include_healthy: bool,
    after: String,
}

pub(crate) fn cli(
    context: &CliContext,
    args: crate::cli::MessageAuditArgs,
) -> Result<Value, CliError> {
    let machine = crate::board::machine_identity(None, context);
    snapshot(
        context,
        &machine,
        Query {
            older_than: args.older_than,
            limit: args.limit,
            cursor: args.cursor,
            include_healthy: args.include_healthy,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
    use pretty_assertions::assert_eq;

    fn fixture() -> (tempfile::TempDir, CliContext) {
        let temp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: temp.path().into(),
            host: None,
        };
        std::fs::create_dir(context.state_dir.join("coordination")).unwrap();
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/coordination/mail-audit-v1.json"
        ))
        .unwrap();
        for (file, value) in [
            ("registry.json", &fixture["registry"]),
            ("federation-journal.json", &fixture["journal"]),
        ] {
            write_atomic(
                &context.state_dir.join("coordination").join(file),
                &serde_json::to_vec(value).unwrap(),
                SECRET_FILE_MODE,
            )
            .unwrap();
        }
        (temp, context)
    }

    #[test]
    fn unread_age_uses_ingress_time_and_strict_threshold_not_source_send() {
        let (_temp, context) = fixture();
        let query = Query {
            include_healthy: true,
            ..Query::default()
        };
        let at_boundary = snapshot_at(&context, "destination", query.clone(), 500).unwrap();
        let rows = at_boundary["records"].as_array().unwrap();
        let overdue = rows.iter().find(|r| r["message_id"] == "overdue").unwrap();
        assert_eq!(overdue["unread_age_seconds"], 300);
        assert_eq!(overdue["end_to_end_latency_seconds"], 100);
        assert!(
            !overdue["anomalies"]
                .as_array()
                .unwrap()
                .contains(&json!("overdue-unread"))
        );
        let next = snapshot_at(&context, "destination", query, 501).unwrap();
        let row = next["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["message_id"] == "overdue")
            .unwrap();
        assert!(
            row["anomalies"]
                .as_array()
                .unwrap()
                .contains(&json!("overdue-unread"))
        );
        let ack = next["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["message_id"] == "acknowledged")
            .unwrap();
        assert_eq!(ack["anomalies"], json!([]));
    }

    #[test]
    fn unavailable_or_unsupported_sources_are_partial_and_leave_other_metadata_visible() {
        let (_temp, context) = fixture();
        let journal = context
            .state_dir
            .join("coordination/federation-journal.json");
        write_atomic(
            &journal,
            br#"{"schema_version":"agent-session.federation-journal.v99"}"#,
            SECRET_FILE_MODE,
        )
        .unwrap();
        let page = snapshot_at(&context, "destination", Query::default(), 1000).unwrap();
        assert_eq!(page["partial"], true);
        assert_eq!(page["sources"]["outbox"], "unsupported");
        assert!(!page["records"].as_array().unwrap().is_empty());
        write_atomic(&journal, b"invalid", SECRET_FILE_MODE).unwrap();
        assert_eq!(
            snapshot_at(&context, "destination", Query::default(), 1000).unwrap()["sources"]["outbox"],
            "invalid"
        );
    }

    #[test]
    fn notification_receipts_are_incarnation_scoped_and_submission_is_not_read() {
        let (_temp, context) = fixture();
        let path = context.state_dir.join("coordination/registry.json");
        let mut registry: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        registry["notifications"]["receipt"]["state"] = json!("attempt_unknown");
        write_atomic(
            &path,
            &serde_json::to_vec(&registry).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        let page = snapshot_at(&context, "destination", Query::default(), 1000).unwrap();
        let rows = page["records"].as_array().unwrap();
        let row = rows.iter().find(|r| r["message_id"] == "overdue").unwrap();
        assert!(
            row["anomalies"]
                .as_array()
                .unwrap()
                .contains(&json!("notification-failed"))
        );
        assert_eq!(row["mailbox_state"], "unread");
        let old = rows
            .iter()
            .find(|r| r["message_id"] == "old-incarnation")
            .unwrap();
        assert_eq!(old["notification"], Value::Null);
    }

    #[test]
    fn absent_exact_target_cannot_resolve_to_another_sessions_prefix() {
        let (_temp, context) = fixture();
        let path = context
            .state_dir
            .join("sessions/missing-extra/session.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_atomic(&path, &serde_json::to_vec(&json!({
            "schema_version":"agent-session.session.v1", "id":"missing-extra", "agent":"codex", "mode":"interactive",
            "title":null,"cwd":".","tmux_session":"fixture","prompt_file":null,"log_file":null,
            "created_at":"2030-01-01T00:00:00Z","updated_at":"2030-01-01T00:00:00Z"
        })).unwrap(),SECRET_FILE_MODE).unwrap();
        let page = snapshot_at(&context, "destination", Query::default(), 1000).unwrap();
        let row = page["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["message_id"] == "missing")
            .unwrap();
        assert_eq!(row["recipient_status"]["runtime"], "missing");
        assert!(
            row["anomalies"]
                .as_array()
                .unwrap()
                .contains(&json!("recipient-missing"))
        );
    }

    #[test]
    fn paginated_healthy_collection_probes_only_selected_recipient_candidates() {
        let (_temp, context) = fixture();
        let path = context.state_dir.join("coordination/registry.json");
        let mut registry: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let template = registry["messages"][0].clone();
        registry["messages"] = json!(
            (0..250)
                .map(|i| {
                    let mut m = template.clone();
                    m["message_id"] = json!(format!("message-{i:04}"));
                    m["recipient_session_id"] = json!(format!("recipient-{i:04}"));
                    m
                })
                .collect::<Vec<_>>()
        );
        write_atomic(
            &path,
            &serde_json::to_vec(&registry).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        STATUS_PROBES.with(|n| n.set(0));
        let mut query = Query {
            limit: 100,
            include_healthy: true,
            ..Query::default()
        };
        let mut count = 0;
        loop {
            let page = snapshot_at(&context, "destination", query.clone(), 1000).unwrap();
            count += page["records"].as_array().unwrap().len();
            query.cursor = page["next_cursor"].as_str().map(str::to_owned);
            if query.cursor.is_none() {
                break;
            }
        }
        assert_eq!(count, 252);
        STATUS_PROBES.with(|n| assert_eq!(n.get(), 252)); // 250 recipients + one lookahead on each inbox page.
    }

    #[test]
    fn audit_preserves_supported_relay_reasons_and_rejects_malformed_service_origins() {
        let (_temp, context) = fixture();
        let path = context
            .state_dir
            .join("coordination/federation-journal.json");
        let mut journal: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        journal["retained"][0]["reason"] = json!("remote-messaging-unsupported");
        write_atomic(
            &path,
            &serde_json::to_vec(&journal).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        let page = snapshot_at(&context, "destination", Query::default(), 1000).unwrap();
        let row = page["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["message_id"] == "failed-delivery")
            .unwrap();
        assert_eq!(row["reason_code"], "remote-messaging-unsupported");
        let mut m:Inbox=serde_json::from_value(json!({"message_id":"service-fixture","sender_session_id":"service:[\"source\",\"reporter\"]","sender_incarnation":"generation","recipient_session_id":"recipient","recipient_incarnation":"incarnation","state":"unread","revision":1,"created_at_epoch":100,"expires_at_epoch":200})).unwrap();
        assert_eq!(sender(&m, "destination")["kind"], "service");
        m.sender_incarnation = "invalid generation".into();
        assert_eq!(sender(&m, "destination"), Value::Null);
    }

    #[test]
    fn cursor_and_limits_are_bounded_and_filters_cannot_change_mid_scan() {
        let (_temp, context) = fixture();
        let page = snapshot_at(
            &context,
            "destination",
            Query {
                limit: 1,
                ..Query::default()
            },
            1000,
        )
        .unwrap();
        let cursor = page["next_cursor"].as_str().unwrap().to_owned();
        for query in [
            Query {
                limit: 0,
                ..Query::default()
            },
            Query {
                limit: 101,
                ..Query::default()
            },
            Query {
                cursor: Some("bad".into()),
                ..Query::default()
            },
            Query {
                cursor: Some(cursor),
                older_than: 301,
                ..Query::default()
            },
        ] {
            assert_eq!(
                snapshot_at(&context, "destination", query, 1000)
                    .unwrap_err()
                    .code(),
                "mail-audit-query-invalid"
            );
        }
    }
}
