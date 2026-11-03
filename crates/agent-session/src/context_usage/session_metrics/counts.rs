//! One forward pass: physical lines and bounded completion metadata, never text.
use std::{collections::BTreeSet, io::Read};

use serde::Deserialize;

use super::{Metric, Reason};

pub(super) const MAX_COUNT_BYTES: u64 = 8 * 1024 * 1024;
const CHUNK_BYTES: usize = 64 * 1024;
const MAX_RECORD_BYTES: usize = 1024 * 1024;
const MAX_BOUNDARIES: usize = 4096;
const MAX_ID_BYTES: usize = 256;

pub(super) struct Counts {
    pub(super) lines: Metric,
    pub(super) compactions: Metric,
    #[cfg(test)]
    stats: Stats,
}

#[cfg(test)]
#[derive(Default)]
struct Stats {
    bytes: u64,
    max_record_buffer: usize,
    max_identities: usize,
}

impl Counts {
    fn unknown(&mut self, reason: Reason) {
        self.lines = Metric::unknown(reason);
        self.compactions = Metric::unknown(reason);
    }
}

// Unknown fields, including transcript text, are skipped by serde. Retained
// strings are bounded by the record cap and are never serialized or logged.
#[derive(Default, Deserialize)]
struct Metadata {
    #[serde(rename = "type")]
    kind: Option<String>,
    subtype: Option<String>,
    #[serde(rename = "sessionId")]
    session_id_camel: Option<String>,
    session_id: Option<String>,
    #[serde(rename = "isSidechain")]
    sidechain: Option<bool>,
    uuid: Option<String>,
    id: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    #[serde(rename = "type")]
    kind: Option<String>,
    id: Option<String>,
    uuid: Option<String>,
}

#[derive(Default)]
struct Completions {
    identities: BTreeSet<String>,
    anonymous: u64,
    notices: bool,
    unknown: Option<Reason>,
}

impl Completions {
    fn record(&mut self, bytes: &[u8], provider: &str, session_id: &str) {
        if self.unknown.is_some() || bytes.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let Ok(metadata) = serde_json::from_slice::<Metadata>(bytes) else {
            self.unknown = Some(Reason::IncompleteSource);
            return;
        };
        let kind = metadata.kind.as_deref();
        let identity = if provider == "claude" {
            if kind != Some("system") || metadata.subtype.as_deref() != Some("compact_boundary") {
                return;
            }
            if metadata.sidechain == Some(true) {
                return;
            }
            match metadata
                .session_id_camel
                .as_deref()
                .or(metadata.session_id.as_deref())
            {
                Some(id) if id != session_id => return,
                None => {
                    self.unknown = Some(Reason::IncompleteSource);
                    return;
                }
                _ => {}
            }
            metadata.uuid
        } else {
            if kind == Some("event_msg")
                && metadata.payload.as_ref().is_some_and(|p| {
                    matches!(
                        p.kind.as_deref(),
                        Some("context_compacted" | "compaction_completed")
                    )
                })
            {
                self.notices = true;
            }
            if kind != Some("compacted") {
                return;
            }
            metadata
                .id
                .or(metadata.uuid)
                .or_else(|| metadata.payload.and_then(|p| p.id.or(p.uuid)))
        };
        match identity {
            Some(id) if !id.is_empty() && id.len() <= MAX_ID_BYTES => {
                if self.identities.contains(&id) {
                    return;
                }
                if self.identities.len() == MAX_BOUNDARIES {
                    self.unknown = Some(Reason::ReadBudgetExceeded);
                } else {
                    self.identities.insert(id);
                }
            }
            None if provider == "codex" => self.anonymous += 1,
            _ => self.unknown = Some(Reason::AmbiguousCompletionEvidence),
        }
    }

    fn metric(self) -> Metric {
        if let Some(reason) = self.unknown {
            Metric::unknown(reason)
        } else if self.anonymous > 1 || (self.anonymous > 0 && !self.identities.is_empty()) {
            Metric::unknown(Reason::AmbiguousCompletionEvidence)
        } else if self.notices && self.anonymous == 0 && self.identities.is_empty() {
            Metric::unknown(Reason::UnsupportedFormat)
        } else {
            Metric::known(self.identities.len() as u64 + self.anonymous)
        }
    }
}

pub(super) fn scan(
    reader: &mut impl Read,
    length: u64,
    provider: &str,
    session_id: &str,
    mut expired: impl FnMut() -> bool,
) -> Counts {
    let mut result = Counts {
        lines: Metric::known(0),
        compactions: Metric::known(0),
        #[cfg(test)]
        stats: Stats::default(),
    };
    if length > MAX_COUNT_BYTES {
        result.unknown(Reason::ReadBudgetExceeded);
        return result;
    }
    let mut chunk = [0; CHUNK_BYTES];
    let mut record = Vec::new();
    let mut oversized = false;
    let mut completions = Completions::default();
    let mut remaining = length;
    let mut lines = 0;
    while remaining > 0 {
        if expired() {
            result.unknown(Reason::ReadBudgetExceeded);
            return result;
        }
        let wanted = remaining.min(CHUNK_BYTES as u64) as usize;
        let count = match reader.read(&mut chunk[..wanted]) {
            Ok(0) => {
                result.unknown(Reason::IncompleteSource);
                return result;
            }
            Ok(count) => count,
            Err(_) => {
                result.unknown(Reason::SourceUnavailable);
                return result;
            }
        };
        remaining -= count as u64;
        #[cfg(test)]
        {
            result.stats.bytes += count as u64;
        }
        for byte in &chunk[..count] {
            if *byte == b'\n' {
                lines += 1;
                if oversized {
                    completions.unknown = Some(Reason::IncompleteSource);
                } else {
                    completions.record(&record, provider, session_id);
                }
                record.clear();
                oversized = false;
                #[cfg(test)]
                {
                    result.stats.max_identities = result
                        .stats
                        .max_identities
                        .max(completions.identities.len());
                }
            } else if record.len() < MAX_RECORD_BYTES {
                record.push(*byte);
                #[cfg(test)]
                {
                    result.stats.max_record_buffer =
                        result.stats.max_record_buffer.max(record.len());
                }
            } else {
                oversized = true;
            }
        }
    }
    if expired() {
        result.unknown(Reason::ReadBudgetExceeded);
        return result;
    }
    if !record.is_empty() || oversized {
        completions.unknown = Some(Reason::IncompleteSource);
    }
    result.lines = Metric::known(lines);
    result.compactions = completions.metric();
    result
}

#[cfg(test)]
#[path = "counts_tests.rs"]
mod tests;
