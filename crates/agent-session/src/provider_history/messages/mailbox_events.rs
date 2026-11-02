//! Interleaves mailbox read events into one history page by transcript anchor.
//!
//! A page over the raw interval `[lo, hi)` takes the events anchored in
//! `(lo, hi]`; `lo == 0` also takes anchor 0 and an open end takes every later
//! anchor. See `docs/specs/mailbox-read-events-v1.md`.
use super::*;
use crate::coordination::read_events::{store, view};
use serde_json::Value;

pub(crate) struct EventPage {
    pub(crate) messages: Vec<Value>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) older_cursor: Option<String>,
}

fn offset(cursor: &Option<String>) -> Option<u64> {
    cursor.as_deref().and_then(|cursor| cursor.parse().ok())
}

impl HistoryCatalog {
    pub(crate) fn messages_with_mailbox_events(
        &self,
        history_id: &str,
        cursor: Option<&str>,
        limit: usize,
        direction: HistoryMessageDirection,
        include_parts: bool,
    ) -> Result<EventPage, HistoryError> {
        let page = self.messages_with_parts(history_id, cursor, limit, direction, include_parts)?;
        let requested = cursor.and_then(|cursor| cursor.parse::<u64>().ok());
        let (lo, hi) = match direction {
            HistoryMessageDirection::Forward => (requested.unwrap_or(0), offset(&page.next_cursor)),
            HistoryMessageDirection::Latest => (offset(&page.older_cursor).unwrap_or(0), None),
            HistoryMessageDirection::Older => (offset(&page.older_cursor).unwrap_or(0), requested),
        };
        let mut messages = page
            .messages
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| HistoryError::Io)?;
        if let Some(state) = self.session_state_dir.as_deref() {
            let events = store::load(state, history_id).map_err(|_| HistoryError::Io)?;
            let mut pending = events
                .iter()
                .filter(|event| {
                    (lo == 0 || event.anchor > lo) && hi.is_none_or(|hi| event.anchor <= hi)
                })
                .filter_map(|event| {
                    Some((
                        event.anchor,
                        view::message(history_id, event, include_parts)?,
                    ))
                })
                .collect::<Vec<_>>();
            pending.sort_by_key(|(anchor, _)| *anchor);
            let mut pending = pending.into_iter().peekable();
            let mut merged = Vec::with_capacity(messages.len() + pending.len());
            for row in messages {
                let at = row["id"]
                    .as_str()
                    .and_then(|id| id.rsplit_once('-')?.1.parse::<u64>().ok())
                    .unwrap_or(u64::MAX);
                while pending.peek().is_some_and(|(anchor, _)| *anchor <= at) {
                    merged.push(pending.next().expect("peeked").1);
                }
                merged.push(row);
            }
            merged.extend(pending.map(|(_, event)| event));
            messages = merged;
        }
        Ok(EventPage {
            messages,
            next_cursor: page.next_cursor,
            older_cursor: page.older_cursor,
        })
    }
}
