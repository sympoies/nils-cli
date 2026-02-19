use serde_json::Value;
use std::fs;
use std::path::Path;

use crate::error::CoreError;

pub fn read_json(path: &Path) -> Result<Value, CoreError> {
    let raw = fs::read_to_string(path).map_err(|err| {
        CoreError::auth(
            "read-json-failed",
            format!("failed to read json: {} ({err})", path.display()),
        )
    })?;
    let value: Value = serde_json::from_str(&raw).map_err(|err| {
        CoreError::auth(
            "invalid-json",
            format!("invalid json: {} ({err})", path.display()),
        )
    })?;
    Ok(value)
}

pub fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor.as_str().map(strip_newlines)
}

pub fn i64_at(value: &Value, path: &[&str]) -> Option<i64> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    match cursor {
        Value::Number(value) => value.as_i64(),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

pub fn strip_newlines(raw: &str) -> String {
    raw.split(&['\n', '\r'][..])
        .next()
        .unwrap_or("")
        .to_string()
}
