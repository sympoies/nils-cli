//! Admission shared by direct structured replies. The caller holds the record lock.
use super::{capture_styled, send_submit};
use crate::{CliError, PANE_OBSERVATION_COMMAND_TIMEOUT, SessionRecord};
use serde_json::json;
use std::path::Path;

pub(crate) fn require_empty_composer(
    record: &SessionRecord,
    tmux_bin: &Path,
) -> Result<(), CliError> {
    let target = format!("{}:0.0", record.tmux_session);
    let input = capture_styled(tmux_bin, &target, PANE_OBSERVATION_COMMAND_TIMEOUT)
        .and_then(|pane| send_submit::direct_composer_input(&record.agent, &pane));
    if input.as_deref() == Some("") {
        return Ok(());
    }
    let (code, message) = if input.is_some() {
        ("composer-not-empty", "composer contains an owner draft")
    } else {
        (
            "composer-unobservable",
            "composer content cannot be observed exactly",
        )
    };
    Err(CliError::data(
        code,
        message,
        Some(json!({
            "id": record.id, "stage": "admission", "pasted": false, "submitted": false,
            "atomic": false,
        })),
    ))
}
