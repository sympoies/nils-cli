//! Mailbox notice bytes share generation ownership with plain reminder attempts.
use super::{CliContext, NotificationCandidate, Registry, coalescing, observation};
use crate::{CliError, SessionRecord, notice::NoticeEnvelope};

pub(super) const FORMAT: &str = "mailbox-notice.v1";

pub(super) fn encode_counts(
    registry: &Registry,
    session: &str,
    incarnation: &str,
    now: i64,
) -> String {
    let body = coalescing::explained_prompt(registry.messages.iter().filter(|item| {
        item.recipient_session_id == session
            && item.recipient_incarnation == incarnation
            && item.state == "unread"
            && !item.notification_suppressed
            && item.expires_at_epoch > now
    }));
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
            &locked.registry,
            &self.target_session_id,
            &self.target_incarnation,
            jiff::Timestamp::now().as_second(),
        ));
        Ok(())
    }
}
