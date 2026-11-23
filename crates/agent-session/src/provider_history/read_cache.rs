//! Polling reuses verified projections for unchanged transcript and notice data.
use super::*;
use crate::send::claude_notice::store::{FileStamp, LiveCache, StoreStamp};
use history_images::native_notices::Bindings;

const LATEST_PAGE_CACHE_ENTRIES: usize = 16;
const LATEST_PAGE_CACHE_BYTES: usize = 256 * 1024;

#[derive(Debug, Default)]
pub(in crate::provider_history) struct HistoryReadCache {
    pub(in crate::provider_history) live_notices: LiveCache,
    previews: Mutex<HashMap<PathBuf, CachedPreview>>,
    latest: Mutex<HashMap<(String, usize, bool), CachedPage>>,
}

#[derive(Debug, Eq, PartialEq)]
struct ProjectionStamp {
    transcript: FileStamp,
    store: Option<StoreStamp>,
    launch: Option<(u64, String)>,
}

impl ProjectionStamp {
    fn read(session: &HistorySession, notices: &Bindings) -> Option<Self> {
        Some(Self {
            transcript: FileStamp::from_metadata(&fs::metadata(&session.transcript_path).ok()?)?,
            store: notices.store_stamp().cloned(),
            launch: notices
                .launch_binding()
                .map(|(offset, _, hash)| (*offset, hash.clone())),
        })
    }

    fn transcript_still_matches(&self, path: &Path) -> bool {
        fs::metadata(path)
            .ok()
            .and_then(|metadata| FileStamp::from_metadata(&metadata))
            .is_some_and(|stamp| stamp == self.transcript)
    }
}

#[derive(Clone, Debug)]
pub(super) struct CutoffPreviews {
    pub(super) first: Option<String>,
    pub(super) last_override: Option<Option<String>>,
}

#[derive(Debug)]
struct CachedPreview {
    stamp: ProjectionStamp,
    history_id: String,
    budget: usize,
    previews: CutoffPreviews,
}

#[derive(Debug)]
struct CachedPage {
    stamp: ProjectionStamp,
    path: PathBuf,
    provider_version: Option<String>,
    page: HistoryMessagesPage,
}

impl HistoryReadCache {
    pub(super) fn cutoff_previews(
        &self,
        session: &HistorySession,
        notices: &Bindings,
        budget: usize,
        deadline: Instant,
        scan: impl FnOnce() -> (CutoffPreviews, bool),
    ) -> CutoffPreviews {
        let stamp = ProjectionStamp::read(session, notices);
        if let Some(current) = &stamp
            && let Some(entry) = self
                .previews
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&session.transcript_path)
                .filter(|entry| {
                    entry.stamp == *current
                        && entry.history_id == session.id
                        && entry.budget == budget
                })
        {
            return entry.previews.clone();
        }

        let (previews, complete) = scan();
        if let Some(stamp) = stamp
            && complete
            && Instant::now() < deadline
            && notices.check().is_ok()
            && stamp.transcript_still_matches(&session.transcript_path)
        {
            let mut cache = self
                .previews
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if cache.len() >= LATEST_PREVIEW_CACHE_MAX_ENTRIES {
                cache.clear();
            }
            cache.insert(
                session.transcript_path.clone(),
                CachedPreview {
                    stamp,
                    history_id: session.id.clone(),
                    budget,
                    previews: previews.clone(),
                },
            );
        }
        previews
    }

    pub(in crate::provider_history) fn latest(
        &self,
        session: &HistorySession,
        notices: &Bindings,
        limit: usize,
        include_parts: bool,
        deadline: Instant,
    ) -> Result<HistoryMessagesPage, HistoryError> {
        let stamp = ProjectionStamp::read(session, notices);
        let key = (session.id.clone(), limit, include_parts);
        if let Some(current) = &stamp
            && let Some(entry) = self
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&key)
                .filter(|entry| {
                    entry.stamp == *current
                        && entry.path == session.transcript_path
                        && entry.provider_version == session.provider_version
                })
        {
            return Ok(entry.page.clone());
        }

        let page = history_images::read_messages_reverse(
            session,
            None,
            limit,
            include_parts,
            notices,
            deadline,
        )?;
        notices.check()?;
        if let Some(stamp) = stamp
            && Instant::now() < deadline
            && stamp.transcript_still_matches(&session.transcript_path)
            && serde_json::to_writer(PageBudget(LATEST_PAGE_CACHE_BYTES), &page).is_ok()
        {
            let mut cache = self
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if cache.len() >= LATEST_PAGE_CACHE_ENTRIES {
                cache.clear();
            }
            cache.insert(
                key,
                CachedPage {
                    stamp,
                    path: session.transcript_path.clone(),
                    provider_version: session.provider_version.clone(),
                    page: page.clone(),
                },
            );
        }
        Ok(page)
    }
}

struct PageBudget(usize);

impl std::io::Write for PageBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or(std::io::ErrorKind::FileTooLarge)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
