use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::provider_prompt::{
    ProviderPromptSource, claude_user_message_text, parse_history_user_prompt, read_last_prompt,
};

const SCAN_MAX_ENTRIES: usize = 10_000;
const SCAN_MAX_DURATION: Duration = Duration::from_secs(2);
const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024;
const MAX_STAR_BYTES: u64 = 4 * 1024;
const STAR_SCHEMA_VERSION: &str = "agent-session.history-star.v1";
const MAX_PREVIEW_LINES: usize = 128;
const MAX_PREVIEW_CHARS: usize = 240;
const MAX_MESSAGE_CHARS: usize = 64 * 1024;
const MAX_MESSAGE_SCAN_LINES: usize = 10_000;
const MESSAGE_SCAN_MAX_DURATION: Duration = Duration::from_secs(2);
const CATALOG_TTL: Duration = Duration::from_secs(30);
const LATEST_PREVIEW_PAGE_MAX_BYTES: usize = 16 * 1024 * 1024;
const LATEST_PREVIEW_SESSION_MAX_BYTES: usize = 1024 * 1024;
const LATEST_PREVIEW_CACHE_MAX_ENTRIES: usize = 1024;
const REVERSE_MESSAGE_MAX_BYTES: u64 = 16 * 1024 * 1024;
const REVERSE_MESSAGE_CHUNK_BYTES: usize = 64 * 1024;
const INCREMENTAL_CONTINUITY_BYTES: u64 = 256;
const INCREMENTAL_OVERLAP_SEARCH_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HistorySource {
    pub(crate) provider: String,
    pub(crate) agent_profile: Option<String>,
    pub(crate) root: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ArchivedSession {
    pub(crate) schema_version: String,
    pub(crate) history_id: String,
    pub(crate) provider: String,
    pub(crate) provider_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) agent_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    pub(crate) cwd: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) archived_at: String,
}

/// A star is stored on its own, keyed by history id, rather than inside the
/// archive record: a session can be starred before it is ever archived, and an
/// archived session can be unstarred later without rewriting archive metadata.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct StarredSession {
    pub(crate) schema_version: String,
    pub(crate) history_id: String,
    pub(crate) starred_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct HistorySession {
    pub(crate) id: String,
    pub(crate) provider: String,
    pub(crate) provider_session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) agent_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) first_user_prompt_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_user_prompt_preview: Option<String>,
    pub(crate) cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) repo_name: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) archived_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) starred_at: Option<String>,
    pub(crate) resumable: bool,
    #[serde(skip)]
    transcript_path: PathBuf,
    #[serde(skip)]
    incremental_catalog_stamp: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct HistoryPage {
    pub(crate) sessions: Vec<HistorySession>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_cursor: Option<String>,
    pub(crate) truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct HistoryMessage {
    pub(crate) id: String,
    pub(crate) role: String,
    pub(crate) text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timestamp: Option<String>,
    #[serde(skip)]
    pub(crate) human_prompt: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct HistoryMessagesPage {
    pub(crate) messages: Vec<HistoryMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) older_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct IncrementalHistoryCursor {
    pub(crate) source_id: String,
    pub(crate) segment_id: String,
    pub(crate) offset: u64,
    pub(crate) continuity_hash: String,
    #[serde(default)]
    pub(crate) discarding_oversized_line: bool,
}

#[derive(Debug)]
pub(crate) struct IncrementalHistoryPage {
    pub(crate) messages: Vec<HistoryMessage>,
    pub(crate) cursor: IncrementalHistoryCursor,
    pub(crate) caught_up: bool,
    pub(crate) discontinuity: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HistoryMessageDirection {
    #[default]
    Forward,
    Latest,
    Older,
}

#[derive(Debug)]
pub(crate) enum HistoryError {
    NotFound,
    InvalidCursor,
    Io,
}

#[derive(Debug)]
pub(crate) struct PendingArchive {
    archive: ArchivedSession,
    path: PathBuf,
    backup: Option<PathBuf>,
    committed: bool,
}

impl PendingArchive {
    pub(crate) fn commit(mut self) -> ArchivedSession {
        if let Some(backup) = self.backup.take() {
            let _ = fs::remove_file(backup);
        }
        self.committed = true;
        self.archive.clone()
    }

    fn rollback(&mut self) {
        if self.committed {
            return;
        }
        let _ = fs::remove_file(&self.path);
        if let Some(backup) = self.backup.take() {
            let _ = fs::rename(backup, &self.path);
        }
        self.committed = true;
    }
}

impl Drop for PendingArchive {
    fn drop(&mut self) {
        self.rollback();
    }
}

#[derive(Clone, Debug)]
struct CatalogSnapshot {
    sessions: Vec<HistorySession>,
    incremental_segments: BTreeMap<String, Vec<HistorySession>>,
    truncated: bool,
}

#[derive(Debug, Default)]
struct CatalogCache {
    refreshed_at: Option<Instant>,
    snapshot: Option<CatalogSnapshot>,
    latest_prompt_previews: HashMap<PathBuf, LatestPromptCacheEntry>,
}

#[derive(Clone, Debug)]
struct LatestPromptCacheEntry {
    len: u64,
    modified: Option<SystemTime>,
    scanned_bytes: usize,
    preview: Option<String>,
}

#[derive(Debug)]
pub(crate) struct HistoryCatalog {
    sources: Vec<HistorySource>,
    archives_root: PathBuf,
    stars_root: PathBuf,
    cache: Mutex<CatalogCache>,
}

impl HistoryCatalog {
    pub(crate) fn new(
        sources: Vec<HistorySource>,
        archives_root: PathBuf,
        stars_root: PathBuf,
    ) -> Self {
        Self {
            sources,
            archives_root,
            stars_root,
            cache: Mutex::new(CatalogCache::default()),
        }
    }

    pub(crate) fn invalidate(&self) {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.refreshed_at = None;
    }

    #[cfg(test)]
    pub(crate) fn list(
        &self,
        machine: &str,
        query: Option<&str>,
        provider: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<HistoryPage, HistoryError> {
        self.list_with_titles(machine, query, provider, cursor, limit, &BTreeMap::new())
    }

    pub(crate) fn list_with_titles(
        &self,
        machine: &str,
        query: Option<&str>,
        provider: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
        managed_titles: &BTreeMap<String, String>,
    ) -> Result<HistoryPage, HistoryError> {
        let mut snapshot = self.snapshot();
        apply_title_overrides(&mut snapshot.sessions, managed_titles);
        let mut page = paginate_snapshot(snapshot, machine, query, provider, cursor, limit)?;
        let mut prompt_cache = {
            let mut cache = self
                .cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if cache.latest_prompt_previews.len() > LATEST_PREVIEW_CACHE_MAX_ENTRIES {
                cache.latest_prompt_previews.clear();
            }
            cache.latest_prompt_previews.clone()
        };
        enrich_latest_prompt_previews(&mut page.sessions, &mut prompt_cache);
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (path, incoming) in prompt_cache {
            let replace = cache
                .latest_prompt_previews
                .get(&path)
                .is_none_or(|current| {
                    current.len != incoming.len
                        || current.modified != incoming.modified
                        || (current.preview.is_none()
                            && (incoming.preview.is_some()
                                || incoming.scanned_bytes > current.scanned_bytes))
                });
            if replace {
                cache.latest_prompt_previews.insert(path, incoming);
            }
        }
        Ok(page)
    }

    /// Set or clear the star on one history session.
    ///
    /// The record is keyed by the catalog's own id for the session, never by the
    /// caller's string: an id that names no session is a not-found instead of an
    /// orphan record, and no request-controlled value reaches the filesystem.
    pub(crate) fn set_star(
        &self,
        history_id: &str,
        starred_at: Option<&str>,
    ) -> Result<Option<String>, HistoryError> {
        let snapshot = self.snapshot();
        let session = snapshot
            .sessions
            .iter()
            .find(|session| session.id == history_id)
            .ok_or(HistoryError::NotFound)?;
        let stored = match starred_at {
            Some(starred_at) => write_star(&self.stars_root, &session.id, starred_at)
                .map(|record| Some(record.starred_at)),
            None => remove_star(&self.stars_root, &session.id).map(|()| None),
        };
        self.invalidate();
        stored
    }

    pub(crate) fn messages(
        &self,
        history_id: &str,
        cursor: Option<&str>,
        limit: usize,
        direction: HistoryMessageDirection,
    ) -> Result<HistoryMessagesPage, HistoryError> {
        let snapshot = self.snapshot();
        let session = snapshot
            .sessions
            .iter()
            .find(|session| session.id == history_id)
            .ok_or(HistoryError::NotFound)?;
        match direction {
            HistoryMessageDirection::Forward => {
                read_messages(session, parse_message_cursor(cursor)?, limit.clamp(1, 100))
            }
            HistoryMessageDirection::Latest => {
                if cursor.is_some() {
                    return Err(HistoryError::InvalidCursor);
                }
                read_messages_reverse(session, None, limit.clamp(1, 100))
            }
            HistoryMessageDirection::Older => read_messages_reverse(
                session,
                Some(parse_required_message_cursor(cursor)?),
                limit.clamp(1, 100),
            ),
        }
    }

    #[cfg(test)]
    pub(crate) fn incremental_messages(
        &self,
        history_id: &str,
        cursor: Option<&IncrementalHistoryCursor>,
        max_bytes: usize,
    ) -> Result<IncrementalHistoryPage, HistoryError> {
        self.incremental_messages_with_applied_ids(history_id, cursor, &[], max_bytes)
    }

    fn incremental_messages_with_applied_ids(
        &self,
        history_id: &str,
        cursor: Option<&IncrementalHistoryCursor>,
        applied_message_ids: &[String],
        max_bytes: usize,
    ) -> Result<IncrementalHistoryPage, HistoryError> {
        if max_bytes == 0 {
            return Err(HistoryError::InvalidCursor);
        }
        let mut snapshot = self.snapshot();
        if snapshot
            .incremental_segments
            .get(history_id)
            .is_some_and(|segments| {
                segments.iter().any(|session| {
                    session.incremental_catalog_stamp
                        != incremental_catalog_stamp(&session.transcript_path)
                })
            })
        {
            // Creating, removing or renaming a provider segment changes an
            // ancestor directory, while ordinary appends do not. Refresh on
            // that generation change so the general 30-second history-list
            // cache cannot hide a just-created resume/rotation segment.
            self.invalidate();
            snapshot = self.snapshot();
        }
        let segments = snapshot
            .incremental_segments
            .get(history_id)
            .ok_or(HistoryError::NotFound)?;
        let source_id = incremental_source_id(&segments[0]);
        let resolved = resolve_incremental_segment(segments, cursor, &source_id)?;
        let session = &segments[resolved.index];
        let mut file =
            fs::File::open(&session.transcript_path).map_err(|_| HistoryError::NotFound)?;
        let file_len = file.metadata().map_err(|_| HistoryError::Io)?.len();
        let page = read_messages_incremental_from_reader(
            session,
            &mut file,
            file_len,
            resolved.offset,
            max_bytes,
            resolved.discarding_oversized_line,
        )?;
        let continuity_hash = incremental_continuity_hash(&mut file, page.next_offset)?;
        let applied = applied_message_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut emitted = HashSet::new();
        let messages = page
            .messages
            .into_iter()
            .filter(|message| {
                !applied.contains(message.id.as_str()) && emitted.insert(message.id.clone())
            })
            .collect();
        Ok(IncrementalHistoryPage {
            messages,
            cursor: IncrementalHistoryCursor {
                source_id,
                segment_id: resolved.segment_id,
                offset: page.next_offset,
                continuity_hash,
                discarding_oversized_line: page.discarding_oversized_line,
            },
            caught_up: page.next_offset == file_len && resolved.index + 1 == segments.len(),
            discontinuity: resolved.discontinuity,
        })
    }

    pub(crate) fn incremental_messages_for_provider_session(
        &self,
        provider: &str,
        provider_session_id: &str,
        cursor: Option<&IncrementalHistoryCursor>,
        applied_message_ids: &[String],
        max_bytes: usize,
    ) -> Result<IncrementalHistoryPage, HistoryError> {
        let snapshot = self.snapshot();
        let history_id = snapshot
            .sessions
            .iter()
            .find(|session| {
                session.provider == provider && session.provider_session_id == provider_session_id
            })
            .map(|session| session.id.clone())
            .ok_or(HistoryError::NotFound)?;
        self.incremental_messages_with_applied_ids(
            &history_id,
            cursor,
            applied_message_ids,
            max_bytes,
        )
    }

    fn snapshot(&self) -> CatalogSnapshot {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fresh = cache
            .refreshed_at
            .is_some_and(|refreshed_at| refreshed_at.elapsed() < CATALOG_TTL);
        if fresh && let Some(snapshot) = cache.snapshot.clone() {
            return snapshot;
        }
        let snapshot = scan_catalog(&self.sources, &self.archives_root, &self.stars_root);
        cache.refreshed_at = Some(Instant::now());
        cache.snapshot = Some(snapshot.clone());
        snapshot
    }
}

pub(crate) fn stable_history_id(
    provider: &str,
    agent_profile: Option<&str>,
    provider_session_id: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(provider.as_bytes());
    digest.update([0]);
    digest.update(agent_profile.unwrap_or_default().as_bytes());
    digest.update([0]);
    digest.update(provider_session_id.as_bytes());
    let encoded = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("h-{encoded}")
}

pub(crate) fn archive_root(state_dir: &Path) -> PathBuf {
    state_dir.join("history").join("archives")
}

pub(crate) fn star_root(state_dir: &Path) -> PathBuf {
    state_dir.join("history").join("stars")
}

/// Every history id this daemon mints is `stable_history_id`'s `h-<sha256 hex>`.
/// Star records are named after that id, so anything else is refused before it
/// can reach the filesystem as a path segment.
fn is_stable_history_id(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("h-")
        && value[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Store a star for one history id. Writing is atomic (temp file + rename) so a
/// concurrent scan never observes a half-written record, and re-starring an
/// already starred session simply refreshes `starred_at`.
pub(crate) fn write_star(
    root: &Path,
    history_id: &str,
    starred_at: &str,
) -> Result<StarredSession, HistoryError> {
    if !is_stable_history_id(history_id) {
        return Err(HistoryError::NotFound);
    }
    let record = StarredSession {
        schema_version: STAR_SCHEMA_VERSION.to_string(),
        history_id: history_id.to_string(),
        starred_at: starred_at.to_string(),
    };
    let root = prepare_star_root(root)?;
    let path = root.join(format!("{history_id}.json"));
    let temp = root.join(format!(".{history_id}.tmp-{}", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(&record).map_err(|_| HistoryError::Io)?;
    let write = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = options.open(&temp).map_err(|_| HistoryError::Io)?;
        std::io::Write::write_all(&mut file, &bytes).map_err(|_| HistoryError::Io)?;
        file.sync_all().map_err(|_| HistoryError::Io)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(HistoryError::Io);
            }
            Ok(_) | Err(_) => {}
        }
        fs::rename(&temp, &path).map_err(|_| HistoryError::Io)
    })();
    if let Err(error) = write {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(record)
}

/// Clear a star. Unstarring a session that was never starred is a no-op success,
/// so a repeated toggle from two devices settles instead of erroring.
pub(crate) fn remove_star(root: &Path, history_id: &str) -> Result<(), HistoryError> {
    if !is_stable_history_id(history_id) {
        return Err(HistoryError::NotFound);
    }
    let path = root.join(format!("{history_id}.json"));
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(HistoryError::Io),
    }
}

fn prepare_star_root(root: &Path) -> Result<PathBuf, HistoryError> {
    fs::create_dir_all(root).map_err(|_| HistoryError::Io)?;
    if fs::symlink_metadata(root)
        .map_err(|_| HistoryError::Io)?
        .file_type()
        .is_symlink()
    {
        return Err(HistoryError::Io);
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(|_| HistoryError::Io)?;
    Ok(root.to_path_buf())
}

pub(crate) fn write_archive(
    root: &Path,
    archive: &ArchivedSession,
) -> Result<PendingArchive, HistoryError> {
    fs::create_dir_all(root).map_err(|_| HistoryError::Io)?;
    if fs::symlink_metadata(root)
        .map_err(|_| HistoryError::Io)?
        .file_type()
        .is_symlink()
    {
        return Err(HistoryError::Io);
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(|_| HistoryError::Io)?;
    let path = root.join(format!("{}.json", archive.history_id));
    let temp = root.join(format!(
        ".{}.tmp-{}",
        archive.history_id,
        uuid::Uuid::new_v4()
    ));
    let backup = root.join(format!(
        ".{}.backup-{}",
        archive.history_id,
        uuid::Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec_pretty(archive).map_err(|_| HistoryError::Io)?;
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = options.open(&temp).map_err(|_| HistoryError::Io)?;
        std::io::Write::write_all(&mut file, &bytes).map_err(|_| HistoryError::Io)?;
        file.sync_all().map_err(|_| HistoryError::Io)?;
        let previous = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(HistoryError::Io);
            }
            Ok(_) => {
                fs::rename(&path, &backup).map_err(|_| HistoryError::Io)?;
                Some(backup.clone())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err(HistoryError::Io),
        };
        if fs::rename(&temp, &path).is_err() {
            if let Some(previous) = previous {
                let _ = fs::rename(previous, &path);
            }
            return Err(HistoryError::Io);
        }
        Ok(previous)
    })();
    let previous = match result {
        Ok(previous) => previous,
        Err(error) => {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
    };
    Ok(PendingArchive {
        archive: archive.clone(),
        path,
        backup: previous,
        committed: false,
    })
}

/// The on-disk metadata roots a scan reads alongside the provider transcripts.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) struct HistoryRoots<'a> {
    pub(crate) archives: &'a Path,
    pub(crate) stars: &'a Path,
}

#[cfg(test)]
pub(crate) fn list(
    sources: &[HistorySource],
    roots: HistoryRoots<'_>,
    machine: &str,
    query: Option<&str>,
    provider: Option<&str>,
    cursor: Option<&str>,
    limit: usize,
) -> Result<HistoryPage, HistoryError> {
    let mut page = paginate_snapshot(
        scan_catalog(sources, roots.archives, roots.stars),
        machine,
        query,
        provider,
        cursor,
        limit,
    )?;
    enrich_latest_prompt_previews(&mut page.sessions, &mut HashMap::new());
    Ok(page)
}

fn apply_title_overrides(
    sessions: &mut [HistorySession],
    managed_titles: &BTreeMap<String, String>,
) {
    for session in sessions {
        if session.title.is_none()
            && let Some(title) = managed_titles.get(&session.id)
        {
            session.title = Some(title.clone());
        }
    }
}

fn paginate_snapshot(
    snapshot: CatalogSnapshot,
    machine: &str,
    query: Option<&str>,
    provider: Option<&str>,
    cursor: Option<&str>,
    limit: usize,
) -> Result<HistoryPage, HistoryError> {
    let cursor = cursor.map(parse_list_cursor).transpose()?;
    let limit = limit.clamp(1, 100);
    let mut sessions = snapshot.sessions;
    sessions.retain(|session| {
        provider.is_none_or(|value| value == session.provider)
            && matches_metadata(session, machine, query)
    });
    sessions.sort_by(|left, right| compare_list_keys(&list_key(left), &list_key(right)));
    if let Some(cursor) = cursor {
        sessions.retain(|session| session_is_after_cursor(session, &cursor));
    }
    let has_more = sessions.len() > limit;
    let page = sessions.into_iter().take(limit).collect::<Vec<_>>();
    let next_cursor = has_more
        .then(|| page.last().map(encode_list_cursor))
        .flatten();
    Ok(HistoryPage {
        sessions: page,
        next_cursor,
        truncated: snapshot.truncated,
    })
}

/// The list order and the cursor share one key so paging stays consistent across
/// the starred boundary: starred sessions lead, newest star first, and the
/// unstarred remainder follows in the recency order it always had.
#[derive(Debug)]
struct ListCursor {
    starred_at: Option<String>,
    updated_at: String,
    created_at: String,
    id: String,
}

fn list_key(session: &HistorySession) -> ListCursor {
    ListCursor {
        starred_at: session.starred_at.clone(),
        updated_at: session.updated_at.clone(),
        created_at: session.created_at.clone(),
        id: session.id.clone(),
    }
}

fn compare_list_keys(left: &ListCursor, right: &ListCursor) -> std::cmp::Ordering {
    right
        .starred_at
        .is_some()
        .cmp(&left.starred_at.is_some())
        .then_with(|| right.starred_at.cmp(&left.starred_at))
        .then_with(|| right.updated_at.cmp(&left.updated_at))
        .then_with(|| right.created_at.cmp(&left.created_at))
        .then_with(|| left.id.cmp(&right.id))
}

fn encode_list_cursor(session: &HistorySession) -> String {
    format!(
        "v2|{}|{}|{}|{}",
        session.starred_at.as_deref().unwrap_or("-"),
        session.updated_at,
        session.created_at,
        session.id
    )
}

fn parse_list_cursor(value: &str) -> Result<ListCursor, HistoryError> {
    let mut parts = value.split('|');
    // A v1 cursor predates stars; treating it as unstarred keeps an in-flight
    // page from an older client resuming where it left off.
    let starred_at = match parts.next() {
        Some("v1") => None,
        Some("v2") => match parts.next() {
            Some("-") => None,
            Some(starred_at) if !starred_at.is_empty() => Some(starred_at.to_string()),
            _ => return Err(HistoryError::InvalidCursor),
        },
        _ => return Err(HistoryError::InvalidCursor),
    };
    let cursor = ListCursor {
        starred_at,
        updated_at: parts.next().unwrap_or_default().to_string(),
        created_at: parts.next().unwrap_or_default().to_string(),
        id: parts.next().unwrap_or_default().to_string(),
    };
    if cursor.updated_at.is_empty()
        || cursor.created_at.is_empty()
        || cursor.id.is_empty()
        || parts.next().is_some()
    {
        return Err(HistoryError::InvalidCursor);
    }
    Ok(cursor)
}

fn session_is_after_cursor(session: &HistorySession, cursor: &ListCursor) -> bool {
    compare_list_keys(&list_key(session), cursor) == std::cmp::Ordering::Greater
}

fn scan_catalog(
    sources: &[HistorySource],
    archives_root: &Path,
    stars_root: &Path,
) -> CatalogSnapshot {
    let deadline = Instant::now() + SCAN_MAX_DURATION;
    let mut truncated = false;
    let archives = read_archives(archives_root, deadline, &mut truncated);
    let stars = read_stars(stars_root, deadline, &mut truncated);
    let mut visited = 0usize;
    let mut sessions = Vec::new();
    let mut incremental_segments = BTreeMap::<String, Vec<HistorySession>>::new();
    let mut seen = HashSet::new();

    for source in sources {
        let mut paths = Vec::new();
        collect_jsonl(
            &source.root,
            if source.provider == "codex" { 6 } else { 2 },
            0,
            &mut paths,
            &mut visited,
            deadline,
            &mut truncated,
        );
        paths.sort();
        for path in paths {
            if Instant::now() >= deadline {
                truncated = true;
                break;
            }
            let Some(mut session) = inspect_history_file(source, &path, deadline) else {
                continue;
            };
            let archive = archives.get(&session.id);
            if let Some(archive) = archive {
                session.id = archive.history_id.clone();
                session.agent_profile = archive.agent_profile.clone();
                session.title = archive.title.clone();
                session.cwd = archive.cwd.clone();
                session.repo_name = repo_name(&archive.cwd);
                session.archived_at = Some(archive.archived_at.clone());
            }
            session.starred_at = stars.get(&session.id).cloned();
            incremental_segments
                .entry(session.id.clone())
                .or_default()
                .push(session.clone());
            if !seen.insert(session.id.clone()) {
                continue;
            }
            sessions.push(session);
            if Instant::now() >= deadline {
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
    }

    for segments in incremental_segments.values_mut() {
        segments.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.transcript_path.cmp(&right.transcript_path))
        });
        segments.dedup_by(|left, right| left.transcript_path == right.transcript_path);
    }

    CatalogSnapshot {
        sessions,
        incremental_segments,
        truncated,
    }
}

#[cfg(test)]
pub(crate) fn messages(
    sources: &[HistorySource],
    history_id: &str,
    cursor: Option<&str>,
    limit: usize,
    direction: HistoryMessageDirection,
) -> Result<HistoryMessagesPage, HistoryError> {
    let limit = limit.clamp(1, 100);
    let session = find_session(sources, history_id).ok_or(HistoryError::NotFound)?;
    match direction {
        HistoryMessageDirection::Forward => {
            read_messages(&session, parse_message_cursor(cursor)?, limit)
        }
        HistoryMessageDirection::Latest => {
            if cursor.is_some() {
                return Err(HistoryError::InvalidCursor);
            }
            read_messages_reverse(&session, None, limit)
        }
        HistoryMessageDirection::Older => read_messages_reverse(
            &session,
            Some(parse_required_message_cursor(cursor)?),
            limit,
        ),
    }
}

fn parse_message_cursor(cursor: Option<&str>) -> Result<u64, HistoryError> {
    cursor
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|_| HistoryError::InvalidCursor)
        .map(|cursor| cursor.unwrap_or(0))
}

fn parse_required_message_cursor(cursor: Option<&str>) -> Result<u64, HistoryError> {
    let value = cursor.ok_or(HistoryError::InvalidCursor)?;
    value
        .parse::<u64>()
        .map_err(|_| HistoryError::InvalidCursor)
}

#[cfg(test)]
fn find_session(sources: &[HistorySource], history_id: &str) -> Option<HistorySession> {
    let deadline = Instant::now() + SCAN_MAX_DURATION;
    let mut visited = 0;
    let mut truncated = false;
    for source in sources {
        let mut paths = Vec::new();
        collect_jsonl(
            &source.root,
            if source.provider == "codex" { 6 } else { 2 },
            0,
            &mut paths,
            &mut visited,
            deadline,
            &mut truncated,
        );
        for path in paths {
            if Instant::now() >= deadline {
                truncated = true;
                break;
            }
            let Some(session) = inspect_history_file(source, &path, deadline) else {
                continue;
            };
            if session.id == history_id {
                return Some(session);
            }
        }
        if truncated {
            break;
        }
    }
    None
}

fn collect_jsonl(
    dir: &Path,
    max_depth: usize,
    depth: usize,
    output: &mut Vec<PathBuf>,
    visited: &mut usize,
    deadline: Instant,
    truncated: &mut bool,
) {
    if depth > max_depth || *truncated {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *visited >= SCAN_MAX_ENTRIES || Instant::now() >= deadline {
            *truncated = true;
            return;
        }
        *visited += 1;
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_jsonl(
                &entry.path(),
                max_depth,
                depth + 1,
                output,
                visited,
                deadline,
                truncated,
            );
        } else if kind.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("jsonl")
        {
            output.push(entry.path());
        }
    }
}

fn inspect_history_file(
    source: &HistorySource,
    path: &Path,
    deadline: Instant,
) -> Option<HistorySession> {
    if Instant::now() >= deadline {
        return None;
    }
    let metadata = fs::metadata(path).ok()?;
    let updated_at = format_system_time(metadata.modified().ok()?);
    let (provider_session_id, cwd, created_at) = match source.provider.as_str() {
        "codex" => {
            let meta = nils_provider_resume::read_codex_resumable_session_meta(path)?;
            (
                meta.session_id,
                meta.cwd,
                format_system_time(meta.created_at),
            )
        }
        "claude" => {
            let id = path.file_stem()?.to_str()?.to_string();
            let cwd = nils_provider_resume::read_claude_session_cwd(path, &id)?;
            (id, cwd, updated_at.clone())
        }
        _ => return None,
    };
    let id = stable_history_id(
        &source.provider,
        source.agent_profile.as_deref(),
        &provider_session_id,
    );
    let first_user_prompt_preview =
        first_prompt(path, &source.provider, &provider_session_id, deadline);
    Some(HistorySession {
        id,
        provider: source.provider.clone(),
        provider_session_id,
        agent_profile: source.agent_profile.clone(),
        title: None,
        prompt_preview: first_user_prompt_preview.clone(),
        first_user_prompt_preview,
        last_user_prompt_preview: None,
        repo_name: repo_name(&cwd),
        cwd,
        created_at,
        updated_at,
        archived_at: None,
        starred_at: None,
        resumable: true,
        transcript_path: path.to_path_buf(),
        incremental_catalog_stamp: incremental_catalog_stamp(path),
    })
}

fn first_prompt(
    path: &Path,
    provider: &str,
    provider_session_id: &str,
    deadline: Instant,
) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    for _ in 0..MAX_PREVIEW_LINES {
        if Instant::now() >= deadline {
            return None;
        }
        let bounded = read_bounded_line(&mut reader, &mut line).ok()?;
        if bounded == BoundedLine::Eof {
            return None;
        }
        if bounded == BoundedLine::Oversized {
            continue;
        }
        let Ok(line) = std::str::from_utf8(&line) else {
            continue;
        };
        let Some(prompt) = parse_history_user_prompt(provider, provider_session_id, line) else {
            continue;
        };
        return Some(truncate_chars(prompt.text.trim(), MAX_PREVIEW_CHARS));
    }
    None
}

fn enrich_latest_prompt_previews(
    sessions: &mut [HistorySession],
    cache: &mut HashMap<PathBuf, LatestPromptCacheEntry>,
) {
    enrich_latest_prompt_previews_with(sessions, cache, |source, budget| {
        read_last_prompt(source, budget)
            .map(|prompt| truncate_chars(prompt.text.trim(), MAX_PREVIEW_CHARS))
    });
}

fn enrich_latest_prompt_previews_with<F>(
    sessions: &mut [HistorySession],
    cache: &mut HashMap<PathBuf, LatestPromptCacheEntry>,
    mut read_preview: F,
) where
    F: FnMut(&ProviderPromptSource, usize) -> Option<String>,
{
    if cache.len() > LATEST_PREVIEW_CACHE_MAX_ENTRIES {
        cache.clear();
    }

    let budget =
        LATEST_PREVIEW_SESSION_MAX_BYTES.min(LATEST_PREVIEW_PAGE_MAX_BYTES / sessions.len().max(1));
    if budget == 0 {
        return;
    }
    for session in sessions.iter_mut() {
        let Ok(metadata) = fs::metadata(&session.transcript_path) else {
            continue;
        };
        let len = metadata.len();
        let modified = metadata.modified().ok();
        if let Some(entry) = cache.get(&session.transcript_path)
            && entry.len == len
            && entry.modified == modified
            && (entry.preview.is_some()
                || entry.scanned_bytes >= usize::try_from(len).unwrap_or(usize::MAX).min(budget))
        {
            session.last_user_prompt_preview = entry.preview.clone();
            continue;
        }
        let preview = ProviderPromptSource::for_history(
            &session.provider,
            session.provider_session_id.clone(),
            session.transcript_path.clone(),
        )
        .and_then(|source| read_preview(&source, budget));
        session.last_user_prompt_preview = preview.clone();
        cache.insert(
            session.transcript_path.clone(),
            LatestPromptCacheEntry {
                len,
                modified,
                scanned_bytes: usize::try_from(len).unwrap_or(usize::MAX).min(budget),
                preview,
            },
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundedLine {
    Eof,
    Line,
    Oversized,
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    output: &mut Vec<u8>,
) -> std::io::Result<BoundedLine> {
    output.clear();
    let mut read_any = false;
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if !read_any {
                BoundedLine::Eof
            } else if oversized {
                BoundedLine::Oversized
            } else {
                BoundedLine::Line
            });
        }
        let end = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        let ends_line = available.get(end.saturating_sub(1)) == Some(&b'\n');
        read_any = true;
        if !oversized {
            let remaining = MAX_LINE_BYTES.saturating_sub(output.len());
            if end <= remaining {
                output.extend_from_slice(&available[..end]);
            } else {
                output.extend_from_slice(&available[..remaining]);
                oversized = true;
            }
        }
        reader.consume(end);
        if ends_line {
            return Ok(if oversized {
                BoundedLine::Oversized
            } else {
                BoundedLine::Line
            });
        }
    }
}

fn read_messages(
    session: &HistorySession,
    offset: u64,
    limit: usize,
) -> Result<HistoryMessagesPage, HistoryError> {
    let file = fs::File::open(&session.transcript_path).map_err(|_| HistoryError::NotFound)?;
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|_| HistoryError::InvalidCursor)?;
    let mut messages = Vec::new();
    let mut line = Vec::new();
    let mut scanned = 0usize;
    let deadline = Instant::now() + MESSAGE_SCAN_MAX_DURATION;
    while messages.len() < limit {
        if scanned >= MAX_MESSAGE_SCAN_LINES || Instant::now() >= deadline {
            let next = reader.stream_position().map_err(|_| HistoryError::Io)?;
            return Ok(HistoryMessagesPage {
                messages,
                next_cursor: Some(next.to_string()),
                older_cursor: None,
            });
        }
        line.clear();
        let line_offset = reader.stream_position().map_err(|_| HistoryError::Io)?;
        let bounded = read_bounded_line(&mut reader, &mut line).map_err(|_| HistoryError::Io)?;
        if bounded == BoundedLine::Eof {
            return Ok(HistoryMessagesPage {
                messages,
                next_cursor: None,
                older_cursor: None,
            });
        }
        scanned += 1;
        if bounded == BoundedLine::Oversized {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let Some((role, text, timestamp, human_prompt)) = normalize_message(
            &session.provider,
            &session.provider_session_id,
            &line,
            &value,
        ) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        messages.push(HistoryMessage {
            id: format!("{}-{line_offset}", session.id),
            role,
            text: truncate_chars(text.trim(), MAX_MESSAGE_CHARS),
            timestamp,
            human_prompt,
        });
    }
    let next = reader.stream_position().map_err(|_| HistoryError::Io)?;
    Ok(HistoryMessagesPage {
        messages,
        next_cursor: Some(next.to_string()),
        older_cursor: None,
    })
}

struct IncrementalReaderPage {
    messages: Vec<HistoryMessage>,
    next_offset: u64,
    discarding_oversized_line: bool,
}

struct ResolvedIncrementalSegment {
    index: usize,
    offset: u64,
    segment_id: String,
    discontinuity: bool,
    discarding_oversized_line: bool,
}

fn resolve_incremental_segment(
    segments: &[HistorySession],
    cursor: Option<&IncrementalHistoryCursor>,
    source_id: &str,
) -> Result<ResolvedIncrementalSegment, HistoryError> {
    let segment_ids = segments
        .iter()
        .map(incremental_segment_id)
        .collect::<Vec<_>>();
    let Some(cursor) = cursor else {
        return Ok(ResolvedIncrementalSegment {
            index: 0,
            offset: 0,
            segment_id: segment_ids[0].clone(),
            discontinuity: false,
            discarding_oversized_line: false,
        });
    };

    if cursor.source_id == source_id
        && let Some(index) = segment_ids
            .iter()
            .position(|segment_id| segment_id == &cursor.segment_id)
    {
        let mut file =
            fs::File::open(&segments[index].transcript_path).map_err(|_| HistoryError::NotFound)?;
        let file_len = file.metadata().map_err(|_| HistoryError::Io)?.len();
        if cursor.offset <= file_len && incremental_continuity_matches(&mut file, cursor)? {
            if cursor.offset == file_len && index + 1 < segments.len() {
                return resolve_incremental_transition(segments, &segment_ids, index + 1, cursor);
            }
            return Ok(ResolvedIncrementalSegment {
                index,
                offset: cursor.offset,
                segment_id: cursor.segment_id.clone(),
                discontinuity: false,
                discarding_oversized_line: cursor.discarding_oversized_line,
            });
        }

        let resumed = find_incremental_continuity(&mut file, file_len, cursor)?;
        return Ok(ResolvedIncrementalSegment {
            index,
            offset: resumed.unwrap_or(0),
            segment_id: compacted_segment_id(&segment_ids[index], cursor, file_len),
            discontinuity: true,
            discarding_oversized_line: resumed.is_some() && cursor.discarding_oversized_line,
        });
    }

    if cursor.source_id == source_id {
        // A compaction generation deliberately gets a new segment id. On the
        // next page, continuity is the authoritative way to re-associate that
        // generation with its still-live path without persisting private paths.
        for (index, session) in segments.iter().enumerate() {
            let mut file = match fs::File::open(&session.transcript_path) {
                Ok(file) => file,
                Err(_) => continue,
            };
            let file_len = file.metadata().map_err(|_| HistoryError::Io)?.len();
            if cursor.offset <= file_len && incremental_continuity_matches(&mut file, cursor)? {
                if cursor.offset == file_len && index + 1 < segments.len() {
                    return resolve_incremental_transition(
                        segments,
                        &segment_ids,
                        index + 1,
                        cursor,
                    );
                }
                return Ok(ResolvedIncrementalSegment {
                    index,
                    offset: cursor.offset,
                    segment_id: cursor.segment_id.clone(),
                    discontinuity: false,
                    discarding_oversized_line: cursor.discarding_oversized_line,
                });
            }
        }

        // The prior path may have been removed after rotation. Search only
        // bounded overlap windows in ordered surviving segments. A matching
        // continuity token advances past the retained prefix/tail instead of
        // replaying it; otherwise start at the oldest surviving boundary so
        // semantic history is not silently skipped.
        for (index, session) in segments.iter().enumerate() {
            let mut file = match fs::File::open(&session.transcript_path) {
                Ok(file) => file,
                Err(_) => continue,
            };
            let file_len = file.metadata().map_err(|_| HistoryError::Io)?.len();
            if let Some(offset) = find_incremental_continuity(&mut file, file_len, cursor)? {
                return Ok(ResolvedIncrementalSegment {
                    index,
                    offset,
                    segment_id: segment_ids[index].clone(),
                    discontinuity: true,
                    discarding_oversized_line: cursor.discarding_oversized_line,
                });
            }
        }
    }

    Ok(ResolvedIncrementalSegment {
        index: 0,
        offset: 0,
        segment_id: segment_ids[0].clone(),
        discontinuity: cursor.source_id != source_id || cursor.offset != 0,
        discarding_oversized_line: false,
    })
}

fn resolve_incremental_transition(
    segments: &[HistorySession],
    segment_ids: &[String],
    index: usize,
    cursor: &IncrementalHistoryCursor,
) -> Result<ResolvedIncrementalSegment, HistoryError> {
    let mut file =
        fs::File::open(&segments[index].transcript_path).map_err(|_| HistoryError::NotFound)?;
    let file_len = file.metadata().map_err(|_| HistoryError::Io)?.len();
    let resumed = find_incremental_continuity(&mut file, file_len, cursor)?;
    Ok(ResolvedIncrementalSegment {
        index,
        offset: resumed.unwrap_or(0),
        segment_id: segment_ids[index].clone(),
        discontinuity: true,
        discarding_oversized_line: resumed.is_some() && cursor.discarding_oversized_line,
    })
}

fn read_messages_incremental_from_reader<R: Read + Seek>(
    session: &HistorySession,
    reader: &mut R,
    file_len: u64,
    offset: u64,
    max_bytes: usize,
    discarding_oversized_line: bool,
) -> Result<IncrementalReaderPage, HistoryError> {
    if offset > file_len || max_bytes == 0 {
        return Err(HistoryError::InvalidCursor);
    }
    if offset == file_len {
        return Ok(IncrementalReaderPage {
            messages: Vec::new(),
            next_offset: offset,
            discarding_oversized_line: false,
        });
    }
    let requested = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .min(file_len.saturating_sub(offset));
    let requested = usize::try_from(requested).map_err(|_| HistoryError::Io)?;
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|_| HistoryError::Io)?;
    let mut bytes = vec![0; requested];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| HistoryError::Io)?;
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    if complete == 0 {
        let oversized = discarding_oversized_line || bytes.len() > MAX_LINE_BYTES;
        return Ok(IncrementalReaderPage {
            messages: Vec::new(),
            next_offset: if oversized {
                offset.saturating_add(bytes.len() as u64)
            } else {
                offset
            },
            discarding_oversized_line: oversized,
        });
    }
    let mut messages = Vec::new();
    let mut lines = bytes[..complete].split_inclusive(|byte| *byte == b'\n');
    if discarding_oversized_line {
        let _ = lines.next();
    }
    for raw_line in lines {
        let line = raw_line.strip_suffix(b"\n").unwrap_or(raw_line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line.len() > MAX_LINE_BYTES {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            continue;
        };
        let Some((role, text, timestamp, human_prompt)) = normalize_message(
            &session.provider,
            &session.provider_session_id,
            line,
            &value,
        ) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        messages.push(HistoryMessage {
            id: incremental_message_id(
                session,
                &value,
                &role,
                text.trim(),
                timestamp.as_deref(),
                human_prompt,
            ),
            role,
            text: truncate_chars(text.trim(), MAX_MESSAGE_CHARS),
            timestamp,
            human_prompt,
        });
    }
    Ok(IncrementalReaderPage {
        messages,
        next_offset: offset.saturating_add(complete as u64),
        discarding_oversized_line: false,
    })
}

fn incremental_source_id(session: &HistorySession) -> String {
    digest_parts(&[
        session.provider.as_bytes(),
        session
            .agent_profile
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
        session.provider_session_id.as_bytes(),
    ])
}

fn incremental_catalog_stamp(path: &Path) -> String {
    let mut stamp = String::new();
    for ancestor in path.ancestors().skip(1).take(4) {
        stamp.push_str(&ancestor.to_string_lossy());
        if let Ok(metadata) = fs::metadata(ancestor) {
            stamp.push('|');
            stamp.push_str(&metadata.len().to_string());
            if let Ok(modified) = metadata.modified()
                && let Ok(elapsed) = modified.duration_since(SystemTime::UNIX_EPOCH)
            {
                stamp.push('|');
                stamp.push_str(&elapsed.as_secs().to_string());
                stamp.push('|');
                stamp.push_str(&elapsed.subsec_nanos().to_string());
            }
        }
        if let Ok(entries) = fs::read_dir(ancestor) {
            let mut names = entries
                .flatten()
                .take(SCAN_MAX_ENTRIES.saturating_add(1))
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let truncated = names.len() > SCAN_MAX_ENTRIES;
            names.truncate(SCAN_MAX_ENTRIES);
            names.sort_unstable();
            stamp.push('|');
            stamp.push_str(if truncated {
                "entries-truncated"
            } else {
                "entries"
            });
            for name in names {
                stamp.push('|');
                stamp.push_str(&name);
            }
        }
        stamp.push('\0');
    }
    digest_parts(&[stamp.as_bytes()])
}

fn incremental_segment_id(session: &HistorySession) -> String {
    let source_id = incremental_source_id(session);
    digest_parts(&[
        source_id.as_bytes(),
        session.transcript_path.to_string_lossy().as_bytes(),
    ])
}

fn compacted_segment_id(
    base_segment_id: &str,
    cursor: &IncrementalHistoryCursor,
    file_len: u64,
) -> String {
    digest_parts(&[
        base_segment_id.as_bytes(),
        cursor.segment_id.as_bytes(),
        cursor.continuity_hash.as_bytes(),
        file_len.to_string().as_bytes(),
    ])
}

fn incremental_continuity_matches<R: Read + Seek>(
    reader: &mut R,
    cursor: &IncrementalHistoryCursor,
) -> Result<bool, HistoryError> {
    Ok(incremental_continuity_hash(reader, cursor.offset)? == cursor.continuity_hash)
}

fn incremental_continuity_hash<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
) -> Result<String, HistoryError> {
    let start = offset.saturating_sub(INCREMENTAL_CONTINUITY_BYTES);
    let len = usize::try_from(offset.saturating_sub(start)).map_err(|_| HistoryError::Io)?;
    reader
        .seek(SeekFrom::Start(start))
        .map_err(|_| HistoryError::Io)?;
    let mut tail = vec![0; len];
    reader.read_exact(&mut tail).map_err(|_| HistoryError::Io)?;
    Ok(digest_parts(&[&tail]))
}

fn find_incremental_continuity<R: Read + Seek>(
    reader: &mut R,
    file_len: u64,
    cursor: &IncrementalHistoryCursor,
) -> Result<Option<u64>, HistoryError> {
    if cursor.offset <= file_len && incremental_continuity_matches(reader, cursor)? {
        return Ok(Some(cursor.offset));
    }
    if cursor.offset == 0 {
        return Ok((cursor.continuity_hash == digest_parts(&[&[]])).then_some(0));
    }

    let prefix_end = file_len.min(INCREMENTAL_OVERLAP_SEARCH_BYTES);
    let mut found =
        search_incremental_continuity_range(reader, 0, prefix_end, &cursor.continuity_hash)?;
    let tail_start = file_len.saturating_sub(INCREMENTAL_OVERLAP_SEARCH_BYTES);
    if tail_start > 0 {
        found = search_incremental_continuity_range(
            reader,
            tail_start,
            file_len,
            &cursor.continuity_hash,
        )?
        .or(found);
    }
    Ok(found)
}

fn search_incremental_continuity_range<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    end: u64,
    expected: &str,
) -> Result<Option<u64>, HistoryError> {
    if start >= end {
        return Ok(None);
    }
    let context_start = start.saturating_sub(INCREMENTAL_CONTINUITY_BYTES);
    let length =
        usize::try_from(end.saturating_sub(context_start)).map_err(|_| HistoryError::Io)?;
    reader
        .seek(SeekFrom::Start(context_start))
        .map_err(|_| HistoryError::Io)?;
    let mut bytes = vec![0; length];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| HistoryError::Io)?;
    let mut found = None;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\n' {
            continue;
        }
        let relative_end = index + 1;
        let offset = context_start.saturating_add(relative_end as u64);
        if offset < start || offset > end {
            continue;
        }
        let relative_start = relative_end.saturating_sub(INCREMENTAL_CONTINUITY_BYTES as usize);
        if digest_parts(&[&bytes[relative_start..relative_end]]) == expected {
            found = Some(offset);
        }
    }
    Ok(found)
}

fn incremental_message_id(
    session: &HistorySession,
    value: &Value,
    role: &str,
    text: &str,
    timestamp: Option<&str>,
    human_prompt: bool,
) -> String {
    let provider_event_id = match session.provider.as_str() {
        "codex" => value
            .pointer("/payload/internal_chat_message_metadata_passthrough/turn_id")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/payload/id").and_then(Value::as_str))
            .or_else(|| value.get("id").and_then(Value::as_str)),
        "claude" => value
            .get("uuid")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/message/id").and_then(Value::as_str)),
        _ => None,
    }
    .unwrap_or_default();
    digest_parts(&[
        session.provider.as_bytes(),
        session.provider_session_id.as_bytes(),
        provider_event_id.as_bytes(),
        role.as_bytes(),
        timestamp.unwrap_or_default().as_bytes(),
        text.as_bytes(),
        if human_prompt { b"human" } else { b"activity" },
    ])
}

fn digest_parts(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part);
        digest.update([0]);
    }
    let bytes = digest.finalize();
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn read_messages_reverse(
    session: &HistorySession,
    before: Option<u64>,
    limit: usize,
) -> Result<HistoryMessagesPage, HistoryError> {
    let deadline = Instant::now() + MESSAGE_SCAN_MAX_DURATION;
    let mut file = fs::File::open(&session.transcript_path).map_err(|_| HistoryError::NotFound)?;
    let file_len = file.metadata().map_err(|_| HistoryError::Io)?.len();
    read_messages_reverse_from_reader(session, &mut file, file_len, before, limit, deadline)
}

fn read_messages_reverse_from_reader<R: Read + Seek>(
    session: &HistorySession,
    reader: &mut R,
    file_len: u64,
    before: Option<u64>,
    limit: usize,
    deadline: Instant,
) -> Result<HistoryMessagesPage, HistoryError> {
    let end = before.unwrap_or(file_len);
    if end > file_len {
        return Err(HistoryError::InvalidCursor);
    }
    if end == 0 {
        return Ok(HistoryMessagesPage {
            messages: Vec::new(),
            next_cursor: None,
            older_cursor: None,
        });
    }
    if before.is_some() {
        reader
            .seek(SeekFrom::Start(end.saturating_sub(1)))
            .map_err(|_| HistoryError::InvalidCursor)?;
        let mut preceding = [0u8; 1];
        reader
            .read_exact(&mut preceding)
            .map_err(|_| HistoryError::InvalidCursor)?;
        if preceding[0] != b'\n' {
            return Err(HistoryError::InvalidCursor);
        }
    }

    let lower_bound = end.saturating_sub(REVERSE_MESSAGE_MAX_BYTES);
    let mut position = end;
    let mut partial = Vec::new();
    let mut partial_oversized = false;
    let mut messages = Vec::new();
    let mut bound_reached = false;
    let mut oldest_offset = None;
    let mut earliest_boundary = None;
    let mut scanned = 0usize;

    'chunks: while position > lower_bound && messages.len() < limit {
        if scanned >= MAX_MESSAGE_SCAN_LINES || Instant::now() >= deadline {
            bound_reached = true;
            break;
        }
        let chunk_start = position
            .saturating_sub(REVERSE_MESSAGE_CHUNK_BYTES as u64)
            .max(lower_bound);
        let chunk_len = usize::try_from(position - chunk_start).map_err(|_| HistoryError::Io)?;
        let mut chunk = vec![0u8; chunk_len];
        reader
            .seek(SeekFrom::Start(chunk_start))
            .map_err(|_| HistoryError::Io)?;
        reader
            .read_exact(&mut chunk)
            .map_err(|_| HistoryError::Io)?;
        position = chunk_start;

        let mut right = chunk.len();
        let mut rightmost = true;
        while let Some(newline) = chunk[..right].iter().rposition(|byte| *byte == b'\n') {
            if scanned >= MAX_MESSAGE_SCAN_LINES || Instant::now() >= deadline {
                bound_reached = true;
                break 'chunks;
            }
            let line_start = newline + 1;
            let line_offset = chunk_start.saturating_add(line_start as u64);
            earliest_boundary = Some(line_offset);
            scanned += 1;

            if rightmost && (!partial.is_empty() || partial_oversized) {
                if !partial_oversized
                    && chunk.len().saturating_sub(line_start) + partial.len() <= MAX_LINE_BYTES
                {
                    let mut joined =
                        Vec::with_capacity(chunk.len().saturating_sub(line_start) + partial.len());
                    joined.extend_from_slice(&chunk[line_start..]);
                    joined.extend_from_slice(&partial);
                    push_reverse_message(session, &joined, line_offset, &mut messages);
                }
                partial.clear();
                partial_oversized = false;
            } else {
                push_reverse_message(
                    session,
                    &chunk[line_start..right],
                    line_offset,
                    &mut messages,
                );
            }
            if messages.len() == limit {
                break 'chunks;
            }
            right = newline;
            rightmost = false;
        }

        if chunk_start == 0 {
            if scanned >= MAX_MESSAGE_SCAN_LINES || Instant::now() >= deadline {
                bound_reached = true;
                break;
            }
            earliest_boundary = Some(0);
            if rightmost && (!partial.is_empty() || partial_oversized) {
                if !partial_oversized && chunk.len() + partial.len() <= MAX_LINE_BYTES {
                    let mut joined = Vec::with_capacity(chunk.len() + partial.len());
                    joined.extend_from_slice(&chunk);
                    joined.extend_from_slice(&partial);
                    push_reverse_message(session, &joined, 0, &mut messages);
                }
            } else {
                push_reverse_message(session, &chunk[..right], 0, &mut messages);
            }
            break;
        }

        let fragment = &chunk[..right];
        if rightmost {
            if partial_oversized || fragment.len() + partial.len() > MAX_LINE_BYTES {
                partial.clear();
                partial_oversized = true;
            } else {
                let mut joined = Vec::with_capacity(fragment.len() + partial.len());
                joined.extend_from_slice(fragment);
                joined.extend_from_slice(&partial);
                partial = joined;
            }
        } else if fragment.len() > MAX_LINE_BYTES {
            partial.clear();
            partial_oversized = true;
        } else {
            partial.clear();
            partial.extend_from_slice(fragment);
        }
    }

    for message in &messages {
        let Some(offset) = message
            .id
            .rsplit('-')
            .next()
            .and_then(|value| value.parse().ok())
        else {
            continue;
        };
        oldest_offset = Some(oldest_offset.map_or(offset, |current: u64| current.min(offset)));
    }
    messages.reverse();
    let unread_older = position > 0
        || lower_bound > 0
        || bound_reached
        || (messages.len() == limit && oldest_offset.is_some_and(|offset| offset > 0));
    let continuation =
        oldest_offset.or_else(|| unread_older.then_some(earliest_boundary).flatten());
    let older_cursor = continuation
        .filter(|offset| *offset > 0 && unread_older)
        .map(|offset| offset.to_string());
    Ok(HistoryMessagesPage {
        messages,
        next_cursor: None,
        older_cursor,
    })
}

fn push_reverse_message(
    session: &HistorySession,
    raw_line: &[u8],
    line_offset: u64,
    messages: &mut Vec<HistoryMessage>,
) {
    let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
    if line.is_empty() || line.len() > MAX_LINE_BYTES {
        return;
    }
    let Ok(value) = serde_json::from_slice::<Value>(line) else {
        return;
    };
    let Some((role, text, timestamp, human_prompt)) = normalize_message(
        &session.provider,
        &session.provider_session_id,
        line,
        &value,
    ) else {
        return;
    };
    if text.trim().is_empty() {
        return;
    }
    messages.push(HistoryMessage {
        id: format!("{}-{line_offset}", session.id),
        role,
        text: truncate_chars(text.trim(), MAX_MESSAGE_CHARS),
        timestamp,
        human_prompt,
    });
}

fn normalize_message(
    provider: &str,
    provider_session_id: &str,
    raw_line: &[u8],
    value: &Value,
) -> Option<(String, String, Option<String>, bool)> {
    let timestamp = value
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    match provider {
        "codex" => {
            if value.get("type").and_then(Value::as_str) != Some("response_item") {
                return None;
            }
            let payload = value.get("payload")?;
            if payload.get("type").and_then(Value::as_str) != Some("message") {
                return None;
            }
            let role = payload.get("role")?.as_str()?;
            if !matches!(role, "user" | "assistant") {
                return None;
            }
            let human_prompt = role == "user"
                && parse_history_user_prompt(
                    provider,
                    provider_session_id,
                    std::str::from_utf8(raw_line).ok()?,
                )
                .is_some();
            let text = content_text(payload.get("content")?);
            Some((role.to_string(), text, timestamp, human_prompt))
        }
        "claude" => {
            let role = value.get("type")?.as_str()?;
            if !matches!(role, "user" | "assistant")
                || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
            {
                return None;
            }
            let message = value.get("message")?;
            let text = if role == "user" {
                claude_user_message_text(value)?
            } else {
                content_text(message.get("content")?)
            };
            let human_prompt = role == "user"
                && parse_history_user_prompt(
                    provider,
                    provider_session_id,
                    std::str::from_utf8(raw_line).ok()?,
                )
                .is_some();
            Some((role.to_string(), text, timestamp, human_prompt))
        }
        _ => None,
    }
}

fn content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| {
            part.get("text")
                .and_then(Value::as_str)
                .or_else(|| part.get("content").and_then(Value::as_str))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn matches_metadata(session: &HistorySession, machine: &str, query: Option<&str>) -> bool {
    let Some(query) = query.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    let query = query.to_lowercase();
    [
        session.title.as_deref(),
        session.prompt_preview.as_deref(),
        Some(session.provider_session_id.as_str()),
        Some(session.provider.as_str()),
        session.agent_profile.as_deref(),
        Some(session.cwd.as_str()),
        session.repo_name.as_deref(),
        Some(machine),
        Some(session.created_at.as_str()),
        Some(session.updated_at.as_str()),
        session.archived_at.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(&query))
}

fn read_archives(
    root: &Path,
    deadline: Instant,
    truncated: &mut bool,
) -> BTreeMap<String, ArchivedSession> {
    let mut archives = BTreeMap::new();
    let Ok(entries) = fs::read_dir(root) else {
        return archives;
    };
    for entry in entries.flatten().take(SCAN_MAX_ENTRIES) {
        if Instant::now() >= deadline {
            *truncated = true;
            break;
        }
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.len() > MAX_ARCHIVE_BYTES {
            continue;
        }
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(archive) = serde_json::from_slice::<ArchivedSession>(&bytes) else {
            continue;
        };
        archives.insert(archive.history_id.clone(), archive);
    }
    archives
}

fn read_stars(root: &Path, deadline: Instant, truncated: &mut bool) -> BTreeMap<String, String> {
    let mut stars = BTreeMap::new();
    let Ok(entries) = fs::read_dir(root) else {
        return stars;
    };
    for entry in entries.flatten().take(SCAN_MAX_ENTRIES) {
        if Instant::now() >= deadline {
            *truncated = true;
            break;
        }
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.len() > MAX_STAR_BYTES {
            continue;
        }
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(star) = serde_json::from_slice::<StarredSession>(&bytes) else {
            continue;
        };
        stars.insert(star.history_id, star.starred_at);
    }
    stars
}

fn format_system_time(value: SystemTime) -> String {
    jiff::Timestamp::try_from(value)
        .map(|timestamp| timestamp.to_string())
        .unwrap_or_default()
}

fn repo_name(cwd: &str) -> Option<String> {
    Path::new(cwd)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut output: String = value.chars().take(max.saturating_sub(1)).collect();
    output.push('…');
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read, Seek};

    struct CountingReader<R> {
        inner: R,
        bytes_read: usize,
    }

    struct GeneratedLongHistoryReader {
        ordinary_row: Vec<u8>,
        semantic_row: Vec<u8>,
        semantic_index: u64,
        records: u64,
        position: u64,
    }

    impl GeneratedLongHistoryReader {
        fn new(row_bytes: usize, records: u64, semantic_index: u64) -> Self {
            fn padded_row(payload: &str, row_bytes: usize) -> Vec<u8> {
                assert!(payload.len() < row_bytes);
                let mut row = Vec::with_capacity(row_bytes);
                row.extend_from_slice(payload.as_bytes());
                row.resize(row_bytes - 1, b' ');
                row.push(b'\n');
                row
            }

            Self {
                ordinary_row: padded_row(
                    r#"{"timestamp":"2026-09-09T00:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{}}}}"#,
                    row_bytes,
                ),
                semantic_row: padded_row(
                    r#"{"timestamp":"2026-09-09T00:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"redesign retitle observability before semantic memory"}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn-semantic","content_item_kinds":["user.text"]}}}"#,
                    row_bytes,
                ),
                semantic_index,
                records,
                position: 0,
            }
        }

        fn len(&self) -> u64 {
            self.records * self.ordinary_row.len() as u64
        }
    }

    impl Read for GeneratedLongHistoryReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.position >= self.len() || buffer.is_empty() {
                return Ok(0);
            }
            let row_bytes = self.ordinary_row.len() as u64;
            let available = self.len().saturating_sub(self.position);
            let target = buffer.len().min(available as usize);
            let mut written = 0usize;
            while written < target {
                let row_index = self.position / row_bytes;
                let row_offset = (self.position % row_bytes) as usize;
                let row = if row_index == self.semantic_index {
                    &self.semantic_row
                } else {
                    &self.ordinary_row
                };
                let count = (target - written).min(row.len() - row_offset);
                buffer[written..written + count]
                    .copy_from_slice(&row[row_offset..row_offset + count]);
                written += count;
                self.position += count as u64;
            }
            Ok(written)
        }
    }

    impl Seek for GeneratedLongHistoryReader {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            let next = match position {
                SeekFrom::Start(offset) => i128::from(offset),
                SeekFrom::End(offset) => i128::from(self.len()) + i128::from(offset),
                SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            };
            if !(0..=i128::from(self.len())).contains(&next) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "seek outside generated transcript",
                ));
            }
            self.position = next as u64;
            Ok(self.position)
        }
    }

    impl<R: Read> Read for CountingReader<R> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = self.inner.read(buffer)?;
            self.bytes_read += read;
            Ok(read)
        }
    }

    impl<R: Seek> Seek for CountingReader<R> {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(position)
        }
    }

    fn history_session(id: &str, updated_at: &str) -> HistorySession {
        HistorySession {
            id: id.to_string(),
            provider: "codex".to_string(),
            provider_session_id: id.to_string(),
            agent_profile: None,
            title: None,
            prompt_preview: None,
            first_user_prompt_preview: None,
            last_user_prompt_preview: None,
            cwd: "/work/example".to_string(),
            repo_name: Some("example".to_string()),
            created_at: updated_at.to_string(),
            updated_at: updated_at.to_string(),
            archived_at: None,
            starred_at: None,
            resumable: true,
            transcript_path: PathBuf::new(),
            incremental_catalog_stamp: String::new(),
        }
    }

    fn codex_meta(session_id: &str, timestamp: &str) -> String {
        format!(
            "{{\"timestamp\":\"{timestamp}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\",\"cwd\":\"/work\",\"source\":\"cli\",\"timestamp\":\"{timestamp}\"}}}}\n"
        )
    }

    fn codex_user_row(timestamp: &str, turn_id: &str, text: &str) -> String {
        format!(
            "{{\"timestamp\":\"{timestamp}\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"{text}\"}}],\"internal_chat_message_metadata_passthrough\":{{\"turn_id\":\"{turn_id}\",\"content_item_kinds\":[\"user.text\"]}}}}}}\n"
        )
    }

    #[test]
    fn starred_sessions_lead_the_list_in_star_recency_order() {
        let mut older_star = history_session("h-older-star", "2026-08-01T00:00:00Z");
        older_star.starred_at = Some("2026-08-10T00:00:00Z".to_string());
        let mut newer_star = history_session("h-newer-star", "2026-08-02T00:00:00Z");
        newer_star.starred_at = Some("2026-08-11T00:00:00Z".to_string());
        let recent = history_session("h-recent", "2026-08-31T00:00:00Z");
        let snapshot = CatalogSnapshot {
            sessions: vec![recent, older_star, newer_star],
            incremental_segments: BTreeMap::new(),
            truncated: false,
        };

        let page = paginate_snapshot(snapshot, "test", None, None, None, 25).unwrap();

        assert_eq!(
            page.sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["h-newer-star", "h-older-star", "h-recent"],
            "stars lead the list newest-first, ahead of unstarred sessions"
        );
    }

    #[test]
    fn star_cursor_pages_across_the_starred_boundary() {
        let mut starred = history_session("h-starred", "2026-08-01T00:00:00Z");
        starred.starred_at = Some("2026-08-10T00:00:00Z".to_string());
        let recent = history_session("h-recent", "2026-08-31T00:00:00Z");
        let older = history_session("h-older", "2026-07-01T00:00:00Z");
        let snapshot = CatalogSnapshot {
            sessions: vec![recent, starred, older],
            incremental_segments: BTreeMap::new(),
            truncated: false,
        };

        let first = paginate_snapshot(snapshot.clone(), "test", None, None, None, 1).unwrap();
        assert_eq!(first.sessions[0].id, "h-starred");
        let cursor = first.next_cursor.clone().expect("cursor for the next page");
        let second = paginate_snapshot(snapshot, "test", None, None, Some(&cursor), 25).unwrap();

        assert_eq!(
            second
                .sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["h-recent", "h-older"],
            "a cursor taken inside the starred group resumes at the unstarred remainder"
        );
    }

    #[test]
    fn stars_are_stored_per_history_id_and_cleared_on_unstar() {
        let tmp = tempfile::tempdir().unwrap();
        let stars = star_root(tmp.path());
        let id = stable_history_id("codex", None, "session-a");

        write_star(&stars, &id, "2026-08-10T00:00:00Z").unwrap();
        let mut truncated = false;
        let stored = read_stars(&stars, Instant::now() + SCAN_MAX_DURATION, &mut truncated);
        assert_eq!(
            stored.get(&id).map(String::as_str),
            Some("2026-08-10T00:00:00Z")
        );

        remove_star(&stars, &id).unwrap();
        let mut truncated = false;
        let cleared = read_stars(&stars, Instant::now() + SCAN_MAX_DURATION, &mut truncated);
        assert!(
            cleared.is_empty(),
            "unstarring removes the stored star record"
        );
        assert!(
            remove_star(&stars, &id).is_ok(),
            "unstarring an unstarred session stays a no-op success"
        );
    }

    #[test]
    fn star_records_reject_history_ids_that_are_not_stable_digests() {
        let tmp = tempfile::tempdir().unwrap();
        let stars = star_root(tmp.path());

        assert!(write_star(&stars, "../escape", "2026-08-10T00:00:00Z").is_err());
        assert!(remove_star(&stars, "../escape").is_err());
        assert!(!tmp.path().join("escape.json").exists());
    }

    #[test]
    fn scanned_sessions_carry_their_stored_star() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions/2026/08/31");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("rollout.jsonl"),
            "{\"timestamp\":\"2026-08-31T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"abc\",\"cwd\":\"/work/example\",\"source\":\"cli\",\"timestamp\":\"2026-08-31T00:00:00Z\"}}\n",
        )
        .unwrap();
        let sources = [HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root: tmp.path().join("sessions"),
        }];
        let stars = star_root(tmp.path());
        write_star(
            &stars,
            &stable_history_id("codex", None, "abc"),
            "2026-08-10T00:00:00Z",
        )
        .unwrap();

        let page = list(
            &sources,
            HistoryRoots {
                archives: &tmp.path().join("archives"),
                stars: &stars,
            },
            "test",
            None,
            None,
            None,
            25,
        )
        .unwrap();

        assert_eq!(page.sessions.len(), 1);
        assert_eq!(
            page.sessions[0].starred_at.as_deref(),
            Some("2026-08-10T00:00:00Z")
        );
    }
    #[test]
    fn metadata_search_does_not_match_non_preview_transcript_text() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions/2026/08/31");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-08-31T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"abc\",\"cwd\":\"/work/example\",\"source\":\"cli\",\"timestamp\":\"2026-08-31T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-08-31T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"injected bootstrap wrapper\"}]}}\n",
                "{\"timestamp\":\"2026-08-31T00:00:02Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"visible preview\"}],\"internal_chat_message_metadata_passthrough\":{\"turn_id\":\"turn-1\",\"content_item_kinds\":[\"user.text\"]}}}\n",
                "{\"timestamp\":\"2026-08-31T00:00:02Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"secret needle\"}]}}\n"
            ),
        )
        .unwrap();
        let sources = [HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root: tmp.path().join("sessions"),
        }];

        let preview = list(
            &sources,
            HistoryRoots {
                archives: &tmp.path().join("archives"),
                stars: &tmp.path().join("stars"),
            },
            "test",
            Some("visible"),
            None,
            None,
            25,
        )
        .unwrap();
        assert_eq!(preview.sessions.len(), 1);
        assert_eq!(
            preview.sessions[0].first_user_prompt_preview.as_deref(),
            Some("visible preview")
        );
        assert_eq!(
            preview.sessions[0].last_user_prompt_preview.as_deref(),
            Some("visible preview")
        );
        let wrapper = list(
            &sources,
            HistoryRoots {
                archives: &tmp.path().join("archives"),
                stars: &tmp.path().join("stars"),
            },
            "test",
            Some("injected bootstrap wrapper"),
            None,
            None,
            25,
        )
        .unwrap();
        assert!(wrapper.sessions.is_empty());
        let body = list(
            &sources,
            HistoryRoots {
                archives: &tmp.path().join("archives"),
                stars: &tmp.path().join("stars"),
            },
            "test",
            Some("secret needle"),
            None,
            None,
            25,
        )
        .unwrap();
        assert!(body.sessions.is_empty());
    }

    #[test]
    fn latest_and_older_message_pages_are_bounded_and_chronological() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions/2026/08/31");
        fs::create_dir_all(&root).unwrap();
        let mut transcript = String::from(
            "{\"timestamp\":\"2026-08-31T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"abc\",\"cwd\":\"/work/example\",\"source\":\"cli\",\"timestamp\":\"2026-08-31T00:00:00Z\"}}\n",
        );
        for index in 1..=6 {
            transcript.push_str(&format!(
                "{{\"timestamp\":\"2026-08-31T00:00:0{index}Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"message {index}\"}}]}}}}\n"
            ));
        }
        fs::write(root.join("rollout.jsonl"), transcript).unwrap();
        let sources = [HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root: tmp.path().join("sessions"),
        }];
        let history_id = stable_history_id("codex", None, "abc");

        let latest = messages(
            &sources,
            &history_id,
            None,
            2,
            HistoryMessageDirection::Latest,
        )
        .unwrap();
        assert_eq!(
            latest
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            vec!["message 5", "message 6"]
        );
        assert!(latest.next_cursor.is_none());
        let older_cursor = latest.older_cursor.as_deref().expect("older page");

        let older = messages(
            &sources,
            &history_id,
            Some(older_cursor),
            2,
            HistoryMessageDirection::Older,
        )
        .unwrap();
        assert_eq!(
            older
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            vec!["message 3", "message 4"]
        );
        assert!(older.older_cursor.is_some());
    }

    #[test]
    fn dense_latest_page_stops_after_one_reverse_chunk() {
        let session = history_session("dense", "2026-09-01T00:00:00Z");
        let mut transcript = vec![b'x'; REVERSE_MESSAGE_CHUNK_BYTES * 2];
        transcript.push(b'\n');
        for index in 0..50 {
            transcript.extend_from_slice(
                format!(
                    "{{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"message {index}\"}}]}}}}\n"
                )
                .as_bytes(),
            );
        }
        let file_len = transcript.len() as u64;
        let mut reader = CountingReader {
            inner: Cursor::new(transcript),
            bytes_read: 0,
        };

        let page = read_messages_reverse_from_reader(
            &session,
            &mut reader,
            file_len,
            None,
            50,
            Instant::now() + MESSAGE_SCAN_MAX_DURATION,
        )
        .unwrap();

        assert_eq!(page.messages.len(), 50);
        assert!(reader.bytes_read <= REVERSE_MESSAGE_CHUNK_BYTES);
    }

    #[test]
    fn generated_200_mib_sparse_semantic_tail_characterizes_v2_starvation() {
        const RECORDS: u64 = 44_000;
        const ROW_BYTES: usize = 4_767;
        const SEMANTIC_RECORD: u64 = 39_000;
        let session = history_session("long-session", "2026-09-09T00:00:00Z");
        let generated = GeneratedLongHistoryReader::new(ROW_BYTES, RECORDS, SEMANTIC_RECORD);
        let file_len = generated.len();
        assert!(
            generated.ordinary_row.len() + generated.semantic_row.len() < 16 * 1024,
            "the generated fixture must not allocate the represented transcript"
        );
        assert!((199 * 1024 * 1024..=201 * 1024 * 1024).contains(&file_len));
        assert!(
            (RECORDS - SEMANTIC_RECORD) * ROW_BYTES as u64 > REVERSE_MESSAGE_MAX_BYTES,
            "the meaningful turn must sit outside the bounded v2 tail"
        );
        let mut reader = CountingReader {
            inner: generated,
            bytes_read: 0,
        };

        let page = read_messages_reverse_from_reader(
            &session,
            &mut reader,
            file_len,
            None,
            100,
            Instant::now() + Duration::from_secs(60),
        )
        .unwrap();

        assert!(
            page.messages.is_empty(),
            "v2 scans a byte-bounded event-dense tail and loses the intermediate semantic turn"
        );
        assert!(page.older_cursor.is_some());
        assert!(reader.bytes_read >= REVERSE_MESSAGE_MAX_BYTES as usize);
        assert!(
            reader.bytes_read <= REVERSE_MESSAGE_MAX_BYTES as usize + REVERSE_MESSAGE_CHUNK_BYTES
        );
    }

    #[test]
    fn generated_200_mib_incremental_v3_projection_recovers_the_semantic_turn() {
        const RECORDS: u64 = 44_000;
        const ROW_BYTES: usize = 4_767;
        const SEMANTIC_RECORD: u64 = 39_000;
        let session = history_session("long-session-v3", "2026-09-09T00:00:00Z");
        let generated = GeneratedLongHistoryReader::new(ROW_BYTES, RECORDS, SEMANTIC_RECORD);
        let file_len = generated.len();
        let mut reader = CountingReader {
            inner: generated,
            bytes_read: 0,
        };
        let mut cursor = 0;
        let mut recovered = None;

        while cursor < file_len {
            let page = read_messages_incremental_from_reader(
                &session,
                &mut reader,
                file_len,
                cursor,
                256 * 1024,
                false,
            )
            .unwrap();
            recovered = recovered.or_else(|| {
                page.messages
                    .iter()
                    .find(|message| message.human_prompt)
                    .map(|message| message.text.clone())
            });
            assert!(page.next_offset > cursor);
            cursor = page.next_offset;
        }

        assert_eq!(
            recovered.as_deref(),
            Some("redesign retitle observability before semantic memory")
        );
        assert_eq!(cursor, file_len);
        let page_count = file_len.div_ceil(256 * 1024) as usize;
        assert!(reader.bytes_read <= file_len as usize + page_count * ROW_BYTES * 2);
        assert!(
            recovered
                .as_deref()
                .is_some_and(|message| message.len() <= MAX_MESSAGE_CHARS)
        );
    }

    #[test]
    fn incremental_cursor_waits_for_a_complete_line() {
        let session = history_session("partial", "2026-09-09T00:00:00Z");
        let first = br#"{"timestamp":"2026-09-09T00:00:00Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"origin"}],"internal_chat_message_metadata_passthrough":{"turn_id":"one","content_item_kinds":["user.text"]}}}
"#;
        let second = br#"{"timestamp":"2026-09-09T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"pivot"}],"internal_chat_message_metadata_passthrough":{"turn_id":"two","content_item_kinds":["user.text"]}}}
"#;
        let mut bytes = first.to_vec();
        bytes.extend_from_slice(second);
        let partial_len = (first.len() + second.len() / 2) as u64;
        let mut reader = Cursor::new(bytes);
        let page = read_messages_incremental_from_reader(
            &session,
            &mut reader,
            partial_len,
            0,
            64 * 1024,
            false,
        )
        .unwrap();
        assert_eq!(page.next_offset, first.len() as u64);
        assert_eq!(page.messages.len(), 1);

        let full_len = reader.get_ref().len() as u64;
        let page = read_messages_incremental_from_reader(
            &session,
            &mut reader,
            full_len,
            page.next_offset,
            64 * 1024,
            false,
        )
        .unwrap();
        assert_eq!(page.next_offset, full_len);
        assert_eq!(page.messages[0].text, "pivot");
    }

    #[test]
    fn incremental_cursor_discards_an_oversized_line_across_bounded_pages() {
        let session = history_session("oversized", "2026-09-09T00:00:00Z");
        let mut bytes = vec![b'x'; MAX_LINE_BYTES * 2 + 17];
        bytes.extend_from_slice(b"\n");
        bytes.extend_from_slice(
            br#"{"timestamp":"2026-09-09T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"after oversized"}],"internal_chat_message_metadata_passthrough":{"turn_id":"after","content_item_kinds":["user.text"]}}}"#,
        );
        bytes.push(b'\n');
        let file_len = bytes.len() as u64;
        let mut reader = Cursor::new(bytes);
        let first = read_messages_incremental_from_reader(
            &session,
            &mut reader,
            file_len,
            0,
            MAX_LINE_BYTES + 1,
            false,
        )
        .unwrap();
        assert!(first.discarding_oversized_line);
        assert!(first.next_offset > 0);
        let mut next_offset = first.next_offset;
        let mut discarding = first.discarding_oversized_line;
        let mut recovered = Vec::new();
        while next_offset < file_len {
            let page = read_messages_incremental_from_reader(
                &session,
                &mut reader,
                file_len,
                next_offset,
                MAX_LINE_BYTES + 1,
                discarding,
            )
            .unwrap();
            assert!(page.next_offset > next_offset);
            assert!(page.next_offset - next_offset <= (MAX_LINE_BYTES + 1) as u64);
            next_offset = page.next_offset;
            discarding = page.discarding_oversized_line;
            recovered.extend(page.messages);
        }
        assert!(!discarding);
        assert_eq!(next_offset, file_len);
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].text, "after oversized");
    }

    #[test]
    fn incremental_identity_detects_rotation_and_compaction() {
        let mut session = history_session("identity", "2026-09-09T00:00:00Z");
        session.transcript_path = PathBuf::from("/history/segment-one.jsonl");
        let mut rotated = session.clone();
        rotated.transcript_path = PathBuf::from("/history/segment-two.jsonl");
        let original = b"first line\nsecond line\n".to_vec();
        let mut reader = Cursor::new(original.clone());
        let source = incremental_source_id(&session);
        let cursor = IncrementalHistoryCursor {
            source_id: source.clone(),
            segment_id: incremental_segment_id(&session),
            offset: original.len() as u64,
            continuity_hash: incremental_continuity_hash(&mut reader, original.len() as u64)
                .unwrap(),
            discarding_oversized_line: false,
        };
        assert!(incremental_continuity_matches(&mut reader, &cursor).unwrap());

        let mut rewritten = Cursor::new(b"FIRST line\nsecond line\n".to_vec());
        assert!(!incremental_continuity_matches(&mut rewritten, &cursor).unwrap());
        let compacted = Cursor::new(b"replacement\n".to_vec());
        assert!(cursor.offset > compacted.get_ref().len() as u64);
        assert_eq!(incremental_source_id(&session), cursor.source_id);
        assert_ne!(
            incremental_segment_id(&session),
            incremental_segment_id(&rotated)
        );
    }

    #[test]
    fn incremental_catalog_restarts_at_a_new_segment_after_compaction() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions/2026/09/09");
        fs::create_dir_all(&root).unwrap();
        let transcript = root.join("rollout.jsonl");
        let row = |text: &str| {
            format!(
                "{{\"timestamp\":\"2026-09-09T00:00:00Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"{text}\"}}],\"internal_chat_message_metadata_passthrough\":{{\"turn_id\":\"turn\",\"content_item_kinds\":[\"user.text\"]}}}}}}\n"
            )
        };
        fs::write(
            &transcript,
            format!(
                "{{\"timestamp\":\"2026-09-09T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"compact-id\",\"cwd\":\"/work\",\"source\":\"cli\",\"timestamp\":\"2026-09-09T00:00:00Z\"}}}}\n{}",
                row("original objective")
            ),
        )
        .unwrap();
        let catalog = HistoryCatalog::new(
            vec![HistorySource {
                provider: "codex".into(),
                agent_profile: None,
                root: tmp.path().join("sessions"),
            }],
            tmp.path().join("archives"),
            tmp.path().join("stars"),
        );
        let history_id = stable_history_id("codex", None, "compact-id");
        let first = catalog
            .incremental_messages(&history_id, None, 64 * 1024)
            .unwrap();
        assert!(first.caught_up);

        fs::write(
            &transcript,
            format!(
                "{{\"timestamp\":\"2026-09-09T00:01:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"compact-id\",\"cwd\":\"/work\",\"source\":\"cli\",\"timestamp\":\"2026-09-09T00:01:00Z\"}}}}\n{}",
                row("post compaction pivot")
            ),
        )
        .unwrap();
        let second = catalog
            .incremental_messages(&history_id, Some(&first.cursor), 64 * 1024)
            .unwrap();
        assert!(second.discontinuity);
        assert!(second.caught_up);
        assert_ne!(second.cursor.segment_id, first.cursor.segment_id);
        assert_eq!(
            second
                .messages
                .iter()
                .find(|message| message.human_prompt)
                .map(|message| message.text.as_str()),
            Some("post compaction pivot")
        );
    }

    #[test]
    fn incremental_catalog_orders_segments_and_suppresses_retained_overlap() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions/2026/09/09");
        fs::create_dir_all(&root).unwrap();
        let first_path = root.join("rollout-2026-09-09T00-00-00-shared.jsonl");
        let second_path = root.join("rollout-2026-09-09T01-00-00-shared.jsonl");
        let origin = codex_user_row("2026-09-09T00:00:01Z", "origin", "origin objective");
        let overlap = codex_user_row("2026-09-09T00:00:02Z", "shared", "shared milestone");
        let pivot = codex_user_row("2026-09-09T01:00:01Z", "pivot", "new segment pivot");
        fs::write(
            &first_path,
            format!(
                "{}{}{}",
                codex_meta("shared-id", "2026-09-09T00:00:00Z"),
                origin,
                overlap
            ),
        )
        .unwrap();
        let catalog = HistoryCatalog::new(
            vec![HistorySource {
                provider: "codex".into(),
                agent_profile: None,
                root: tmp.path().join("sessions"),
            }],
            tmp.path().join("archives"),
            tmp.path().join("stars"),
        );
        let history_id = stable_history_id("codex", None, "shared-id");
        let first = catalog
            .incremental_messages(&history_id, None, 64 * 1024)
            .unwrap();
        assert!(
            first.caught_up,
            "the initial segment is current before rotation"
        );
        assert_eq!(
            first
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            ["origin objective", "shared milestone"]
        );
        fs::write(
            &second_path,
            format!(
                "{}{}{}",
                codex_meta("shared-id", "2026-09-09T01:00:00Z"),
                overlap,
                pivot
            ),
        )
        .unwrap();
        let applied = first
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect::<Vec<_>>();

        let second = catalog
            .incremental_messages_with_applied_ids(
                &history_id,
                Some(&first.cursor),
                &applied,
                64 * 1024,
            )
            .unwrap();

        assert!(second.discontinuity);
        assert!(second.caught_up);
        assert_ne!(second.cursor.segment_id, first.cursor.segment_id);
        assert_eq!(
            second
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<Vec<_>>(),
            ["new segment pivot"]
        );
    }

    #[test]
    fn incremental_message_ids_are_stable_across_segment_offsets_and_content_free() {
        let row = codex_user_row("2026-09-09T00:00:01Z", "stable-turn", "private objective");
        let mut first_session = history_session("stable-id", "2026-09-09T00:00:00Z");
        first_session.provider_session_id = "provider-id".to_string();
        first_session.transcript_path = PathBuf::from("/history/first.jsonl");
        let mut second_session = first_session.clone();
        second_session.transcript_path = PathBuf::from("/history/second.jsonl");
        let mut first_reader = Cursor::new(row.as_bytes());
        let mut prefixed = b"{\"type\":\"turn_context\"}\n".to_vec();
        prefixed.extend_from_slice(row.as_bytes());
        let mut second_reader = Cursor::new(prefixed);
        let first_len = first_reader.get_ref().len() as u64;
        let second_len = second_reader.get_ref().len() as u64;

        let first = read_messages_incremental_from_reader(
            &first_session,
            &mut first_reader,
            first_len,
            0,
            64 * 1024,
            false,
        )
        .unwrap();
        let second = read_messages_incremental_from_reader(
            &second_session,
            &mut second_reader,
            second_len,
            0,
            64 * 1024,
            false,
        )
        .unwrap();

        assert_eq!(first.messages[0].id, second.messages[0].id);
        assert_eq!(first.messages[0].id.len(), "sha256:".len() + 64);
        assert!(first.messages[0].id.starts_with("sha256:"));
        assert!(!first.messages[0].id.contains("private objective"));
    }

    #[test]
    fn incremental_cursor_json_round_trip_is_stable_and_path_free() {
        let cursor = IncrementalHistoryCursor {
            source_id: digest_parts(&[b"source"]),
            segment_id: digest_parts(&[b"segment"]),
            offset: 42,
            continuity_hash: digest_parts(&[b"continuity"]),
            discarding_oversized_line: true,
        };

        let encoded = serde_json::to_string(&cursor).unwrap();
        let decoded = serde_json::from_str::<IncrementalHistoryCursor>(&encoded).unwrap();

        assert_eq!(decoded, cursor);
        assert_eq!(
            encoded,
            format!(
                "{{\"source_id\":\"{}\",\"segment_id\":\"{}\",\"offset\":42,\"continuity_hash\":\"{}\",\"discarding_oversized_line\":true}}",
                cursor.source_id, cursor.segment_id, cursor.continuity_hash
            )
        );
        assert!(!encoded.contains("/history/"));
    }

    #[test]
    fn incremental_overlap_search_resumes_after_a_copied_prefix() {
        let old = b"first line\nsecond line\n";
        let mut old_reader = Cursor::new(old.as_slice());
        let cursor = IncrementalHistoryCursor {
            source_id: digest_parts(&[b"source"]),
            segment_id: digest_parts(&[b"old-segment"]),
            offset: old.len() as u64,
            continuity_hash: incremental_continuity_hash(&mut old_reader, old.len() as u64)
                .unwrap(),
            discarding_oversized_line: false,
        };
        let mut next = old.to_vec();
        next.extend_from_slice(b"third line\n");
        let mut next_reader = Cursor::new(next);
        let next_len = next_reader.get_ref().len() as u64;

        assert_eq!(
            find_incremental_continuity(&mut next_reader, next_len, &cursor).unwrap(),
            Some(old.len() as u64)
        );
    }

    #[test]
    #[ignore = "explicit virtual 1 GiB stress surface"]
    fn generated_1_gib_incremental_v3_recovers_semantics_with_constant_fixture_allocation() {
        const RECORDS: u64 = 225_245;
        const ROW_BYTES: usize = 4_767;
        const SEMANTIC_RECORD: u64 = 220_000;
        let session = history_session("one-gib-session", "2026-09-09T00:00:00Z");
        let generated = GeneratedLongHistoryReader::new(ROW_BYTES, RECORDS, SEMANTIC_RECORD);
        let file_len = generated.len();
        assert!(
            generated.ordinary_row.len() + generated.semantic_row.len() < 16 * 1024,
            "the virtual stress fixture must retain constant allocation"
        );
        assert!((1023 * 1024 * 1024..=1025 * 1024 * 1024).contains(&file_len));
        assert!(
            (RECORDS - SEMANTIC_RECORD) * ROW_BYTES as u64 > REVERSE_MESSAGE_MAX_BYTES,
            "the meaningful turn must sit outside the bounded production tail"
        );
        let mut reader = CountingReader {
            inner: generated,
            bytes_read: 0,
        };

        let mut cursor = 0;
        let mut recovered = None;
        while cursor < file_len {
            let page = read_messages_incremental_from_reader(
                &session,
                &mut reader,
                file_len,
                cursor,
                1024 * 1024 + 1,
                false,
            )
            .unwrap();
            recovered = recovered.or_else(|| {
                page.messages
                    .iter()
                    .find(|message| message.human_prompt)
                    .map(|message| message.text.clone())
            });
            assert!(page.next_offset > cursor);
            cursor = page.next_offset;
        }
        assert_eq!(
            recovered.as_deref(),
            Some("redesign retitle observability before semantic memory")
        );
        assert_eq!(cursor, file_len);
        let page_count = file_len.div_ceil((1024 * 1024 + 1) as u64) as usize;
        assert!(reader.bytes_read <= file_len as usize + page_count * ROW_BYTES * 2);

        let mut cached_reader = CountingReader {
            inner: GeneratedLongHistoryReader::new(ROW_BYTES, RECORDS, SEMANTIC_RECORD),
            bytes_read: 0,
        };
        let continuity = incremental_continuity_hash(&mut cached_reader, file_len).unwrap();
        assert_eq!(
            cached_reader.bytes_read,
            INCREMENTAL_CONTINUITY_BYTES as usize
        );
        let cached = read_messages_incremental_from_reader(
            &session,
            &mut cached_reader,
            file_len,
            file_len,
            1024 * 1024 + 1,
            false,
        )
        .unwrap();
        assert!(cached.messages.is_empty());
        assert_eq!(cached.next_offset, file_len);
        assert_eq!(
            cached_reader.bytes_read,
            INCREMENTAL_CONTINUITY_BYTES as usize
        );
        assert_eq!(continuity.len(), "sha256:".len() + 64);
    }

    #[test]
    fn empty_bounded_reverse_page_keeps_a_cursor_to_older_conversation() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions/2026/08/31");
        fs::create_dir_all(&root).unwrap();
        let mut transcript = String::from(
            "{\"timestamp\":\"2026-08-31T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"abc\",\"cwd\":\"/work/example\",\"source\":\"cli\",\"timestamp\":\"2026-08-31T00:00:00Z\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"older conversation\"}]}}\n",
        );
        for _ in 0..=MAX_MESSAGE_SCAN_LINES {
            transcript.push_str("{\"type\":\"turn_context\"}\n");
        }
        fs::write(root.join("rollout.jsonl"), transcript).unwrap();
        let sources = [HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root: tmp.path().join("sessions"),
        }];
        let history_id = stable_history_id("codex", None, "abc");

        let latest = messages(
            &sources,
            &history_id,
            None,
            50,
            HistoryMessageDirection::Latest,
        )
        .unwrap();
        assert!(latest.messages.is_empty());
        let cursor = latest.older_cursor.as_deref().expect("older cursor");

        let older = messages(
            &sources,
            &history_id,
            Some(cursor),
            50,
            HistoryMessageDirection::Older,
        )
        .unwrap();
        assert_eq!(older.messages.len(), 1);
        assert_eq!(older.messages[0].text, "older conversation");
    }

    #[test]
    fn transcript_messages_are_normalized_and_cursor_paged() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions/2026/08/31");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-08-31T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"abc\",\"cwd\":\"/work/example\",\"source\":\"cli\",\"timestamp\":\"2026-08-31T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-08-31T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"first\"}]}}\n",
                "{\"timestamp\":\"2026-08-31T00:00:02Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"second\"}]}}\n"
            ),
        )
        .unwrap();
        let sources = [HistorySource {
            provider: "codex".into(),
            agent_profile: None,
            root: tmp.path().join("sessions"),
        }];
        let history_id = stable_history_id("codex", None, "abc");
        let first = messages(
            &sources,
            &history_id,
            None,
            1,
            HistoryMessageDirection::Forward,
        )
        .unwrap();
        assert_eq!(first.messages[0].role, "user");
        assert_eq!(first.messages[0].text, "first");
        let second = messages(
            &sources,
            &history_id,
            first.next_cursor.as_deref(),
            1,
            HistoryMessageDirection::Forward,
        )
        .unwrap();
        assert_eq!(second.messages[0].role, "assistant");
        assert_eq!(second.messages[0].text, "second");
        assert_ne!(first.messages[0].id, second.messages[0].id);
        let latest = messages(
            &sources,
            &history_id,
            None,
            2,
            HistoryMessageDirection::Latest,
        )
        .unwrap();
        assert_eq!(first.messages[0].id, latest.messages[0].id);
        assert_eq!(second.messages[0].id, latest.messages[1].id);
    }

    #[test]
    fn catalog_reuses_a_snapshot_until_explicit_invalidation() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sessions/2026/08/31");
        fs::create_dir_all(&root).unwrap();
        let write = |name: &str, id: &str| {
            fs::write(
                root.join(name),
                format!(
                    "{{\"timestamp\":\"2026-08-31T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"/work/example\",\"source\":\"cli\",\"timestamp\":\"2026-08-31T00:00:00Z\"}}}}\n"
                ),
            )
            .unwrap();
        };
        write("one.jsonl", "one");
        let catalog = HistoryCatalog::new(
            vec![HistorySource {
                provider: "codex".into(),
                agent_profile: None,
                root: tmp.path().join("sessions"),
            }],
            tmp.path().join("archives"),
            tmp.path().join("stars"),
        );
        assert_eq!(
            catalog
                .list("test", None, None, None, 50)
                .unwrap()
                .sessions
                .len(),
            1
        );
        write("two.jsonl", "two");
        assert_eq!(
            catalog
                .list("test", None, None, None, 50)
                .unwrap()
                .sessions
                .len(),
            1
        );
        catalog.invalidate();
        assert_eq!(
            catalog
                .list("test", None, None, None, 50)
                .unwrap()
                .sessions
                .len(),
            2
        );
    }

    #[test]
    fn latest_preview_enrichment_respects_the_page_aggregate_byte_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sessions = Vec::new();
        for index in 0..50 {
            let path = tmp.path().join(format!("history-{index}.jsonl"));
            fs::File::create(&path)
                .unwrap()
                .set_len((LATEST_PREVIEW_SESSION_MAX_BYTES * 2) as u64)
                .unwrap();
            let mut session = history_session(&format!("session-{index}"), "2026-09-01T00:00:00Z");
            session.transcript_path = path;
            sessions.push(session);
        }
        let mut cache = HashMap::new();

        enrich_latest_prompt_previews(&mut sessions, &mut cache);

        assert_eq!(cache.len(), sessions.len());
        assert!(
            cache
                .values()
                .map(|entry| entry.scanned_bytes)
                .sum::<usize>()
                <= LATEST_PREVIEW_PAGE_MAX_BYTES
        );
        assert!(
            sessions
                .iter()
                .all(|session| session.last_user_prompt_preview.is_none())
        );
    }

    #[test]
    fn unchanged_negative_preview_cache_avoids_equal_budget_rescans() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sessions = Vec::new();
        for index in 0..50 {
            let path = tmp.path().join(format!("history-{index}.jsonl"));
            fs::File::create(&path)
                .unwrap()
                .set_len((LATEST_PREVIEW_SESSION_MAX_BYTES * 2) as u64)
                .unwrap();
            let mut session = history_session(&format!("session-{index}"), "2026-09-01T00:00:00Z");
            session.transcript_path = path;
            sessions.push(session);
        }
        let mut cache = HashMap::new();
        let mut reads = 0usize;

        enrich_latest_prompt_previews_with(&mut sessions, &mut cache, |_, _| {
            reads += 1;
            None
        });
        assert_eq!(reads, sessions.len());
        enrich_latest_prompt_previews_with(&mut sessions, &mut cache, |_, _| {
            reads += 1;
            None
        });
        assert_eq!(reads, sessions.len());

        enrich_latest_prompt_previews_with(&mut sessions[..1], &mut cache, |_, _| {
            reads += 1;
            None
        });
        assert_eq!(reads, sessions.len() + 1);
        assert_eq!(
            cache[&sessions[0].transcript_path].scanned_bytes,
            LATEST_PREVIEW_SESSION_MAX_BYTES
        );
    }

    #[test]
    fn bounded_line_reader_discards_overflow_before_the_next_record() {
        let mut input = vec![b'x'; MAX_LINE_BYTES + 4096];
        input.extend_from_slice(b"\n{\"ok\":true}\n");
        let mut reader = BufReader::new(input.as_slice());
        let mut line = Vec::new();

        assert_eq!(
            read_bounded_line(&mut reader, &mut line).unwrap(),
            BoundedLine::Oversized
        );
        assert!(line.len() <= MAX_LINE_BYTES);
        assert_eq!(
            read_bounded_line(&mut reader, &mut line).unwrap(),
            BoundedLine::Line
        );
        assert_eq!(line, b"{\"ok\":true}\n");
    }

    #[test]
    fn list_cursor_is_stable_when_a_newer_session_is_inserted() {
        let original = CatalogSnapshot {
            sessions: vec![
                history_session("a", "2026-08-31T03:00:00Z"),
                history_session("b", "2026-08-31T02:00:00Z"),
                history_session("c", "2026-08-31T01:00:00Z"),
            ],
            incremental_segments: BTreeMap::new(),
            truncated: false,
        };
        let first = paginate_snapshot(original, "test", None, None, None, 2).unwrap();
        assert_eq!(
            first
                .sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );

        let refreshed = CatalogSnapshot {
            sessions: vec![
                history_session("new", "2026-08-31T04:00:00Z"),
                history_session("a", "2026-08-31T03:00:00Z"),
                history_session("b", "2026-08-31T02:00:00Z"),
                history_session("c", "2026-08-31T01:00:00Z"),
            ],
            incremental_segments: BTreeMap::new(),
            truncated: false,
        };
        let second = paginate_snapshot(
            refreshed,
            "test",
            None,
            None,
            first.next_cursor.as_deref(),
            2,
        )
        .unwrap();
        assert_eq!(
            second
                .sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c"]
        );
    }

    #[test]
    fn exact_managed_title_is_applied_before_metadata_filtering() {
        let id = stable_history_id("codex", Some("profile-a"), "provider-id");
        let mut snapshot = CatalogSnapshot {
            sessions: vec![HistorySession {
                id: id.clone(),
                provider: "codex".to_string(),
                provider_session_id: "provider-id".to_string(),
                agent_profile: Some("profile-a".to_string()),
                title: None,
                prompt_preview: Some("first prompt".to_string()),
                first_user_prompt_preview: Some("first prompt".to_string()),
                last_user_prompt_preview: Some("latest prompt".to_string()),
                cwd: "/work/example".to_string(),
                repo_name: Some("example".to_string()),
                created_at: "2026-08-31T00:00:00Z".to_string(),
                updated_at: "2026-08-31T00:00:01Z".to_string(),
                archived_at: None,
                starred_at: None,
                resumable: true,
                transcript_path: PathBuf::new(),
                incremental_catalog_stamp: String::new(),
            }],
            incremental_segments: BTreeMap::new(),
            truncated: false,
        };
        apply_title_overrides(
            &mut snapshot.sessions,
            &BTreeMap::from([(id, "Saved Console title".to_string())]),
        );

        let page = paginate_snapshot(
            snapshot,
            "test",
            Some("saved console title"),
            None,
            None,
            10,
        )
        .unwrap();

        assert_eq!(page.sessions.len(), 1);
        assert_eq!(
            page.sessions[0].title.as_deref(),
            Some("Saved Console title")
        );
    }

    #[test]
    fn claude_history_excludes_sidechains_and_pages_visible_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("projects/repo");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("claude-id.jsonl"),
            concat!(
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"user\",\"promptSource\":\"typed\",\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n",
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"assistant\",\"isSidechain\":true,\"message\":{\"content\":\"hidden\"}}\n",
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"second\"}]}}\n"
            ),
        )
        .unwrap();
        let sources = [HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: tmp.path().join("projects"),
        }];

        let page = list(
            &sources,
            HistoryRoots {
                archives: &tmp.path().join("archives"),
                stars: &tmp.path().join("stars"),
            },
            "test",
            Some("first"),
            Some("claude"),
            None,
            10,
        )
        .unwrap();
        assert_eq!(page.sessions.len(), 1);
        let first = messages(
            &sources,
            &page.sessions[0].id,
            None,
            1,
            HistoryMessageDirection::Forward,
        )
        .unwrap();
        assert_eq!(first.messages[0].text, "first");
        let second = messages(
            &sources,
            &page.sessions[0].id,
            first.next_cursor.as_deref(),
            10,
            HistoryMessageDirection::Forward,
        )
        .unwrap();
        assert_eq!(second.messages.len(), 1);
        assert_eq!(second.messages[0].text, "second");
    }

    #[test]
    fn claude_history_excludes_generated_user_records_and_tool_result_parts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("projects/repo");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("claude-id.jsonl"),
            concat!(
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"user\",\"promptSource\":\"typed\",\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n",
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"tool\",\"content\":\"tool output\"}]}}\n",
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"user\",\"isMeta\":true,\"message\":{\"role\":\"user\",\"content\":\"meta context\"}}\n",
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"user\",\"promptSource\":\"system\",\"message\":{\"role\":\"user\",\"content\":\"system context\"}}\n",
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"user\",\"isCompactSummary\":true,\"message\":{\"role\":\"user\",\"content\":\"compact summary\"}}\n",
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"user\",\"isVisibleInTranscriptOnly\":true,\"message\":{\"role\":\"user\",\"content\":\"transcript-only context\"}}\n",
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"user\",\"promptSource\":\"typed\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"second\"},{\"type\":\"tool_result\",\"tool_use_id\":\"tool\",\"content\":\"hidden tool output\"}]}}\n",
                "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"answer\"}]}}\n"
            ),
        )
        .unwrap();
        let sources = [HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: tmp.path().join("projects"),
        }];

        let page = list(
            &sources,
            HistoryRoots {
                archives: &tmp.path().join("archives"),
                stars: &tmp.path().join("stars"),
            },
            "test",
            Some("first"),
            Some("claude"),
            None,
            10,
        )
        .unwrap();
        let result = messages(
            &sources,
            &page.sessions[0].id,
            None,
            20,
            HistoryMessageDirection::Forward,
        )
        .unwrap();

        assert_eq!(
            result
                .messages
                .iter()
                .map(|message| (message.role.as_str(), message.text.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("user", "first"),
                ("user", "second"),
                ("assistant", "answer")
            ]
        );
    }

    #[test]
    fn archives_do_not_cross_profile_identity_boundaries() {
        let tmp = tempfile::tempdir().unwrap();
        let profile_a_root = tmp.path().join("profile-a/2026/08/31");
        let profile_b_root = tmp.path().join("profile-b/2026/08/31");
        fs::create_dir_all(&profile_a_root).unwrap();
        fs::create_dir_all(&profile_b_root).unwrap();
        fs::write(
            profile_a_root.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-08-31T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"shared-id\",\"cwd\":\"/work/profile-a\",\"source\":\"cli\",\"timestamp\":\"2026-08-31T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-08-31T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"profile a transcript\"}]}}\n"
            ),
        )
        .unwrap();
        fs::write(
            profile_b_root.join("rollout.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-08-31T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"shared-id\",\"cwd\":\"/work/profile-b\",\"source\":\"cli\",\"timestamp\":\"2026-08-31T00:00:00Z\"}}\n",
                "{\"timestamp\":\"2026-08-31T00:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"profile b transcript\"}]}}\n"
            ),
        )
        .unwrap();
        let sources = [
            HistorySource {
                provider: "codex".into(),
                agent_profile: Some("profile-a".into()),
                root: tmp.path().join("profile-a"),
            },
            HistorySource {
                provider: "codex".into(),
                agent_profile: Some("profile-b".into()),
                root: tmp.path().join("profile-b"),
            },
        ];
        let archives = tmp.path().join("archives");
        let stars_root = tmp.path().join("stars");
        let profile_a_id = stable_history_id("codex", Some("profile-a"), "shared-id");
        let profile_b_id = stable_history_id("codex", Some("profile-b"), "shared-id");
        write_archive(
            &archives,
            &ArchivedSession {
                schema_version: "agent-session.history-archive.v1".into(),
                history_id: profile_a_id.clone(),
                provider: "codex".into(),
                provider_session_id: "shared-id".into(),
                agent_profile: Some("profile-a".into()),
                title: Some("Archived profile A".into()),
                cwd: "/work/profile-a".into(),
                created_at: "2026-08-31T00:00:00Z".into(),
                updated_at: "2026-08-31T00:00:01Z".into(),
                archived_at: "2026-08-31T00:00:02Z".into(),
            },
        )
        .unwrap()
        .commit();

        let page = list(
            &sources,
            HistoryRoots {
                archives: &archives,
                stars: &stars_root,
            },
            "test",
            None,
            None,
            None,
            10,
        )
        .unwrap();
        assert_eq!(page.sessions.len(), 2);
        let profile_a = page
            .sessions
            .iter()
            .find(|session| session.id == profile_a_id)
            .unwrap();
        assert_eq!(profile_a.title.as_deref(), Some("Archived profile A"));
        assert!(profile_a.archived_at.is_some());
        let profile_b = page
            .sessions
            .iter()
            .find(|session| session.id == profile_b_id)
            .unwrap();
        assert_eq!(profile_b.agent_profile.as_deref(), Some("profile-b"));
        assert_eq!(profile_b.cwd, "/work/profile-b");
        assert!(profile_b.archived_at.is_none());
        assert_eq!(
            messages(
                &sources,
                &profile_b_id,
                None,
                10,
                HistoryMessageDirection::Forward,
            )
            .unwrap()
            .messages[0]
                .text,
            "profile b transcript"
        );
    }

    #[test]
    fn archive_without_provider_transcript_is_not_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let archives = tmp.path().join("archives");
        let stars_root = tmp.path().join("stars");
        write_archive(
            &archives,
            &ArchivedSession {
                schema_version: "agent-session.history-archive.v1".into(),
                history_id: stable_history_id("codex", None, "missing"),
                provider: "codex".into(),
                provider_session_id: "missing".into(),
                agent_profile: None,
                title: Some("Orphaned archive".into()),
                cwd: "/work/example".into(),
                created_at: "2026-08-31T00:00:00Z".into(),
                updated_at: "2026-08-31T00:00:01Z".into(),
                archived_at: "2026-08-31T00:00:02Z".into(),
            },
        )
        .unwrap()
        .commit();

        let page = list(
            &[],
            HistoryRoots {
                archives: &archives,
                stars: &stars_root,
            },
            "test",
            None,
            None,
            None,
            10,
        )
        .unwrap();

        assert!(page.sessions.is_empty());
    }

    #[test]
    fn failed_rearchive_restores_previously_committed_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("archives");
        let mut archive = ArchivedSession {
            schema_version: "agent-session.history-archive.v1".to_string(),
            history_id: stable_history_id("codex", None, "abc"),
            provider: "codex".to_string(),
            provider_session_id: "abc".to_string(),
            agent_profile: None,
            title: Some("original".to_string()),
            cwd: "/work/example".to_string(),
            created_at: "2026-08-31T00:00:00Z".to_string(),
            updated_at: "2026-08-31T00:00:01Z".to_string(),
            archived_at: "2026-08-31T00:00:02Z".to_string(),
        };
        write_archive(&root, &archive).unwrap().commit();
        let path = root.join(format!("{}.json", archive.history_id));
        let original = fs::read(&path).unwrap();

        archive.title = Some("replacement".to_string());
        let pending = write_archive(&root, &archive).unwrap();
        drop(pending);

        assert_eq!(fs::read(path).unwrap(), original);
    }
}
