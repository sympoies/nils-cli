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

fn suppresses(
    context: &CliContext,
    session: &str,
    category: MessageCategory,
) -> Result<bool, CliError> {
    let path = context
        .state_dir
        .join("coordination/notification-policy.json");
    let bytes = match super::read_private_file(&path, MAX_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(invalid()),
    };
    let policy: Policy = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if policy.schema_version != VERSION {
        return Err(invalid());
    }
    let mut sessions = std::collections::BTreeSet::new();
    let mut suppressed = false;
    for recipient in policy.recipients {
        if recipient.session_id.is_empty() || !sessions.insert(recipient.session_id.clone()) {
            return Err(invalid());
        }
        if recipient.session_id == session {
            suppressed = recipient
                .suppress_categories
                .iter()
                .any(|selected| selected.matches(category));
        }
    }
    Ok(suppressed)
}

/// The caller holds the destination registry lock and persists this decision
/// with the message and replay receipt. Sender fields never select a policy.
pub(super) fn admit(
    context: &CliContext,
    registry: &mut Registry,
    message: &mut StoredMessage,
    now: i64,
) -> Result<NotificationProjection, CliError> {
    message.notification_suppressed = suppresses(
        context,
        &message.recipient_session_id,
        message.category.unwrap_or_default(),
    )?;
    if message.notification_suppressed {
        return Ok(NotificationProjection {
            state: "suppressed".into(),
            generation: 0,
            notified_generation: 0,
            last_reason: Some("recipient-category-policy".into()),
            controller_available: false,
        });
    }
    Ok(super::notification::schedule(
        registry,
        &message.recipient_session_id,
        &message.recipient_incarnation,
        now,
    ))
}
