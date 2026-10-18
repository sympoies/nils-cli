//! Preserve terminal cleanup observations in structured prompt outcomes.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct ComposerCleanup {
    cleared: bool,
    remaining_input: String,
}

impl ComposerCleanup {
    pub(super) fn from_details(details: &Value) -> Option<Self> {
        let cleared = details["cleared"].as_bool()?;
        let remaining = details["remaining_input"].as_str()?;
        matches!(
            remaining,
            "empty" | "pasted_text" | "different_text" | "unobservable"
        )
        .then(|| Self {
            cleared,
            remaining_input: remaining.to_string(),
        })
    }
}

pub(super) fn failure(error: CliError) -> Response {
    let details = error.details().cloned().unwrap_or(Value::Null);
    let before_delivery = details["stage"] == "admission"
        && details["pasted"] == false
        && details["submitted"] == false;
    // Cleanup proves what remains in the composer, not that a provider did
    // nothing. Post-paste attempts remain unknown and keep their original key.
    let code = if before_delivery {
        error.code()
    } else {
        "structured-prompt-outcome-unknown"
    };
    let mut response = envelope_err(CliError::data(code, error.message(), Some(details)));
    *response.status_mut() = StatusCode::CONFLICT;
    response
}
