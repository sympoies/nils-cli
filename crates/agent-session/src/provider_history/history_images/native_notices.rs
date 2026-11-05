//! Machine notice projection requires an immutable producer/physical-row join.
use super::*;
use crate::send::claude_notice::{sha256, store};
use std::cell::Cell;

#[derive(Default)]
pub(crate) struct Bindings {
    reader: Option<store::Reader>,
    failed: Cell<bool>,
}

pub(crate) enum Decision {
    NotBound,
    Rejected,
    Notice(Box<HistoryMessage>),
}

impl Bindings {
    pub(crate) fn load(
        state: Option<&Path>,
        session: &HistorySession,
        deadline: Instant,
    ) -> Result<Self, HistoryError> {
        let Some(state) = state else {
            return Ok(Self::default());
        };
        if session.provider != "claude" {
            return Ok(Self::default());
        }
        Ok(Self {
            reader: store::Reader::open(state, &session.id, deadline)
                .map_err(|_| HistoryError::Io)?,
            failed: Cell::new(false),
        })
    }

    pub(crate) fn check(&self) -> Result<(), HistoryError> {
        if self.failed.get() {
            Err(HistoryError::Io)
        } else {
            Ok(())
        }
    }

    pub(crate) fn project(
        &self,
        session: &HistorySession,
        offset: u64,
        line: &[u8],
        value: &Value,
        include_parts: bool,
    ) -> Decision {
        let Some(reader) = &self.reader else {
            return Decision::NotBound;
        };
        let proof = match reader.proof(offset) {
            Ok(proof) => proof,
            Err(_) => {
                self.failed.set(true);
                return Decision::Rejected;
            }
        };
        let Some(proof) = proof else {
            return Decision::NotBound;
        };
        let Some(proof) = proof else {
            return Decision::Rejected;
        };
        if proof.claim.binding.provider_session_id != session.provider_session_id
            || proof.claim.binding.launch.profile != session.agent_profile
        {
            return Decision::Rejected;
        }
        let payload = crate::provider_prompt::native_notice::payload(line);
        if sha256(payload) != proof.row_payload_sha256 {
            return Decision::Rejected;
        }
        let Some(row) = crate::provider_prompt::native_notice::row(value, &proof.claim.binding)
        else {
            return Decision::Rejected;
        };
        if row.native_uuid != proof.native_uuid
            || row.text.len() != proof.claim.envelope_bytes
            || sha256(row.text.as_bytes()) != proof.claim.envelope_sha256
            || row.timestamp != proof.timestamp
        {
            return Decision::Rejected;
        }
        Decision::Notice(Box::new(HistoryMessage {
            id: format!("{}-{offset}", session.id),
            paging_offset: offset,
            history_turn_id: None,
            prompt_request: None,
            row_class: None,
            compaction: None,
            role: "notice".into(),
            text: row.text,
            timestamp: row.timestamp,
            images: Vec::new(),
            parts: include_parts.then(Vec::new),
            human_prompt: false,
        }))
    }
}
