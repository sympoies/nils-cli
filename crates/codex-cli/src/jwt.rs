use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde_json::Value;

use crate::json::strip_newlines;

pub fn decode_payload(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    if payload.is_empty() {
        return None;
    }

    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    String::from_utf8(decoded).ok()
}

pub fn decode_payload_json(token: &str) -> Option<Value> {
    let payload = decode_payload(token)?;
    serde_json::from_str(&payload).ok()
}

pub fn identity_from_payload(payload: &Value) -> Option<String> {
    nested_string(payload, "https://api.openai.com/auth", "chatgpt_user_id")
        .or_else(|| nested_string(payload, "https://api.openai.com/auth", "user_id"))
        .or_else(|| top_level_string(payload, "sub"))
        .or_else(|| top_level_string(payload, "email"))
}

pub fn email_from_payload(payload: &Value) -> Option<String> {
    top_level_string(payload, "email")
        .or_else(|| nested_string(payload, "https://api.openai.com/auth", "email"))
}

fn nested_string(payload: &Value, parent: &str, key: &str) -> Option<String> {
    payload
        .get(parent)
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_str())
        .map(strip_newlines)
}

fn top_level_string(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(strip_newlines)
}
