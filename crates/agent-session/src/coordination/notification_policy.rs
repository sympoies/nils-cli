//! Operator-owned destination policy, evaluated only during message admission.
use serde::Deserialize;

use super::{Registry, mailbox::StoredMessage, notification::NotificationProjection};
use crate::{CliContext, CliError, cli::MessageCategory};

const VERSION: &str = "agent-session.notification-policy.v1";
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema_version: String,
    recipients: Vec<Recipient>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipient {
    session_id: String,
    suppress_categories: Vec<QuietCategory>,
    #[serde(default)]
    routine_interval_seconds: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum QuietCategory {
    Progress,
    Report,
}

impl QuietCategory {
    fn matches(&self, category: MessageCategory) -> bool {
        matches!(
            (self, category),
            (Self::Progress, MessageCategory::Progress) | (Self::Report, MessageCategory::Report)
        )
    }
}

pub(super) fn is_false(value: &bool) -> bool {
    !value
}

fn invalid() -> CliError {
    CliError::data(
        "notification-policy-invalid",
        "destination notification policy is invalid or untrusted",
        None,
    )
}

fn settings(
    context: &CliContext,
    session: &str,
    category: MessageCategory,
) -> Result<(bool, i64), CliError> {
    let path = context
        .state_dir
        .join("coordination/notification-policy.json");
    let bytes = match super::read_private_file(&path, MAX_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((false, 0)),
        Err(_) => return Err(invalid()),
    };
    let policy: Policy = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if policy.schema_version != VERSION {
        return Err(invalid());
    }
    let mut sessions = std::collections::BTreeSet::new();
    let mut suppressed = false;
    let mut interval = 0;
    for recipient in policy.recipients {
        if recipient.session_id.is_empty()
            || !sessions.insert(recipient.session_id.clone())
            || !matches!(recipient.routine_interval_seconds, 0 | 600)
        {
            return Err(invalid());
        }
        if recipient.session_id == session {
            interval = recipient.routine_interval_seconds;
            suppressed = recipient
                .suppress_categories
                .iter()
                .any(|selected| selected.matches(category));
        }
    }
    Ok((suppressed, interval))
}

/// The caller holds the destination registry lock and persists this decision
/// with the message and replay receipt. Sender fields never select a policy.
pub(super) fn admit(
    context: &CliContext,
    registry: &mut Registry,
    message: &mut StoredMessage,
    now: i64,
) -> Result<NotificationProjection, CliError> {
    let (suppressed, interval) = settings(
        context,
        &message.recipient_session_id,
        message.category.unwrap_or_default(),
    )?;
    message.notification_suppressed =
        suppressed && !super::service::is_service_sender(&message.sender_session_id);
    if message.notification_suppressed {
        return Ok(NotificationProjection {
            state: "suppressed".into(),
            generation: 0,
            notified_generation: 0,
            last_reason: Some("recipient-category-policy".into()),
            controller_available: false,
        });
    }
    let projection = super::notification::schedule(
        registry,
        &message.recipient_session_id,
        &message.recipient_incarnation,
        now,
    );
    super::notification::coalescing::admit(registry, message, interval, now);
    Ok(projection)
}

pub(super) fn coalescing_status(context: &CliContext) -> serde_json::Value {
    let path = context
        .state_dir
        .join("coordination/notification-policy.json");
    let selected = super::read_private_file(&path, MAX_BYTES)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Policy>(&bytes).ok());
    let mut out = serde_json::Map::new();
    if let Some(policy) = selected.filter(|policy| policy.schema_version == VERSION) {
        for recipient in policy.recipients {
            if settings(context, &recipient.session_id).is_ok_and(|(_, interval)| interval == 600) {
                out.insert(recipient.session_id, serde_json::json!(600));
            }
        }
    }
    serde_json::Value::Object(out)
}
