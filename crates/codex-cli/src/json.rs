use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub fn read_json(path: &Path) -> Result<Value> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read json: {}", path.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid json: {}", path.display()))?;
    Ok(value)
}

pub fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    cursor.as_str().map(strip_newlines)
}

pub fn strip_newlines(raw: &str) -> String {
    raw.split(&['\n', '\r'][..])
        .next()
        .unwrap_or("")
        .to_string()
}
