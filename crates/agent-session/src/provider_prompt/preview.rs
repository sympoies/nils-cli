use super::*;

/// Response-only previews; never stored in a session record or board entry.
#[derive(Debug, Default, Serialize)]
pub(crate) struct SessionPreviews {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_prompt: Option<LastPrompt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_message: Option<LastMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_prompt_state: Option<crate::serve::LastPromptState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_prompt_continuity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LastMessage {
    pub(crate) role: &'static str,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) submitted_at: Option<String>,
    pub(crate) truncated: bool,
}

impl LastMessage {
    fn from_prompt(role: &'static str, prompt: ParsedPrompt) -> Self {
        let prompt = LastPrompt::from(prompt);
        Self {
            role,
            text: prompt.text,
            submitted_at: prompt.submitted_at,
            truncated: prompt.truncated,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct PreviewMessages {
    pub(super) last_message: Option<LastMessage>,
    claude_human_seen: bool,
}

impl PreviewMessages {
    /// Observe transcript order independently of submission events, whose Claude
    /// canonicalization delay must not delay or reorder list previews.
    pub(super) fn observe(
        &mut self,
        value: &Value,
        provider: ProviderKind,
        session: &str,
    ) -> Option<ParsedPrompt> {
        let prompt = match provider {
            ProviderKind::Codex => parse_codex_prompt_value(value),
            ProviderKind::Claude => {
                let queued = parse_claude_enqueued_prompt_value(value, session);
                let human = queued.is_some() || claude_prompt_is_human_submitted(value);
                let prompt = queued.or_else(|| parse_claude_user_prompt_value(value, session));
                if let Some(prompt) = prompt {
                    let head = prompt.prompt.trim_start();
                    if crate::coordination::is_mailbox_reminder_prompt(&prompt.prompt)
                        || head.starts_with(CLAUDE_INTERRUPT_MARKER_PREFIX)
                        || CLAUDE_MACHINE_PROMPT_PREFIXES
                            .iter()
                            .any(|prefix| head.starts_with(prefix))
                    {
                        return None;
                    }
                    if human {
                        self.claude_human_seen = true;
                    }
                    if human || !self.claude_human_seen {
                        Some(prompt)
                    } else {
                        None
                    }
                } else if !self.claude_human_seen {
                    // Legacy transcripts without human-source tags retain the
                    // canonical last-prompt fallback, but it is not a message.
                    if let Some(prompt) = parse_claude_last_prompt_value(value, session) {
                        return Some(prompt);
                    }
                    None
                } else {
                    None
                }
            }
        };
        if let Some(prompt) = &prompt {
            self.last_message = Some(LastMessage::from_prompt("user", prompt.clone()));
        } else if let Some(prompt) = assistant_prompt(value, provider, session) {
            self.last_message = Some(LastMessage::from_prompt("assistant", prompt));
        }
        prompt
    }
}

fn assistant_prompt(value: &Value, provider: ProviderKind, session: &str) -> Option<ParsedPrompt> {
    let text = match provider {
        ProviderKind::Claude => {
            if !claude_session_matches(value, session)
                || value.get("type")?.as_str()? != "assistant"
                || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
                || value.get("isMeta").and_then(Value::as_bool) == Some(true)
            {
                return None;
            }
            let message = value.get("message")?;
            if message.get("role")?.as_str()? != "assistant" {
                return None;
            }
            message_text(message.get("content")?, "text")?
        }
        ProviderKind::Codex => {
            let payload = value.get("payload")?;
            match value.get("type")?.as_str()? {
                "event_msg" if payload.get("type")?.as_str()? == "agent_message" => {
                    payload.get("message")?.as_str()?.to_string()
                }
                "response_item"
                    if payload.get("type")?.as_str()? == "message"
                        && payload.get("role")?.as_str()? == "assistant" =>
                {
                    message_text(payload.get("content")?, "output_text")?
                }
                _ => return None,
            }
        }
    };
    let mut prompt = bounded_prompt(&text)?;
    prompt.submitted_at = provider_timestamp(value);
    Some(prompt)
}

fn message_text(content: &Value, kind: &str) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    Some(
        content
            .as_array()?
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some(kind))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

pub(super) fn clean_text(text: String) -> String {
    text.chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect()
}
