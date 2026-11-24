//! Cache only validated, committed registry generations.
use super::*;
use crate::send::claude_notice::store::FileStamp;
use std::sync::Mutex;

static CURRENT: Mutex<Option<(FileStamp, Arc<Registry>)>> = Mutex::new(None);

/// Atomic commits publish complete inodes. A snapshot can read one of them
/// without joining the exclusive registry writer queue.
pub(super) fn snapshot(context: &CliContext) -> Result<Arc<Registry>, CliError> {
    let path = heartbeat_state::observational_root(context)?.join(REGISTRY_FILE);
    load(&path)?.ok_or_else(store_unavailable)
}

pub(super) fn load(path: &Path) -> Result<Option<Arc<Registry>>, CliError> {
    // Share one parse when a new generation is first seen by concurrent polls.
    // Every reader still opens and validates the private file it was asked for.
    let mut current = CURRENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut file, metadata) = match open_private_file(path, MAX_REGISTRY_BYTES) {
        Ok(opened) => opened,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(store_unavailable()),
    };
    let stamp = FileStamp::from_metadata(&metadata).ok_or_else(store_unavailable)?;
    if let Some((cached, registry)) = current.as_ref()
        && *cached == stamp
    {
        return Ok(Some(Arc::clone(registry)));
    }

    let bytes = read_opened_private_file(&mut file, metadata.len(), MAX_REGISTRY_BYTES)
        .map_err(|_| store_unavailable())?;
    let registry: Registry = serde_json::from_slice(&bytes).map_err(|_| store_corrupt())?;
    if !delegation::registry_version_supported(&registry.schema_version) {
        return Err(store_corrupt());
    }
    let registry = Arc::new(registry);
    // An atomic replacement during this read can unlink the opened inode.
    // Its bytes can serve this request, while only a stable stamp is reusable.
    if file
        .metadata()
        .ok()
        .and_then(|metadata| FileStamp::from_metadata(&metadata))
        .as_ref()
        == Some(&stamp)
    {
        *current = Some((stamp, Arc::clone(&registry)));
    }
    Ok(Some(registry))
}
