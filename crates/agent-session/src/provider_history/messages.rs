use super::*;

pub(crate) fn apply_prompt_request_bindings(
    messages: &mut [HistoryMessage],
    bindings: &HashMap<String, PromptRequestBinding>,
) {
    for message in messages {
        message.prompt_request = message
            .history_turn_id
            .as_ref()
            .and_then(|turn_id| bindings.get(turn_id).cloned());
    }
}

impl HistoryCatalog {
    pub(crate) fn messages(
        &self,
        history_id: &str,
        cursor: Option<&str>,
        limit: usize,
        direction: HistoryMessageDirection,
    ) -> Result<HistoryMessagesPage, HistoryError> {
        self.messages_with_parts(history_id, cursor, limit, direction, false)
    }

    pub(crate) fn messages_with_parts(
        &self,
        history_id: &str,
        cursor: Option<&str>,
        limit: usize,
        direction: HistoryMessageDirection,
        include_parts: bool,
    ) -> Result<HistoryMessagesPage, HistoryError> {
        let snapshot = self.snapshot();
        let session = snapshot
            .sessions
            .iter()
            .find(|session| session.id == history_id)
            .ok_or(HistoryError::NotFound)?;
        if session.provider == "dsh" {
            let source = self
                .dsh_sources
                .iter()
                .find(|source| session.agent_profile.as_deref() == Some(&source.agent_profile))
                .ok_or(HistoryError::NotFound)?;
            return read_dsh_messages(
                source,
                &session.provider_session_id,
                cursor,
                limit,
                direction,
                include_parts,
            );
        }
        let mut page = match direction {
            HistoryMessageDirection::Forward => read_messages(
                session,
                parse_message_cursor(cursor)?,
                limit.clamp(1, 100),
                include_parts,
            ),
            HistoryMessageDirection::Latest => {
                if cursor.is_some() {
                    return Err(HistoryError::InvalidCursor);
                }
                history_images::read_messages_reverse(
                    session,
                    None,
                    limit.clamp(1, 100),
                    include_parts,
                )
            }
            HistoryMessageDirection::Older => history_images::read_messages_reverse(
                session,
                Some(parse_required_message_cursor(cursor)?),
                limit.clamp(1, 100),
                include_parts,
            ),
        }?;
        self.project_upload_references(session, &mut page.messages, include_parts);
        if self.session_state_dir.is_some() {
            let bindings = self.prompt_request_bindings(&session.provider_session_id);
            apply_prompt_request_bindings(&mut page.messages, &bindings);
        }
        Ok(page)
    }
}
