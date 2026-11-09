//! Runtime-owned composer residue, fenced to the incarnation that typed it.
use super::*;

#[derive(Serialize, Deserialize)]
struct Injected {
    incarnation: String,
    text: String,
}

/// Called under the record lock after a successful notice paste. This is
/// private ownership evidence, never a saved draft or public projection.
pub(crate) fn remember(
    context: &CliContext,
    record: &SessionRecord,
    text: &str,
) -> Result<(), CliError> {
    let Some(runtime) = &record.runtime else {
        return Ok(());
    };
    if text.len() > MAX_TEXT {
        return Err(unavailable());
    }
    let root = root(context, record);
    private_dir(&root)?;
    persist(
        &root.join("injected.json"),
        &Injected {
            incarnation: runtime.launch_id.clone(),
            text: text.to_owned(),
        },
    )
}

pub(super) fn matches(
    context: &CliContext,
    record: &SessionRecord,
    text: &str,
) -> Result<bool, CliError> {
    if fallback(text) {
        return Ok(true);
    }
    let path = root(context, record).join("injected.json");
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(unavailable()),
        Ok(_) => {}
    }
    let injected: Injected = read_json(&path, MAX_RECORD)?;
    Ok(!text.is_empty()
        && record
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.launch_id == injected.incarnation)
        && injected.text.starts_with(text))
}

/// The fallback table is documented once in session-notices-v1.md.
fn fallback(text: &str) -> bool {
    let first = text.split('\n').next().unwrap_or("");
    (first.starts_with("<!-- agent-notice:v1 ") && first.ends_with(" -->"))
        || crate::notice::labels::NOTICE_LABELS.iter().any(|entry| {
            [entry.label, entry.stored_title].into_iter().any(|title| {
                first
                    .strip_prefix(title)
                    .is_some_and(|rest| rest.starts_with(": "))
            })
        })
}
