//! Notice metadata on an already encoded Console create request.
use super::{NoticeEnvelope, StartNoticeArgs};
use crate::{CliError, SessionRecord};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LaunchNotice {
    Written,
    NotRequested,
    Unavailable,
}

/// Called only after release_start succeeded, including the private receipt write.
pub(crate) fn project(
    notice: &StartNoticeArgs,
    record: &SessionRecord,
    view: &mut crate::SessionView,
) {
    view.launch_notice = Some(if notice.notice_kind.is_none() {
        LaunchNotice::NotRequested
    } else if record.provider_resume.as_ref().is_some_and(|provider| {
        provider.provider == "claude" && provider.capture_method == "claude-explicit-session-id"
    }) {
        LaunchNotice::Written
    } else {
        LaunchNotice::Unavailable
    });
}

/// The origin encoded the prompt once; the target checks its metadata without re-encoding it.
pub(crate) fn validate_prepared(
    notice: &StartNoticeArgs,
    prompt: Option<&str>,
) -> Result<(), CliError> {
    let Some(kind) = notice.notice_kind.as_deref() else {
        return if notice.notice_source.is_none() && notice.notice_id.is_none() {
            Ok(())
        } else {
            Err(super::error(super::NoticeError::InvalidField("kind")))
        };
    };
    let metadata = NoticeEnvelope::new(
        kind,
        notice.notice_source.as_deref().unwrap_or(""),
        notice.notice_id.as_deref(),
    )
    .map_err(super::error)?;
    super::require_enabled()?;
    if super::parse_notice(prompt.unwrap_or("")).is_none_or(|parsed| parsed.metadata != metadata) {
        return Err(CliError::usage(
            "launch-notice-invalid",
            "initial notice metadata does not match the prompt",
            None,
        ));
    }
    Ok(())
}
