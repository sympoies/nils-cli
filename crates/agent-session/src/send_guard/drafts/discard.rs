//! Remove one saved draft while the caller holds the session record lock.
use super::*;

pub(crate) fn discard(
    context: &CliContext,
    record: &SessionRecord,
    id: &str,
) -> Result<Value, CliError> {
    let root = root(context, record);
    let path = saved_path(&root, id)?;
    let mut drafts = list(context, record)?;
    let index = drafts
        .iter()
        .position(|draft| draft.draft_id == id)
        .ok_or_else(|| CliError::runtime("draft-not-found", "saved draft was not found", None))?;
    drafts.remove(index);
    fs::remove_file(path).map_err(|_| unavailable())?;
    fs::File::open(root.join("saved"))
        .and_then(|dir| dir.sync_all())
        .map_err(|_| unavailable())?;
    if let Some(newest) = drafts.last() {
        persist(&root.join("latest.json"), &newest.metadata(&record.id))?;
    } else {
        match fs::remove_file(root.join("latest.json")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unavailable()),
        }
        fs::File::open(&root)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| unavailable())?;
    }
    Ok(
        json!({"discarded":id, "remaining":drafts.len(), "stale_draft":latest(context, &record.id)?}),
    )
}
