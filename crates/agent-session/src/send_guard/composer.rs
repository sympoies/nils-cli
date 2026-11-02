//! Admission shared by direct structured replies. The caller holds the record lock.
use super::{capture_styled, send_submit};
use crate::{CliError, PANE_OBSERVATION_COMMAND_TIMEOUT, SessionRecord};
use send_submit::Unobserved;
use serde_json::json;
use std::path::Path;

pub(crate) fn require_empty_composer(
    record: &SessionRecord,
    tmux_bin: &Path,
) -> Result<(), CliError> {
    let target = format!("{}:0.0", record.tmux_session);
    let input = capture_styled(tmux_bin, &target, PANE_OBSERVATION_COMMAND_TIMEOUT)
        .ok_or(Unobserved::CaptureFailed)
        .and_then(|pane| send_submit::direct_composer_observation(&record.agent, &pane));
    let unobserved = match input.as_deref() {
        Ok("") => return Ok(()),
        Ok(_) => None,
        Err(reason) => Some(*reason),
    };
    let (code, message) = if unobserved.is_none() {
        ("composer-not-empty", "composer contains an owner draft")
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
