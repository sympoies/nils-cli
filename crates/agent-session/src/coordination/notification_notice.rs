//! Mailbox notice bytes share generation ownership with plain reminder attempts.
use super::{CliContext, NotificationCandidate, Registry, coalescing, observation};
use crate::{CliError, SessionRecord, notice::NoticeEnvelope};

pub(super) const FORMAT: &str = "mailbox-notice.v1";

pub(super) fn encode_counts(
    context: &CliContext,
    registry: &Registry,
    session: &str,
    incarnation: &str,
    now: i64,
) -> String {
    let messages: Vec<_> = registry
        .messages
        .iter()
        .filter(|item| {
            item.recipient_session_id == session
                && item.recipient_incarnation == incarnation
                && item.state == "unread"
                && !item.notification_suppressed
                && item.expires_at_epoch > now
        })
        .collect();
    let summary = coalescing::explained_summary(messages.iter().copied());
    let relations = crate::load_session_record(context, session)
        .ok()
        .map(|record| super::super::mailbox_sender_relation::SenderRelations::new(context, &record))
        .unwrap_or_default();
    let body = crate::notice::render_summary(
        "mailbox.wake",
        &summary,
        &relations.notice_guidance(&messages),
    );
    NoticeEnvelope::new("mailbox.wake", "agent-session", None)
        .expect("fixed mailbox notice metadata")
        .encode(&body)
        .expect("nonempty count reminder")
}

pub(super) fn attempt_format(prompt: Option<&str>) -> &'static str {
    match prompt {
        Some(text) if crate::notice::parse_notice(text).is_some() => FORMAT,
        Some(_) => "mailbox-counts.v1",
        None => observation::PROMPT_FORMAT,
    }
}

impl NotificationCandidate {
    pub(crate) fn is_notice(&self) -> bool {
        self.prompt
            .as_deref()
            .is_some_and(|text| crate::notice::parse_notice(text).is_some())
    }

    /// Select before attempting. Missing readiness on an installed module is
    /// uncertainty, whereas an explicitly unattached receiver is unsupported.
    pub(crate) fn prepare_notice(
        &mut self,
        context: &CliContext,
        record: &SessionRecord,
    ) -> Result<(), CliError> {
        if !crate::notice::writers_enabled() {
            return Ok(());
        }
        if record.agent == "claude"
            && crate::send::claude_notice::runtime::readiness(context, record).is_none()
        {
            if crate::send::claude_notice::attach::known_unsupported(context, record) {
                return Ok(());
            }
            return Err(crate::send::claude_notice::runtime::unavailable());
        }
        let locked = super::super::lock_registry_observational(context)?;
        self.prompt = Some(encode_counts(
            context,
            &locked.registry,
            &self.target_session_id,
            &self.target_incarnation,
            jiff::Timestamp::now().as_second(),
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn mailbox_wake_preserves_summary_bytes_including_long_counts() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../docs/specs/fixtures/history-row-classes-v1.json"
        ))
        .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: temp.path().into(),
            host: None,
        };
        for case in corpus["mailbox_byte_invariance"].as_array().unwrap() {
            let mut registry = Registry::default();
            let categories: &[&str] = match case["name"].as_str().unwrap() {
                "fixed" => &[],
                "single-count" => &["report"],
                "all-categories" => &[
                    "uncategorized",
                    "report",
                    "progress",
                    "handoff",
                    "decision",
                    "blocker",
                ],
                name => panic!("unhandled mailbox contract case: {name}"),
            };
            for (index, category) in categories.iter().enumerate() {
                registry.messages.push(serde_json::from_value(json!({
                    "schema_version": "agent-session.message.v1", "message_id": format!("message-{index}"),
                    "sender_session_id": "sender", "sender_incarnation": "sender-incarnation",
                    "recipient_session_id": "recipient", "recipient_incarnation": "incarnation",
                    "state": "unread", "revision": 1, "reply_to": null, "reply_depth": 0,
                    "created_at": "2000-01-01T00:00:00Z", "created_at_epoch": 0,
                    "expires_at": "2000-01-01T00:02:00Z", "expires_at_epoch": 120,
                    "category": category, "body_bytes": 0, "body": ""
                })).unwrap());
            }
            let encoded = if categories.is_empty() {
                NoticeEnvelope::new("mailbox.wake", "agent-session", None)
                    .unwrap()
                    .encode(&crate::notice::render_summary(
                        "mailbox.wake",
                        "agent-session message inbox",
                        "",
                    ))
                    .unwrap()
            } else {
                encode_counts(&context, &registry, "recipient", "incarnation", 100)
            };
            assert_eq!(
                encoded
                    .lines()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("\n")
                    .as_bytes(),
                case["before"].as_str().unwrap().as_bytes(),
                "{}",
                case["name"]
            );
        }
    }
}
