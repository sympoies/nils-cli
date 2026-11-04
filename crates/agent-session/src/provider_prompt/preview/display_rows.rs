use super::*;

impl PreviewMessages {
    fn compaction(&mut self, timestamp: Option<String>, text: &str) {
        self.last_message = Some(Self::compaction_message(timestamp, text));
    }

    fn compaction_message(timestamp: Option<String>, text: &str) -> LastMessage {
        let (text, compaction) = row_classes::summary(text);
        LastMessage {
            role: "notice",
            text,
            row_class: Some(RowClass::Compaction),
            submitted_at: timestamp,
            truncated: compaction.summary_truncated,
            compaction: Some(compaction),
        }
    }

    pub(in super::super) fn skipped_record(&mut self, bytes: usize) {
        self.compact_echo_bytes = self
            .compact_echo_bytes
            .map(|scanned| scanned.saturating_add(bytes))
            .filter(|scanned| *scanned <= row_classes::ADJACENT_BYTES);
        if let Some((message, scanned)) = self.pending_manual_compaction.take() {
            let scanned = scanned.saturating_add(bytes);
            if scanned >= row_classes::ADJACENT_BYTES {
                self.last_message = Some(message);
            } else {
                self.pending_manual_compaction = Some((message, scanned));
            }
        }
        if let Some((timestamp, scanned, trigger)) = self.pending_compaction.take() {
            let scanned = scanned.saturating_add(bytes);
            if scanned >= row_classes::ADJACENT_BYTES {
                self.compaction(timestamp, "");
            } else {
                self.pending_compaction = Some((timestamp, scanned, trigger));
            }
        }
    }

    pub(super) fn observe_display(
        &mut self,
        value: &Value,
        provider: ProviderKind,
        session: &str,
        raw_bytes: usize,
        complete: bool,
    ) -> bool {
        if provider == ProviderKind::Claude
            && (value["isSidechain"] == true || row_classes::claude_foreign(value, session))
        {
            self.skipped_record(raw_bytes);
            return true;
        }
        if !complete
            && (row_classes::claude_boundary(value)
                || value["isCompactSummary"] == true
                || value["type"] == "compacted")
        {
            return true;
        }
        if provider == ProviderKind::Claude {
            if !complete
                && (self.pending_manual_compaction.is_some()
                    || self
                        .pending_compaction
                        .as_ref()
                        .is_some_and(|(_, _, trigger)| *trigger == CompactionTrigger::Manual))
            {
                return true;
            }
            if let Some((message, scanned)) = self.pending_manual_compaction.take() {
                let scanned = scanned.saturating_add(raw_bytes);
                if scanned <= row_classes::ADJACENT_BYTES
                    && row_classes::claude_compact_echo(value).is_some()
                {
                    self.last_message = Some(message);
                    self.compact_echo_bytes = Some(scanned);
                    return true;
                }
                if scanned >= row_classes::ADJACENT_BYTES
                    || row_classes::claude_logical_record(value)
                {
                    self.last_message = Some(message);
                } else {
                    self.pending_manual_compaction = Some((message, scanned));
                    return true;
                }
            }
            if let Some(scanned) = self.compact_echo_bytes.take() {
                let scanned = scanned.saturating_add(raw_bytes);
                if scanned <= row_classes::ADJACENT_BYTES {
                    if row_classes::claude_compact_echo(value).is_some() {
                        self.compact_echo_bytes = Some(scanned);
                        return true;
                    }
                    if !row_classes::claude_logical_record(value) {
                        self.compact_echo_bytes = Some(scanned);
                    }
                }
            }
            if let Some((timestamp, scanned, trigger)) = self.pending_compaction.take() {
                let scanned = scanned.saturating_add(raw_bytes);
                if scanned > row_classes::ADJACENT_BYTES {
                    self.compaction(timestamp, "");
                } else if trigger != CompactionTrigger::Auto
                    && row_classes::claude_compact_echo(value).is_some()
                {
                    self.compaction(timestamp, "");
                    self.compact_echo_bytes = Some(scanned);
                    return true;
                } else if let Some(summary) = row_classes::claude_summary(value) {
                    if trigger == CompactionTrigger::Manual {
                        self.pending_manual_compaction =
                            Some((Self::compaction_message(timestamp, &summary), scanned));
                    } else {
                        self.compaction(timestamp, &summary);
                        if trigger != CompactionTrigger::Auto {
                            self.compact_echo_bytes = Some(scanned);
                        }
                    }
                    return true;
                } else if scanned == row_classes::ADJACENT_BYTES
                    || row_classes::claude_logical_record(value)
                {
                    self.compaction(timestamp, "");
                } else {
                    self.pending_compaction = Some((timestamp, scanned, trigger));
                    return true;
                }
            }
            if row_classes::claude_boundary(value) {
                self.pending_compaction = Some((
                    provider_timestamp(value),
                    raw_bytes,
                    row_classes::claude_compaction_trigger(value),
                ));
                return true;
            }
        } else if value["type"] == "compacted" {
            self.compaction(
                provider_timestamp(value),
                value
                    .pointer("/payload/message")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
            return true;
        }
        if let Some(text) = row_classes::command_text(provider.as_str(), value) {
            let (text, truncated) = row_classes::bounded_text(&text);
            self.last_message = Some(LastMessage {
                role: "user",
                text,
                row_class: Some(RowClass::RuntimeCommand),
                compaction: None,
                submitted_at: provider_timestamp(value),
                truncated,
            });
            return true;
        }
        if let Some(text) = row_classes::notice::input(provider.as_str(), value, session) {
            self.last_message = Some(LastMessage {
                role: "user",
                text,
                row_class: Some(RowClass::Notice),
                compaction: None,
                submitted_at: provider_timestamp(value),
                truncated: false,
            });
            return true;
        }
        false
    }
}
