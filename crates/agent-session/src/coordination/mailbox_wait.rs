use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::mailbox;
use super::{
    CliError, authenticate_from_file, clean_expired, coordination_root, lock_registry, now_epoch,
    revalidate_capability_file,
};
use crate::CliContext;
use crate::cli::MessageWaitArgs;

const CANCELLATION_CHECK_INTERVAL: Duration = Duration::from_millis(100);
const CURSOR_SCHEMA: &str = "agent-session.message-wait-any-unread.cursor.v1";
const MAX_CURSOR_BYTES: usize = 16 * 1024;
const MAX_CURSOR_IDS: usize = mailbox::MAX_SESSION_MESSAGES;
const MAX_CURSOR_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WaitCursor {
    schema_version: String,
    recipient_session_id: String,
    recipient_incarnation: String,
    seen_message_ids: BTreeSet<String>,
}

pub(super) fn wait_with_cancellation(
    context: &CliContext,
    args: &MessageWaitArgs,
    cancelled: Option<&AtomicBool>,
) -> Result<Value, CliError> {
    let capability_file = super::mailbox::resolve_capability_file(args.capability_file.as_deref())?;
    let (record, recipient_incarnation) =
        authenticate_from_file(context, &args.session, Some(&capability_file))?;
    let timeout = mailbox::parse_wait(&args.timeout)?;
    let cursor = args.newer_than.as_deref().map(parse_cursor).transpose()?;
    if cursor.as_ref().is_some_and(|cursor| {
        cursor.schema_version != CURSOR_SCHEMA
            || cursor.recipient_session_id != record.id
            || cursor.recipient_incarnation != recipient_incarnation
    }) {
        return Err(cursor_invalid());
    }

    // Watch the containing directory because registry writes replace the file
    // atomically. Install the watcher before the first registry read so an
    // arrival between setup and that read is either observed or returned.
    let root = coordination_root(context)?;
    let (events_tx, events_rx) = mpsc::channel();
    let registry_path = root.join(super::REGISTRY_FILE);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        // Reads and lock activity must not wake the waiter: rechecking the
        // registry produces those notifications itself. Always forward errors.
        if event
            .as_ref()
            .is_ok_and(|event| !changes_registry(event, &registry_path))
        {
            return;
        }
        let _ = events_tx.send(event);
    })
    .map_err(|_| notification_error())?;
    watcher
        .watch(&root, RecursiveMode::NonRecursive)
        .map_err(|_| notification_error())?;

    let started = Instant::now();
    loop {
        if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            return Err(CliError::runtime(
                "wait-cancelled",
                "message wait was cancelled",
                None,
            ));
        }

        let mut locked = lock_registry(context)?;
        let registry_changed = clean_expired(&mut locked.registry, now_epoch());
        revalidate_capability_file(
            context,
            &locked.registry,
            &record,
            &recipient_incarnation,
            &capability_file,
        )?;

        let current_unread_ids: BTreeSet<_> = locked
            .registry
            .messages
            .iter()
            .filter(|message| {
                message.recipient_session_id == record.id
                    && message.recipient_incarnation == recipient_incarnation
                    && message.state == "unread"
            })
            .map(|message| message.message_id.clone())
            .collect();
        let mut seen_message_ids: BTreeSet<_> = cursor
            .as_ref()
            .map(|cursor| cursor.seen_message_ids.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|message_id| current_unread_ids.contains(message_id))
            .collect();
        let unread: Vec<_> = locked
            .registry
            .messages
            .iter()
            .filter(|message| {
                message.recipient_session_id == record.id
                    && message.recipient_incarnation == recipient_incarnation
                    && message.state == "unread"
                    && !seen_message_ids.contains(&message.message_id)
            })
            .collect();

        if !unread.is_empty() {
            let rows: Vec<_> = unread
                .iter()
                .map(|message| {
                    seen_message_ids.insert(message.message_id.clone());
                    mailbox::metadata(message)
                })
                .collect();
            let cursor = WaitCursor {
                schema_version: CURSOR_SCHEMA.to_string(),
                recipient_session_id: record.id.clone(),
                recipient_incarnation: recipient_incarnation.clone(),
                seen_message_ids,
            };
            if registry_changed {
                locked.save()?;
            }
            return Ok(json!({
                "schema_version": "agent-session.message-wait-any-unread.v1",
                "status": "arrived",
                "messages": rows,
                "cursor": encode_cursor(&cursor)?,
            }));
        }
        if registry_changed {
            locked.save()?;
        }
        drop(locked);

        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(timeout_error());
        }
        let wait = if cancelled.is_some() {
            remaining.min(CANCELLATION_CHECK_INTERVAL)
        } else {
            remaining
        };
        match events_rx.recv_timeout(wait) {
            Ok(Ok(_event)) => {}
            Ok(Err(_)) => return Err(notification_error()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(notification_error()),
        }
    }
}

fn changes_registry(event: &Event, registry_path: &std::path::Path) -> bool {
    event.paths.iter().any(|path| path == registry_path)
        && matches!(
            event.kind,
            EventKind::Create(_)
                | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_))
                | EventKind::Access(AccessKind::Close(AccessMode::Write))
        )
}

fn encode_cursor(cursor: &WaitCursor) -> Result<String, CliError> {
    if cursor.seen_message_ids.len() > MAX_CURSOR_IDS
        || cursor.seen_message_ids.iter().any(|id| {
            id.is_empty() || id.len() > MAX_CURSOR_ID_BYTES || id.chars().any(char::is_control)
        })
    {
        return Err(cursor_invalid());
    }
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(cursor).expect("wait cursor serializes"));
    if encoded.len() > MAX_CURSOR_BYTES {
        return Err(cursor_invalid());
    }
    Ok(encoded)
}

fn parse_cursor(value: &str) -> Result<WaitCursor, CliError> {
    if value.is_empty() || value.len() > MAX_CURSOR_BYTES {
        return Err(cursor_invalid());
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| cursor_invalid())?;
    let cursor: WaitCursor = serde_json::from_slice(&bytes).map_err(|_| cursor_invalid())?;
    if cursor.seen_message_ids.len() > MAX_CURSOR_IDS
        || cursor.seen_message_ids.iter().any(|id| {
            id.is_empty() || id.len() > MAX_CURSOR_ID_BYTES || id.chars().any(char::is_control)
        })
        || cursor.recipient_session_id.is_empty()
        || cursor.recipient_session_id.len() > MAX_CURSOR_ID_BYTES
        || cursor.recipient_incarnation.is_empty()
        || cursor.recipient_incarnation.len() > MAX_CURSOR_ID_BYTES
        || cursor.recipient_session_id.chars().any(char::is_control)
        || cursor.recipient_incarnation.chars().any(char::is_control)
    {
        return Err(cursor_invalid());
    }
    Ok(cursor)
}

fn cursor_invalid() -> CliError {
    CliError::data("cursor-invalid", "message wait cursor is invalid", None)
}

fn notification_error() -> CliError {
    CliError::runtime(
        "wait-notification-error",
        "message wait could not monitor mailbox changes",
        Some(json!({
            "retryable": true,
            "next_action": "retry_wait",
            "recovery": {"kind": "recreate_wait"}
        })),
    )
}

fn timeout_error() -> CliError {
    CliError::unavailable(
        "wait-timeout",
        "message wait reached its bounded timeout",
        Some(json!({
            "retryable": true,
            "next_action": "retry_wait",
            "recovery": {"kind": "same_cursor"}
        })),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{WaitCursor, encode_cursor, notification_error, parse_cursor, timeout_error};
    use pretty_assertions::assert_eq;

    #[test]
    fn cursor_round_trips_recipient_binding_and_seen_ids() {
        let cursor = WaitCursor {
            schema_version: super::CURSOR_SCHEMA.to_string(),
            recipient_session_id: "recipient-session".to_string(),
            recipient_incarnation: "recipient-incarnation".to_string(),
            seen_message_ids: BTreeSet::from(["0d2d87fc-7ec2-4b92-a248-4ed17ea2d407".to_string()]),
        };
        let decoded = parse_cursor(&encode_cursor(&cursor).unwrap()).unwrap();
        assert_eq!(decoded.schema_version, cursor.schema_version);
        assert_eq!(decoded.recipient_session_id, cursor.recipient_session_id);
        assert_eq!(decoded.recipient_incarnation, cursor.recipient_incarnation);
        assert_eq!(decoded.seen_message_ids, cursor.seen_message_ids);
    }

    #[test]
    fn cursor_rejects_malformed_values() {
        assert!(parse_cursor("not-a-cursor").is_err());
        assert!(parse_cursor(&"A".repeat(super::MAX_CURSOR_BYTES + 1)).is_err());
        let too_many = WaitCursor {
            schema_version: super::CURSOR_SCHEMA.to_string(),
            recipient_session_id: "recipient-session".to_string(),
            recipient_incarnation: "recipient-incarnation".to_string(),
            seen_message_ids: (0..=super::MAX_CURSOR_IDS)
                .map(|index| format!("message-{index}"))
                .collect(),
        };
        assert_eq!(
            encode_cursor(&too_many).unwrap_err().code(),
            "cursor-invalid"
        );
    }

    #[test]
    fn recoverable_wait_errors_expose_typed_recovery_details() {
        for error in [timeout_error(), notification_error()] {
            let data = error.into_inner();
            let details = data.details.expect("recovery details");
            assert_eq!(details["retryable"].as_bool(), Some(true));
            assert_eq!(details["next_action"].as_str(), Some("retry_wait"));
            assert!(details["recovery"].is_object());
            assert!(data.message.contains("wait"));
            assert!(!data.message.contains('/'));
            assert_eq!(
                data.exit_code,
                if data.code == "wait-timeout" { 69 } else { 1 }
            );
        }
    }
}
