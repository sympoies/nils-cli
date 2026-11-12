//! Offline diagnostics reuse the same eligible shared projection as live fallback.
use super::*;

pub(super) fn read(target: &Path, name: &str) -> Result<cache::CacheRead, String> {
    observation::import_shared_cache(target, name);
    cache::read_for_cached_mode(name, now_epoch())
}
