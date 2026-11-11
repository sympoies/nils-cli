//! Current mailbox notice bytes share generation and delivery ownership.
use super::{CliContext, NotificationCandidate, Registry, coalescing};
use crate::{CliError, SessionRecord, notice::NoticeEnvelope};

pub(super) const FORMAT: &str = "mailbox-notice.v1";

pub(super) fn encode_counts(
    context: &CliContext,
    registry: &Registry,
    session: &str,
    incarnation: &str,
    now: i64,
) -> Option<String> {
    if super::super::mailbox_quota::has_pending(registry, session, incarnation, now) {
        return Some(nils_common::coordination_projection::MAILBOX_QUOTA_ALERT_NOTICE.into());
    }
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
    let executable = trusted_executable()?;
    let path = shell_words::quote(executable.to_str()?);
    let recipient = shell_words::quote(session);
    let guidance = format!(
        "Read: {path} message inbox --session {recipient} --format json\nShow: {path} message show --session {recipient} --message MESSAGE_ID --format json\n{}",
        relations.notice_guidance(&messages)
    );
    let body = crate::notice::render_summary("mailbox.wake", &summary, &guidance);
    let notice = NoticeEnvelope::new("mailbox.wake", "agent-session", None)
        .expect("fixed mailbox notice metadata")
        .encode(&crate::notice::generated::body(&body))
        .expect("nonempty count reminder");
    (notice.len() <= 4096).then_some(notice)
}

fn trusted_executable() -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let executable = |path: &std::path::Path| {
        path.is_absolute()
            && std::fs::metadata(path)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if let Some(path) = std::env::var_os("AGENT_SESSION_BIN")
        .map(std::path::PathBuf::from)
        .filter(|p| executable(p))
    {
        return Some(path);
    }
    if let Some(roots) = std::env::var_os("AGENT_RUNTIME_TRUSTED_CLI_ROOT")
        && let Some(path) = std::env::split_paths(&roots)
            .find_map(|root| nils_common::session_binary::trusted_root(&root))
    {
        return Some(path);
    }
    #[cfg(test)]
    return crate::resolve_agent_session_executable().ok();
    #[cfg(not(test))]
    let current = std::env::current_exe().ok()?;
    #[cfg(not(test))]
    crate::stable_executable::launch_executable_from(&current, &[])
        .ok()
        .map(|launch| launch.path)
}

pub(super) fn encode_summary(summary: &str, session: &str) -> Option<String> {
    let executable = trusted_executable()?;
    let path = shell_words::quote(executable.to_str()?);
    let recipient = shell_words::quote(session);
    let guidance = format!(
        "Read: {path} message inbox --session {recipient} --format json\nShow: {path} message show --session {recipient} --message MESSAGE_ID --format json\nsender_relation=peer\n{}",
        nils_common::mailbox_notice::VERIFY_GUIDANCE
    );
    let body = crate::notice::render_summary("mailbox.wake", summary, &guidance);
    let notice = NoticeEnvelope::new("mailbox.wake", "agent-session", None)
        .ok()?
        .encode(&crate::notice::generated::body(&body))
        .ok()?;
    (notice.len() <= 4096).then_some(notice)
}

impl NotificationCandidate {
    /// One line: the Claude paste guard refuses multi-line notices, so the
    /// wake carries the exact read command instead of a notice envelope.
    pub(crate) fn text_wake_prompt(&self) -> String {
        let command = trusted_executable()
            .and_then(|path| Some(shell_words::quote(path.to_str()?).into_owned()))
            .unwrap_or_else(|| crate::executable().to_owned());
        format!(
            "Unread mailbox messages; run {command} message inbox --session {} --format json",
            shell_words::quote(&self.target_session_id)
        )
    }

    pub(crate) fn is_text_wake(&self) -> bool {
        self.prompt.as_deref() == Some(self.text_wake_prompt().as_str())
    }

    /// Preserve the selected route's configuration cause on both queued and
    /// consumed receipts, without changing their scheduling or attempt state.
    pub(crate) fn record_text_wake_reason(
        &self,
        context: &CliContext,
        reason: &str,
    ) -> Result<(), CliError> {
        let mut locked = super::super::lock_registry(context)?;
        let key = super::receipt_key(&self.target_session_id, &self.target_incarnation);
        if let Some(receipt) = locked.registry.notifications.get_mut(&key)
            && receipt.generation == self.generation
        {
            receipt.last_reason_detail = Some(reason.to_owned());
            locked.save()?;
        }
        Ok(())
    }

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
            return Err(CliError::unavailable(
                "notice-writers-disabled",
                "notice writers are disabled",
                None,
            ));
        }
        if record.agent == "claude"
            && crate::send::claude_notice::runtime::readiness(context, record).is_none()
        {
            return Err(crate::send::claude_notice::runtime::unavailable());
        }
        let locked = super::super::lock_registry_observational(context)?;
        self.prompt = Some(
            encode_counts(
                context,
                &locked.registry,
                &self.target_session_id,
                &self.target_incarnation,
                jiff::Timestamp::now().as_second(),
            )
            .ok_or_else(|| {
                CliError::unavailable(
                    "mailbox-helper-unavailable",
                    "trusted agent-session executable unavailable",
                    None,
                )
            })?,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn generated_mailbox_summary_prevents_trailing_ascii_spaces_before_encoding() {
        let encoded =
            encode_summary("generated reminder \nnext line \r\nlast line", "recipient").unwrap();
        let parsed = crate::notice::parse_notice(&encoded).unwrap();
        assert_eq!(
            parsed.body.lines().take(3).collect::<Vec<_>>().join("\n"),
            "Mailbox: generated reminder\nnext line\nlast line"
        );
    }

    #[test]
    fn mailbox_wake_includes_recipient_read_command() {
        let temp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: temp.path().into(),
            host: None,
        };
        let encoded = encode_counts(
            &context,
            &Registry::default(),
            "recipient",
            "incarnation",
            100,
        )
        .unwrap();
        let path = trusted_executable().unwrap();
        let executable = shell_words::quote(path.to_str().unwrap());
        assert!(encoded.contains(&format!(
            "Read: {executable} message inbox --session recipient --format json"
        )));
        assert!(encoded.contains(&format!(
            "Show: {executable} message show --session recipient --message MESSAGE_ID --format json"
        )));
        assert!(encoded.len() <= 4096);
    }

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
                encode_counts(&context, &registry, "recipient", "incarnation", 100).unwrap()
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
