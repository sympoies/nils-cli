//! Canonical display metadata. Descriptive only; never changes lineage or authority.
use crate::{
    CliContext, CliError, SessionRecord, lock_exact_session_authority, write_session_record,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TitleMode {
    #[default]
    Auto,
    Pinned,
}

pub(crate) fn title_mode(record: &SessionRecord) -> TitleMode {
    record
        .extra
        .get("title_mode")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

pub(crate) fn revision(record: &SessionRecord) -> u64 {
    record
        .extra
        .get("display_revision")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

pub(crate) fn require_auto(record: &SessionRecord) -> Result<(), CliError> {
    if title_mode(record) == TitleMode::Pinned {
        return Err(CliError::data(
            "title-mode-pinned",
            "automatic retitle is disabled for this session",
            None,
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Update {
    pub(crate) expected_session_created_at: String,
    pub(crate) expected_revision: u64,
    #[serde(default)]
    pub(crate) role: Option<String>,
    pub(crate) title_mode: Option<TitleMode>,
}

pub(crate) fn update(
    context: &CliContext,
    id: &str,
    request: Update,
) -> Result<SessionRecord, CliError> {
    if request.role.is_none() && request.title_mode.is_none() {
        return Err(CliError::usage(
            "display-metadata-invalid",
            "supply role or title_mode",
            None,
        ));
    }
    let role = crate::lineage::role_from_request(request.role.as_deref())?;
    let mut authority = lock_exact_session_authority(context, id)?
        .ok_or_else(|| CliError::data("session-not-found", "session does not exist", None))?;
    let record = &mut authority.record;
    if record.created_at != request.expected_session_created_at
        || revision(record) != request.expected_revision
    {
        return Err(CliError::data(
            "display-revision-conflict",
            "session display metadata changed; refresh and retry",
            None,
        ));
    }
    if let Some(ref role) = role {
        crate::lineage::require_root_for_role(
            Some(role),
            record
                .lineage
                .as_ref()
                .is_some_and(|lineage| lineage.parent.is_some()),
        )?;
    }
    if role
        .as_ref()
        .is_none_or(|role| record.role.as_ref() == Some(role))
        && request
            .title_mode
            .is_none_or(|mode| title_mode(record) == mode)
    {
        return Ok(record.clone());
    }
    let next = revision(record).checked_add(1).ok_or_else(|| {
        CliError::data(
            "display-revision-overflow",
            "display revision overflow",
            None,
        )
    })?;
    if let Some(role) = role {
        record.role = Some(role);
    }
    if let Some(mode) = request.title_mode {
        record.extra.insert("title_mode".into(), json!(mode));
        // Invalidate every in-flight retitle fence even if auto is re-enabled.
        record.title_revision = record.title_revision.checked_add(1).ok_or_else(|| {
            CliError::data("title-revision-overflow", "title revision overflow", None)
        })?;
    }
    record.extra.insert("display_revision".into(), json!(next));
    record.updated_at = jiff::Zoned::now().timestamp().to_string();
    write_session_record(context, record)?;
    Ok(record.clone())
}
