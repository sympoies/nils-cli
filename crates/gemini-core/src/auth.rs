use std::path::Path;

use crate::error::CoreError;
use crate::json;
use crate::jwt;

pub fn identity_from_auth_file(path: &Path) -> Result<Option<String>, CoreError> {
    let value = json::read_json(path)?;
    let token = token_from_auth_json(&value);
    let payload = token
        .and_then(|tok| jwt::decode_payload_json(&tok))
        .and_then(|payload| jwt::identity_from_payload(&payload));
    Ok(payload)
}

pub fn email_from_auth_file(path: &Path) -> Result<Option<String>, CoreError> {
    let value = json::read_json(path)?;
    let token = token_from_auth_json(&value);
    let payload = token
        .and_then(|tok| jwt::decode_payload_json(&tok))
        .and_then(|payload| jwt::email_from_payload(&payload));
    Ok(payload)
}

pub fn account_id_from_auth_file(path: &Path) -> Result<Option<String>, CoreError> {
    let value = json::read_json(path)?;
    let account = json::string_at(&value, &["tokens", "account_id"])
        .or_else(|| json::string_at(&value, &["account_id"]));
    Ok(account)
}

pub fn last_refresh_from_auth_file(path: &Path) -> Result<Option<String>, CoreError> {
    let value = json::read_json(path)?;
    Ok(json::string_at(&value, &["last_refresh"]))
}

pub fn identity_key_from_auth_file(path: &Path) -> Result<Option<String>, CoreError> {
    let identity = identity_from_auth_file(path)?;
    let identity = match identity {
        Some(value) => value,
        None => return Ok(None),
    };
    let account_id = account_id_from_auth_file(path)?;
    let key = match account_id {
        Some(account) => format!("{}::{}", identity, account),
        None => identity,
    };
    Ok(Some(key))
}

fn token_from_auth_json(value: &serde_json::Value) -> Option<String> {
    json::string_at(value, &["tokens", "id_token"])
        .or_else(|| json::string_at(value, &["tokens", "access_token"]))
}
