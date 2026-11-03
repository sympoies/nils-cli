use super::*;

impl PreviewMessages {
    fn compaction(&mut self, timestamp: Option<String>, text: &str) {
        let (text, compaction) = row_classes::summary(text);
        self.last_message = Some(LastMessage {
            role: "notice",
            text,
            row_class: Some(RowClass::Compaction),
            submitted_at: timestamp,
            truncated: compaction.summary_truncated,
            compaction: Some(compaction),
        });
    }

    pub(in super::super) fn skipped_record(&mut self, bytes: usize) {
        if let Some((timestamp, scanned)) = self.pending_compaction.take() {
            let scanned = scanned.saturating_add(bytes);
            if scanned >= row_classes::ADJACENT_BYTES {
                self.compaction(timestamp, "");
            } else {
                self.pending_compaction = Some((timestamp, scanned));
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
            if let Some((timestamp, scanned)) = self.pending_compaction.take() {
                let scanned = scanned.saturating_add(raw_bytes);
                if scanned > row_classes::ADJACENT_BYTES {
                    self.compaction(timestamp, "");
                } else if let Some(summary) = row_classes::claude_summary(value) {
                    self.compaction(timestamp, &summary);
                    return true;
                } else if scanned == row_classes::ADJACENT_BYTES
                    || row_classes::claude_logical_record(value)
                {
                    self.compaction(timestamp, "");
                } else {
                    self.pending_compaction = Some((timestamp, scanned));
                    return true;
                }
            }
            if row_classes::claude_boundary(value) {
                self.pending_compaction = Some((provider_timestamp(value), raw_bytes));
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
