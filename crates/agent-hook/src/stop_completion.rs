//! Content-free admission evidence shared by raw and dispatched Claude Stops.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompletionCandidateReason {
    Candidate,
    StopHookActive,
    StopHookActiveMissing,
    StopHookActiveInvalid,
    BackgroundTasksPresent,
    BackgroundTasksMissing,
    BackgroundTasksInvalid,
}

pub fn classify(fields: &Map<String, Value>) -> CompletionCandidateReason {
    use CompletionCandidateReason::*;
    match fields.get("stop_hook_active") {
        Some(Value::Bool(false)) => {}
        Some(Value::Bool(true)) => return StopHookActive,
        Some(_) => return StopHookActiveInvalid,
        None => return StopHookActiveMissing,
    }
    match fields.get("background_tasks") {
        Some(Value::Array(tasks)) if tasks.is_empty() => Candidate,
        Some(Value::Array(_)) => BackgroundTasksPresent,
        Some(_) => BackgroundTasksInvalid,
        None => BackgroundTasksMissing,
    }
}
