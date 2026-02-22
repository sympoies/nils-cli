use std::path::Path;

use serde_json::Value;

use super::error::CoreError;
use super::json;
use super::jwt;

pub fn identity_from_auth_file(path: &Path) -> Result<Option<String>, CoreError> {
    let value = json::read_json(path)?;
    let payload = decoded_payload_from_auth_json(&value);
    Ok(identity_from_auth_json(payload.as_ref()))
}

pub fn email_from_auth_file(path: &Path) -> Result<Option<String>, CoreError> {
    let value = json::read_json(path)?;
    let payload = decoded_payload_from_auth_json(&value);
    Ok(email_from_auth_json(payload.as_ref()))
}

pub fn account_id_from_auth_file(path: &Path) -> Result<Option<String>, CoreError> {
    let value = json::read_json(path)?;
    let payload = decoded_payload_from_auth_json(&value);
    Ok(account_id_from_auth_json(&value, payload.as_ref()))
}

pub fn last_refresh_from_auth_file(path: &Path) -> Result<Option<String>, CoreError> {
    let value = json::read_json(path)?;
    Ok(json::string_at(&value, &["last_refresh"]))
}

pub fn identity_key_from_auth_file(path: &Path) -> Result<Option<String>, CoreError> {
    let value = json::read_json(path)?;
    let payload = decoded_payload_from_auth_json(&value);
    let identity = identity_from_auth_json(payload.as_ref());
    let identity = match identity {
        Some(value) => value,
        None => return Ok(None),
    };
    let account_id = account_id_from_auth_json(&value, payload.as_ref());
    let key = match account_id {
        Some(account) => format!("{}::{}", identity, account),
        None => identity,
    };
    Ok(Some(key))
}

pub fn token_from_auth_json(value: &serde_json::Value) -> Option<String> {
    json::string_at(value, &["tokens", "id_token"])
        .or_else(|| json::string_at(value, &["id_token"]))
        .or_else(|| json::string_at(value, &["tokens", "access_token"]))
        .or_else(|| json::string_at(value, &["access_token"]))
}

fn decoded_payload_from_auth_json(value: &Value) -> Option<Value> {
    let token = token_from_auth_json(value)?;
    jwt::decode_payload_json(&token)
}

fn identity_from_auth_json(payload: Option<&Value>) -> Option<String> {
    payload.and_then(jwt::identity_from_payload)
}

fn email_from_auth_json(payload: Option<&Value>) -> Option<String> {
    payload.and_then(jwt::email_from_payload)
}

fn account_id_from_auth_json(value: &Value, payload: Option<&Value>) -> Option<String> {
    json::string_at(value, &["tokens", "account_id"])
        .or_else(|| json::string_at(value, &["account_id"]))
        .or_else(|| {
            payload
                .and_then(|decoded| decoded.get("sub"))
                .and_then(|sub| sub.as_str())
                .map(json::strip_newlines)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use std::fs;
    use tempfile::TempDir;

    fn write_auth_json(path: &Path, contents: &str) {
        fs::write(path, contents).expect("write auth json");
    }

    fn jwt(payload: Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"none","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("{header}.{payload}.sig")
    }

    #[test]
    fn account_id_falls_back_to_token_sub_when_fields_missing() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("auth.json");
        let token = jwt(serde_json::json!({"sub":"acct_123"}));

        write_auth_json(&path, &format!(r#"{{"tokens":{{"id_token":"{token}"}}}}"#));

        assert_eq!(
            account_id_from_auth_file(&path).expect("account id"),
            Some("acct_123".to_string())
        );
    }

    #[test]
    fn identity_key_prefers_account_id_from_json_over_jwt_subject() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("auth.json");
        let token = jwt(serde_json::json!({
            "sub": "sub_from_jwt",
            "https://api.openai.com/auth": {"chatgpt_user_id":"user_456"}
        }));

        write_auth_json(
            &path,
            &format!(r#"{{"tokens":{{"id_token":"{token}","account_id":"acct_999"}}}}"#),
        );

        assert_eq!(
            identity_key_from_auth_file(&path).expect("identity key"),
            Some("user_456::acct_999".to_string())
        );
    }

    #[test]
    fn identity_key_is_none_when_identity_is_missing() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("auth.json");
        write_auth_json(&path, r#"{"tokens":{"account_id":"acct_100"}}"#);

        assert_eq!(
            identity_key_from_auth_file(&path).expect("identity key"),
            None
        );
    }
}
