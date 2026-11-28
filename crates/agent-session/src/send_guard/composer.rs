//! Admission shared by direct structured replies. The caller holds the record lock.
use super::{capture_styled, send_submit};
use crate::{CliError, PANE_OBSERVATION_COMMAND_TIMEOUT, SessionRecord};
use send_submit::Unobserved;
use serde_json::json;
use std::path::Path;

pub(crate) fn require_empty_composer(
    context: &crate::CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
) -> Result<(), CliError> {
    require_empty_composer_within(context, record, tmux_bin, crate::send_submit::HOLD_BUDGET)
}

/// `hold` bounds only the stale-draft cleanup; the production budget is the
/// default above. Tests whose fixture spawns processes pass a wider one so a
/// slow host cannot turn cleanup into a refusal.
pub(crate) fn require_empty_composer_within(
    context: &crate::CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
    hold: std::time::Duration,
) -> Result<(), CliError> {
    let target = format!("{}:0.0", record.tmux_session);
    let input = capture_styled(tmux_bin, &target, PANE_OBSERVATION_COMMAND_TIMEOUT)
        .ok_or(Unobserved::CaptureFailed)
        .and_then(|pane| send_submit::direct_composer_observation(&record.agent, &pane));
    let stale = super::drafts::observe(context, record, input.as_deref().ok())?;
    if let Ok(text) = &input
        && !text.is_empty()
        && stale
        && super::drafts::clear_observed(
            context,
            record,
            tmux_bin,
            text,
            std::time::Instant::now() + hold,
        )?
    {
        return Ok(());
    }
    let unobserved = match input.as_deref() {
        Ok("") => return Ok(()),
        Ok(_) => None,
        Err(reason) => Some(*reason),
    };
    let (code, message) = if unobserved.is_none() {
        ("composer-not-empty", "composer contains an owner draft")
    } else if record.agent == "claude" && unobserved == Some(Unobserved::RegionNotSingleRow) {
        (
            "composer-unobservable",
            "multi-row composer cannot be read exactly; send or clear the draft first",
        )
    } else {
        (
            "composer-unobservable",
            "composer content cannot be observed exactly",
        )
    };
    let mut details = json!({
        "id": record.id, "stage": "admission", "pasted": false, "submitted": false,
        "atomic": false,
    });
    if let (Some(reason), Some(map)) = (unobserved, details.as_object_mut()) {
        map.insert("observation".to_string(), json!(reason.as_str()));
    }
    Err(CliError::data(code, message, Some(details)))
}
