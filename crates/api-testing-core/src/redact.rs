use crate::Result;

pub const REDACTED: &str = "<REDACTED>";

fn should_redact_key(key: &str) -> bool {
    let k = key.trim().to_ascii_lowercase();
    matches!(
        k.as_str(),
        "accesstoken"
            | "refreshtoken"
            | "password"
            | "token"
            | "apikey"
            | "authorization"
            | "cookie"
            | "set-cookie"
    )
}

fn redact_value(value: &mut serde_json::Value) {
    *value = serde_json::Value::String(REDACTED.to_string());
}

fn redact_in_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if should_redact_key(k) {
                    redact_value(v);
                } else {
                    redact_in_json(v);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for v in values {
                redact_in_json(v);
            }
        }
        _ => {}
    }
}

/// Apply default redaction rules to a JSON value in-place.
///
/// This is shared between REST and GraphQL report generation and history snippets.
pub fn redact_json(value: &mut serde_json::Value) -> Result<()> {
    redact_in_json(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn redact_replaces_common_secret_fields_recursively() {
        let mut v = serde_json::json!({
            "accessToken": "a",
            "refreshToken": "b",
            "password": "c",
            "token": "d",
            "apiKey": "e",
            "authorization": "Bearer x",
            "cookie": "a=b",
            "set-cookie": "c=d",
            "nested": {
                "Authorization": "Bearer y",
                "ok": "keep"
            },
            "arr": [
                {"token": "t"},
                {"ok": 1}
            ]
        });

        redact_json(&mut v).unwrap();

        assert_eq!(v["accessToken"], REDACTED);
        assert_eq!(v["refreshToken"], REDACTED);
        assert_eq!(v["password"], REDACTED);
        assert_eq!(v["token"], REDACTED);
        assert_eq!(v["apiKey"], REDACTED);
        assert_eq!(v["authorization"], REDACTED);
        assert_eq!(v["cookie"], REDACTED);
        assert_eq!(v["set-cookie"], REDACTED);
        assert_eq!(v["nested"]["Authorization"], REDACTED);
        assert_eq!(v["nested"]["ok"], "keep");
        assert_eq!(v["arr"][0]["token"], REDACTED);
        assert_eq!(v["arr"][1]["ok"], 1);
    }
}
