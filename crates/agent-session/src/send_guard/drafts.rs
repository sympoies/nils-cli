//! Exact stale draft preservation. Callers hold the session record lock.
use super::{Admission, GuardArgs};
use crate::{CliContext, CliError, SessionRecord, session_dir};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Instant;
use std::{
    fs,
    io::Read,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

pub(crate) const BOUND_ENV: &str = "AGENT_SESSION_STALE_DRAFT_SECONDS";
pub(crate) const DEFAULT_BOUND: u64 = 600;
const MAX_TEXT: usize = 64 * 1024;
const MAX_RECORD: u64 = (MAX_TEXT * 6 + 2048) as u64;
const MAX_STORE: u64 = 8 * 1024 * 1024;
const MAX_COUNT: usize = 128;

fn unavailable() -> CliError {
    CliError::runtime(
        "draft-store-unavailable",
        "saved draft store is unavailable or full",
        None,
    )
}

pub(crate) fn parse_bound(raw: &str) -> Result<u64, CliError> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(CliError::usage(
            "invalid-stale-draft-bound",
            "stale draft bound must be unsigned integer seconds",
            None,
        ));
    }
    raw.parse().map_err(|_| {
        CliError::usage(
            "invalid-stale-draft-bound",
            "stale draft bound must fit unsigned integer seconds",
            None,
        )
    })
}

pub(crate) fn bound() -> Result<u64, CliError> {
    match std::env::var(BOUND_ENV) {
        Ok(raw) => parse_bound(&raw),
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_BOUND),
        Err(_) => Err(CliError::usage(
            "invalid-stale-draft-bound",
            "stale draft bound must be unsigned integer seconds",
            None,
        )),
    }
}

fn root(context: &CliContext, record: &SessionRecord) -> PathBuf {
    session_dir(context, &record.id).join("drafts")
}

fn private_dir(path: &Path) -> Result<(), CliError> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
        Ok(_) => return Err(unavailable()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|_| unavailable())?
        }
        Err(_) => return Err(unavailable()),
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| unavailable())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T, CliError> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| unavailable())?;
    if !file.metadata().map_err(|_| unavailable())?.is_file() {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() as u64 > limit {
        return Err(unavailable());
    }
    serde_json::from_slice(&bytes).map_err(|_| unavailable())
}

fn persist(path: &Path, value: &impl Serialize) -> Result<(), CliError> {
    let bytes = serde_json::to_vec(value).map_err(|_| unavailable())?;
    crate::write_private_file(path, &bytes).map_err(|_| unavailable())?;
    // A saved draft must survive a crash before any deletion keys are issued.
    fs::File::open(path.parent().ok_or_else(unavailable)?)
        .and_then(|dir| dir.sync_all())
        .map_err(|_| unavailable())
}

#[derive(Serialize, Deserialize)]
struct Observation {
    fingerprint: String,
    unchanged_since: i64,
}

/// Observe only exact reader output. Unknown, empty, disabled, edited and
/// replaced-runtime input cannot inherit a stale timer.
pub(crate) fn observe_at(
    context: &CliContext,
    record: &SessionRecord,
    input: Option<&str>,
    bound: u64,
    now: i64,
) -> Result<bool, CliError> {
    let root = root(context, record);
    let path = root.join("observation.json");
    let Some(input) = input.filter(|text| !text.is_empty() && text.len() <= MAX_TEXT && bound != 0)
    else {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unavailable()),
        }
        return Ok(false);
    };
    let identity = json!([
        record.runtime.as_ref().map(|r| &r.launch_id),
        record
            .provider_resume
            .as_ref()
            .map(|r| (&r.provider, &r.session_id)),
        input
    ]);
    let fingerprint = Sha256::digest(serde_json::to_vec(&identity).map_err(|_| unavailable())?)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    private_dir(&root)?;
    let previous = match fs::symlink_metadata(&path) {
        Ok(_) => Some(read_json::<Observation>(&path, 1024)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(unavailable()),
    };
    if let Some(previous) = previous
        && previous.fingerprint == fingerprint
        && previous.unchanged_since <= now
    {
        return Ok((now.saturating_sub(previous.unchanged_since) as u64) > bound);
    }
    persist(
        &path,
        &Observation {
            fingerprint,
            unchanged_since: now,
        },
    )?;
    Ok(false)
}

pub(crate) fn observe(
    context: &CliContext,
    record: &SessionRecord,
    input: Option<&str>,
) -> Result<bool, CliError> {
    observe_at(
        context,
        record,
        input,
        bound()?,
        jiff::Timestamp::now().as_second(),
    )
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SavedDraft {
    schema_version: String,
    event: String,
    draft_id: String,
    timestamp: String,
    session_incarnation: Option<String>,
    text: String,
    cleared: bool,
}
impl SavedDraft {
    pub(crate) fn metadata(&self) -> Value {
        json!({"event":self.event, "draft_id":self.draft_id, "timestamp":self.timestamp,
            "session_incarnation":self.session_incarnation, "bytes":self.text.len(), "cleared":self.cleared})
    }
    pub(crate) fn text(self) -> String {
        self.text
    }
}

fn saved_path(root: &Path, id: &str) -> Result<PathBuf, CliError> {
    if uuid::Uuid::parse_str(id)
        .ok()
        .is_none_or(|uuid| uuid.to_string() != id)
    {
        return Err(CliError::usage(
            "invalid-draft-id",
            "draft id must be a canonical UUID",
            None,
        ));
    }
    Ok(root.join("saved").join(format!("{id}.json")))
}

pub(crate) fn list(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<Vec<SavedDraft>, CliError> {
    let dir = root(context, record).join("saved");
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(unavailable()),
    };
    let mut drafts = Vec::new();
    for entry in entries {
        if drafts.len() >= MAX_COUNT {
            return Err(unavailable());
        }
        let path = entry.map_err(|_| unavailable())?.path();
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(unavailable)?;
        drafts.push(read(context, record, id)?);
    }
    drafts.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then(a.draft_id.cmp(&b.draft_id))
    });
    Ok(drafts)
}

pub(crate) fn read(
    context: &CliContext,
    record: &SessionRecord,
    id: &str,
) -> Result<SavedDraft, CliError> {
    let draft: SavedDraft = read_json(&saved_path(&root(context, record), id)?, MAX_RECORD)?;
    if draft.schema_version != "agent-session.saved-draft.v1"
        || draft.draft_id != id
        || draft.text.len() > MAX_TEXT
    {
        return Err(unavailable());
    }
    Ok(draft)
}

/// The caller has reached the existing mutation admission boundary. Saving
/// never grants admission; cleanup must still prove an empty composer.
pub(crate) fn clear(admission: &Admission<'_>, text: &str) -> Result<bool, CliError> {
    let drafts = list(admission.context, admission.record)?;
    let root = root(admission.context, admission.record);
    private_dir(&root)?;
    private_dir(&root.join("saved"))?;
    let mut draft = SavedDraft {
        schema_version: "agent-session.saved-draft.v1".into(),
        event: "stale_draft_stashed".into(),
        draft_id: uuid::Uuid::new_v4().to_string(),
        timestamp: jiff::Timestamp::now().to_string(),
        session_incarnation: admission
            .record
            .runtime
            .as_ref()
            .map(|r| r.launch_id.clone()),
        text: text.to_owned(),
        cleared: false,
    };
    let used = drafts
        .iter()
        .try_fold(0u64, |total, d| {
            serde_json::to_vec(d).map(|b| total + b.len() as u64)
        })
        .map_err(|_| unavailable())?;
    let size = serde_json::to_vec(&draft).map_err(|_| unavailable())?.len() as u64;
    if text.len() > MAX_TEXT || drafts.len() >= MAX_COUNT || used + size > MAX_STORE {
        return Err(unavailable());
    }
    let path = saved_path(&root, &draft.draft_id)?;
    persist(&path, &draft)?;
    persist(&root.join("latest.json"), &draft.metadata())?;
    if !admission.clear_saved_draft(text) {
        return Ok(false);
    }
    draft.cleared = true;
    persist(&path, &draft)?;
    persist(&root.join("latest.json"), &draft.metadata())?;
    observe_at(
        admission.context,
        admission.record,
        Some(""),
        0,
        jiff::Timestamp::now().as_second(),
    )?;
    Ok(true)
}

pub(crate) fn clear_observed(
    context: &CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
    text: &str,
    deadline: Instant,
) -> Result<bool, CliError> {
    let args = GuardArgs::default();
    clear(
        &Admission {
            context,
            record,
            args: &args,
            tmux_bin,
            verify_composer: false,
            deadline,
        },
        text,
    )
}

/// Called only after the mailbox's existing quiescence fence and attachment,
/// identity and activity admission, immediately before its paste.
pub(crate) fn clear_pending(
    context: &CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
    deadline: Instant,
) -> Result<(), CliError> {
    let target = format!("{}:0.0", record.tmux_session);
    let input = super::capture_styled(tmux_bin, &target, crate::PANE_OBSERVATION_COMMAND_TIMEOUT)
        .and_then(|pane| crate::send_submit::exact_composer_input(&record.agent, &pane));
    let stale = observe(context, record, input.as_deref())?;
    if input.as_deref() == Some("") {
        return Ok(());
    }
    if let Some(input) = input
        && stale
        && clear_observed(context, record, tmux_bin, &input, deadline)?
    {
        return Ok(());
    }
    Err(CliError::data(
        "composer-not-empty",
        "composer does not permit guarded input",
        None,
    ))
}

pub(crate) fn latest(context: &CliContext, id: &str) -> Result<Option<Value>, CliError> {
    crate::validate_id(id)?;
    let path = session_dir(context, id).join("drafts/latest.json");
    if !path.exists() {
        return Ok(None);
    }
    let value: Value = read_json(&path, 2048)?;
    // Board projections never copy arbitrary fields from private storage.
    Ok(Some(
        json!({"event":"stale_draft_stashed", "draft_id":value["draft_id"],
        "timestamp":value["timestamp"], "bytes":value["bytes"], "cleared":value["cleared"]}),
    ))
}
