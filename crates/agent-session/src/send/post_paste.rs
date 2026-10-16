//! Error metadata after a terminal paste has completed successfully.
use crate::CliError;
use serde_json::{Map, Value, json};

pub(super) fn error(mut error: CliError) -> CliError {
    let mut details = match error.details_mut().take() {
        Some(Value::Object(details)) => details,
        Some(cause) => Map::from_iter([("cause".to_string(), cause)]),
        None => Map::new(),
    };
    details.insert("pasted".to_string(), json!(true));
    details.entry("stage").or_insert(json!("submit"));
    details.entry("atomic").or_insert(json!(false));
    *error.details_mut() = Some(Value::Object(details));
    error
}
