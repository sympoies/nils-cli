//! Machine notice projection requires an immutable producer/physical-row join.
use super::*;
use crate::send::claude_notice::{sha256, store};
use std::collections::hash_map::Entry;

#[derive(Default)]
pub(crate) struct Bindings {
    rows: HashMap<u64, Option<store::Confirmed>>,
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
        let confirmed = store::confirmed(state, deadline).map_err(|_| HistoryError::Io)?;
        let mut rows = HashMap::new();
        for proof in confirmed {
            let binding = &proof.claim.binding;
            if proof.claim.history_id != session.id
                || binding.provider_session_id != session.provider_session_id
                || binding.launch.profile != session.agent_profile
            {
                continue;
            }
            match rows.entry(proof.row_offset) {
                Entry::Vacant(entry) => {
                    entry.insert(Some(proof));
                }
                Entry::Occupied(mut entry) => {
                    entry.insert(None);
                }
            }
        }
        Ok(Self { rows })
    }

    pub(crate) fn project(
        &self,
        session: &HistorySession,
        offset: u64,
        line: &[u8],
        value: &Value,
    ) -> Decision {
        let Some(proof) = self.rows.get(&offset) else {
            return Decision::NotBound;
        };
        let Some(proof) = proof else {
            return Decision::Rejected;
        };
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
            history_turn_id: None,
            prompt_request: None,
            role: "notice".into(),
            text: row.text,
            timestamp: row.timestamp,
            images: Vec::new(),
            parts: None,
            human_prompt: false,
        }))
    }
}
