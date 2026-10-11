//! Bounded context-window facts from provider-owned session transcripts.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct ContextUsage {
    pub(crate) used_tokens: Option<u64>,
    pub(crate) window_tokens: Option<u64>,
    pub(crate) source: Option<String>,
    pub(crate) observed_at: Option<String>,
    pub(crate) model: Option<String>,
}

#[derive(Default)]
struct Reader {
    bytes_read: usize,
}
impl Reader {
    fn read(&mut self, _path: &Path, _provider: &str, _id: &str) -> Option<ContextUsage> {
        None
    }
}

#[cfg(test)]
#[path = "context_usage_tests.rs"]
mod tests;
