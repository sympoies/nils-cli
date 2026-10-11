//! Bounded context-window facts from provider-owned session transcripts.
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CliContext, SessionRecord, provider_prompt::ProviderPromptSource};

const MAX_TAIL_BYTES: u64 = 256 * 1024;
const MAX_CACHE_ENTRIES: usize = 256;
const SOURCE_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct ContextUsage {
    pub(crate) used_tokens: Option<u64>,
    pub(crate) window_tokens: Option<u64>,
    pub(crate) source: Option<String>,
    pub(crate) observed_at: Option<String>,
    pub(crate) model: Option<String>,
}

impl ContextUsage {
    /// Re-project remote/view values through the same typed fact allowlist.
    pub(crate) fn from_value(value: Value) -> Option<Self> {
        let mut usage: Self = serde_json::from_value(value).ok()?;
        if !matches!(
            usage.source.as_deref(),
            None | Some("claude-transcript" | "codex-rollout")
        ) {
            return None;
        }
        usage.window_tokens = usage.window_tokens.filter(|n| *n > 0);
        usage.model = usage
            .model
            .as_deref()
            .and_then(crate::session_model::model_label);
        usage.observed_at = usage.observed_at.as_deref().and_then(timestamp);
        Some(usage)
    }
}

#[derive(Default)]
struct Cache {
    sources: BTreeMap<String, (Instant, Option<ProviderPromptSource>)>,
    reader: Reader,
}

pub(crate) fn projection(context: &CliContext, record: &SessionRecord) -> Option<ContextUsage> {
    let resume = record.provider_resume.as_ref()?;
    if !matches!(record.agent.as_str(), "claude" | "codex") || resume.provider != record.agent {
        return None;
    }
    // Include configured roots and runtime binding: a resume/profile change
    // must never reuse another provider session's discovery result.
    let key = serde_json::to_string(&(
        &context.state_dir,
        &record.id,
        &record.created_at,
        record.runtime.as_ref().map(|r| &r.launch_id),
        &resume.provider,
        &resume.session_id,
        crate::session_effective_provider_config_dir(record),
        crate::codex_sessions_root(),
        crate::claude_projects_root(),
    ))
    .ok()?;
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Mutex::default).lock().ok()?;
    if cache
        .sources
        .get(&key)
        .is_none_or(|(at, _)| at.elapsed() >= SOURCE_TTL)
    {
        if cache.sources.len() >= MAX_CACHE_ENTRIES {
            cache.sources.pop_first();
        }
        cache.sources.insert(
            key.clone(),
            (
                Instant::now(),
                crate::provider_prompt::ProviderPromptTail::resolve_source(record),
            ),
        );
    }
    let source = cache.sources.get(&key)?.1.as_ref()?.clone();
    cache
        .reader
        .read_source(&source, &record.agent, &resume.session_id)
}

#[derive(Debug, PartialEq, Eq)]
struct Fingerprint {
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

fn fingerprint(metadata: &fs::Metadata) -> Option<Fingerprint> {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    metadata.is_file().then_some(Fingerprint {
        len: metadata.len(),
        modified: metadata.modified().ok()?,
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
    })
}

#[derive(Default)]
struct Reader {
    entries: BTreeMap<(PathBuf, String, String), (Fingerprint, Option<ContextUsage>)>,
    #[cfg(test)]
    bytes_read: usize,
}

impl Reader {
    fn read_source(
        &mut self,
        source: &ProviderPromptSource,
        provider: &str,
        id: &str,
    ) -> Option<ContextUsage> {
        self.read_with_source(source.path(), provider, id, Some(source))
    }

    #[cfg(test)]
    fn read(&mut self, path: &Path, provider: &str, id: &str) -> Option<ContextUsage> {
        self.read_with_source(path, provider, id, None)
    }

    fn read_with_source(
        &mut self,
        path: &Path,
        provider: &str,
        id: &str,
        source: Option<&ProviderPromptSource>,
    ) -> Option<ContextUsage> {
        let key = (path.to_path_buf(), provider.to_string(), id.to_string());
        let result = self.read_inner(path, provider, id, &key, source);
        if result.is_err() {
            self.entries.remove(&key);
        }
        result.ok().flatten()
    }

    fn read_inner(
        &mut self,
        path: &Path,
        provider: &str,
        id: &str,
        key: &(PathBuf, String, String),
        source: Option<&ProviderPromptSource>,
    ) -> Result<Option<ContextUsage>, ()> {
        let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
        let before = fingerprint(&metadata).ok_or(())?;
        if let Some((cached, usage)) = self.entries.get(key)
            && *cached == before
        {
            return Ok(usage.clone());
        }
        if source.is_some_and(|source| source.validated_path().is_none()) {
            return Err(());
        }
        let mut file = File::open(path).map_err(|_| ())?;
        if fingerprint(&file.metadata().map_err(|_| ())?).as_ref() != Some(&before) {
            return Err(());
        }
        let start = before.len.saturating_sub(MAX_TAIL_BYTES);
        file.seek(SeekFrom::Start(start)).map_err(|_| ())?;
        let mut bytes = Vec::with_capacity((before.len - start) as usize);
        (&mut file)
            .take(before.len - start)
            .read_to_end(&mut bytes)
            .map_err(|_| ())?;
        #[cfg(test)]
        {
            self.bytes_read += bytes.len();
        }
        // Reject a concurrent replacement/rewrite rather than caching a mixed
        // observation under the new file's fingerprint.
        if fingerprint(&fs::symlink_metadata(path).map_err(|_| ())?).as_ref() != Some(&before)
            || fingerprint(&file.metadata().map_err(|_| ())?).as_ref() != Some(&before)
        {
            return Err(());
        }
        let usage = parse_tail(&bytes, start != 0, provider, id);
        if self.entries.len() >= MAX_CACHE_ENTRIES {
            self.entries.pop_first();
        }
        self.entries.insert(key.clone(), (before, usage.clone()));
        Ok(usage)
    }
}

fn timestamp(value: &str) -> Option<String> {
    value.parse::<jiff::Timestamp>().ok().map(|t| t.to_string())
}

fn parse_tail(bytes: &[u8], skip_first: bool, provider: &str, id: &str) -> Option<ContextUsage> {
    let mut latest = None;
    let mut model = None;
    for (index, raw) in bytes.split_inclusive(|b| *b == b'\n').enumerate() {
        if (skip_first && index == 0) || !raw.ends_with(b"\n") {
            continue;
        }
        let Ok(row) = serde_json::from_slice::<Value>(raw) else {
            continue;
        };
        let kind = row.get("type").and_then(Value::as_str);
        if provider == "claude" {
            if row
                .get("sessionId")
                .or_else(|| row.get("session_id"))
                .and_then(Value::as_str)
                != Some(id)
                || row.get("isSidechain").and_then(Value::as_bool) == Some(true)
            {
                continue;
            }
            if kind == Some("system")
                && row.get("subtype").and_then(Value::as_str) == Some("compact_boundary")
            {
                latest = None;
            } else if kind == Some("assistant")
                && let Some(usage) = row.pointer("/message/usage").and_then(Value::as_object)
            {
                let model = row
                    .pointer("/message/model")
                    .and_then(Value::as_str)
                    .and_then(crate::session_model::model_label);
                let used_tokens =
                    usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .and_then(|input| {
                            let cache = |key: &str| usage.get(key).map_or(Some(0), Value::as_u64);
                            input
                                .checked_add(cache("cache_read_input_tokens")?)?
                                .checked_add(cache("cache_creation_input_tokens")?)
                        });
                latest = Some(ContextUsage {
                    used_tokens,
                    window_tokens: model.as_deref().and_then(claude_window),
                    source: Some("claude-transcript".into()),
                    observed_at: row
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .and_then(timestamp),
                    model,
                });
            }
        } else if provider == "codex" {
            if kind == Some("session_meta")
                && row.pointer("/payload/id").and_then(Value::as_str) != Some(id)
            {
                return None;
            }
            if kind == Some("turn_context") {
                model = row
                    .pointer("/payload/model")
                    .and_then(Value::as_str)
                    .and_then(crate::session_model::model_label);
            } else if kind == Some("compacted") {
                latest = None;
                model = None;
            } else if kind == Some("event_msg")
                && row.pointer("/payload/type").and_then(Value::as_str) == Some("token_count")
                && let Some(info) = row.pointer("/payload/info").filter(|v| v.is_object())
            {
                latest = Some(ContextUsage {
                    used_tokens: info
                        .pointer("/last_token_usage/total_tokens")
                        .and_then(Value::as_u64),
                    window_tokens: info
                        .get("model_context_window")
                        .and_then(Value::as_u64)
                        .filter(|n| *n > 0),
                    source: Some("codex-rollout".into()),
                    observed_at: row
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .and_then(timestamp),
                    model: model.clone(),
                });
            }
        }
    }
    latest
}

// Conservative Claude Code variant capacities. A plain ID for a model that
// supports both windows is ambiguous in transcript usage: do not guess from
// account settings or a launch argument that may have changed mid-session.
// https://code.claude.com/docs/en/model-config#extended-context
const CLAUDE_WINDOWS: &[(&str, Option<u64>, Option<u64>)] = &[
    ("claude-3-opus", Some(200_000), None),
    ("claude-3-sonnet", Some(200_000), None),
    ("claude-3-haiku", Some(200_000), None),
    ("claude-3-5-sonnet", Some(200_000), None),
    ("claude-3-5-haiku", Some(200_000), None),
    ("claude-3-7-sonnet", Some(200_000), None),
    ("claude-opus-4", Some(200_000), None),
    ("claude-opus-4-1", Some(200_000), None),
    ("claude-opus-4-5", Some(200_000), None),
    ("claude-haiku-4-5", Some(200_000), None),
    ("claude-sonnet-4", None, Some(1_000_000)),
    ("claude-sonnet-4-5", None, Some(1_000_000)),
    ("claude-sonnet-4-6", None, Some(1_000_000)),
    ("claude-opus-4-6", None, Some(1_000_000)),
    ("claude-opus-4-7", Some(1_000_000), Some(1_000_000)),
    ("claude-opus-4-8", Some(1_000_000), Some(1_000_000)),
    ("claude-opus-5", Some(1_000_000), Some(1_000_000)),
    ("claude-opus-5-5", Some(1_000_000), Some(1_000_000)),
    ("claude-sonnet-5", Some(1_000_000), Some(1_000_000)),
    ("claude-sonnet-5-5", Some(1_000_000), Some(1_000_000)),
    ("claude-haiku-5-5", Some(1_000_000), Some(1_000_000)),
    ("claude-fable-5", Some(1_000_000), Some(1_000_000)),
    ("claude-fable-5-1", Some(1_000_000), Some(1_000_000)),
];

fn claude_window(model: &str) -> Option<u64> {
    let (base, extended) = model
        .strip_suffix("[1m]")
        .map_or((model, false), |m| (m, true));
    let base = base
        .rsplit_once('-')
        .filter(|(_, suffix)| suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_digit()))
        .map_or(base, |(base, _)| base);
    let (_, normal, large) = CLAUDE_WINDOWS.iter().find(|(name, _, _)| *name == base)?;
    if extended { *large } else { *normal }
}

#[cfg(test)]
#[path = "context_usage_tests.rs"]
mod tests;
