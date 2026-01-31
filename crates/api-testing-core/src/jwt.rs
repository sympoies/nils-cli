use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JwtValidationOptions {
    pub enabled: bool,
    pub strict: bool,
    pub leeway_seconds: i64,
}

impl Default for JwtValidationOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            strict: false,
            leeway_seconds: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwtCheck {
    Ok,
    Warn(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JwtErr {
    NotJwt,
    InvalidJwt,
    ExpInvalid,
    NbfInvalid,
    Expired { exp: i64, now: i64 },
    NotYetValid { nbf: i64, now: i64 },
}

impl JwtErr {
    fn script_code(&self) -> String {
        match self {
            Self::NotJwt => "not_jwt".to_string(),
            Self::InvalidJwt => "invalid_jwt".to_string(),
            Self::ExpInvalid => "exp_invalid".to_string(),
            Self::NbfInvalid => "nbf_invalid".to_string(),
            Self::Expired { exp, now } => format!("expired exp={exp} now={now}"),
            Self::NotYetValid { nbf, now } => format!("nbf_in_future nbf={nbf} now={now}"),
        }
    }
}

fn unix_now_seconds() -> Result<i64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    Ok(i64::try_from(now).unwrap_or(i64::MAX))
}

fn parse_numeric(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Bool(_) => None,
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                return Some(i);
            }
            if let Some(u) = n.as_u64() {
                return i64::try_from(u).ok();
            }
            n.as_f64().map(|f| f as i64)
        }
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.chars().all(|c| c.is_ascii_digit()) {
                return trimmed.parse::<i64>().ok();
            }
            None
        }
        _ => None,
    }
}

fn decode_json_segment(segment: &str) -> Result<serde_json::Value> {
    let decoded = URL_SAFE_NO_PAD
        .decode(segment.as_bytes())
        .map_err(|_| anyhow::anyhow!("base64url decode failed"))?;
    let v: serde_json::Value =
        serde_json::from_slice(&decoded).map_err(|_| anyhow::anyhow!("json decode failed"))?;
    Ok(v)
}

fn validate_jwt_at(token: &str, leeway_seconds: i64, now: i64) -> std::result::Result<(), JwtErr> {
    let parts: Vec<&str> = token.trim().split('.').collect();
    if parts.len() != 3 {
        return Err(JwtErr::NotJwt);
    }

    let payload = match (decode_json_segment(parts[0]), decode_json_segment(parts[1])) {
        (Ok(_header), Ok(payload)) => payload,
        _ => return Err(JwtErr::InvalidJwt),
    };

    if let Some(exp) = payload.get("exp") {
        let Some(exp_val) = parse_numeric(exp) else {
            return Err(JwtErr::ExpInvalid);
        };
        if exp_val < (now - leeway_seconds) {
            return Err(JwtErr::Expired { exp: exp_val, now });
        }
    }

    if let Some(nbf) = payload.get("nbf") {
        let Some(nbf_val) = parse_numeric(nbf) else {
            return Err(JwtErr::NbfInvalid);
        };
        if nbf_val > (now + leeway_seconds) {
            return Err(JwtErr::NotYetValid { nbf: nbf_val, now });
        }
    }

    Ok(())
}

pub fn check_bearer_jwt_at(
    token: &str,
    label: &str,
    opts: JwtValidationOptions,
    now: i64,
) -> Result<JwtCheck> {
    let token = token.trim();
    if token.is_empty() || !opts.enabled {
        return Ok(JwtCheck::Ok);
    }

    let leeway_seconds = opts.leeway_seconds.max(0);
    match validate_jwt_at(token, leeway_seconds, now) {
        Ok(()) => Ok(JwtCheck::Ok),
        Err(JwtErr::Expired { exp, now }) => {
            let code = JwtErr::Expired { exp, now }.script_code();
            anyhow::bail!("JWT expired for {label} ({code})");
        }
        Err(JwtErr::NotYetValid { nbf, now }) => {
            let code = JwtErr::NotYetValid { nbf, now }.script_code();
            anyhow::bail!("JWT not yet valid for {label} ({code})");
        }
        Err(
            err @ (JwtErr::NotJwt | JwtErr::InvalidJwt | JwtErr::ExpInvalid | JwtErr::NbfInvalid),
        ) => {
            let code = err.script_code();
            if opts.strict {
                anyhow::bail!("invalid JWT for {label} ({code})");
            }
            Ok(JwtCheck::Warn(format!(
                "token for {label} is not a valid JWT ({code}); skipping format validation"
            )))
        }
    }
}

pub fn check_bearer_jwt(token: &str, label: &str, opts: JwtValidationOptions) -> Result<JwtCheck> {
    let now = unix_now_seconds()?;
    check_bearer_jwt_at(token, label, opts, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64url_json(value: &serde_json::Value) -> String {
        let bytes = serde_json::to_vec(value).expect("json");
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn make_jwt(payload: serde_json::Value) -> String {
        let header = serde_json::json!({"alg":"none","typ":"JWT"});
        format!("{}.{}.sig", b64url_json(&header), b64url_json(&payload))
    }

    #[test]
    fn jwt_ok_when_claims_are_valid() {
        let now = 1_700_000_000;
        let token = make_jwt(serde_json::json!({"exp": now + 10, "nbf": now - 10}));
        let out = check_bearer_jwt_at(&token, "t", JwtValidationOptions::default(), now).unwrap();
        assert_eq!(out, JwtCheck::Ok);
    }

    #[test]
    fn jwt_expired_is_hard_error_even_when_non_strict() {
        let now = 1_700_000_000;
        let token = make_jwt(serde_json::json!({"exp": now - 1}));
        let err =
            check_bearer_jwt_at(&token, "t", JwtValidationOptions::default(), now).unwrap_err();
        assert!(format!("{err:#}").contains("JWT expired"));
    }

    #[test]
    fn jwt_nbf_in_future_is_hard_error_even_when_non_strict() {
        let now = 1_700_000_000;
        let token = make_jwt(serde_json::json!({"nbf": now + 1}));
        let err =
            check_bearer_jwt_at(&token, "t", JwtValidationOptions::default(), now).unwrap_err();
        assert!(format!("{err:#}").contains("JWT not yet valid"));
    }

    #[test]
    fn jwt_format_errors_warn_when_non_strict() {
        let now = 1_700_000_000;
        let opts = JwtValidationOptions {
            strict: false,
            ..JwtValidationOptions::default()
        };

        let out = check_bearer_jwt_at("not.a.jwt", "t", opts, now).unwrap();
        match out {
            JwtCheck::Warn(msg) => assert!(msg.contains("skipping format validation")),
            other => panic!("expected warn, got {other:?}"),
        }
    }

    #[test]
    fn jwt_format_errors_fail_when_strict() {
        let now = 1_700_000_000;
        let opts = JwtValidationOptions {
            strict: true,
            ..JwtValidationOptions::default()
        };

        let err = check_bearer_jwt_at("not.a.jwt", "t", opts, now).unwrap_err();
        assert!(format!("{err:#}").contains("invalid JWT"));
    }

    #[test]
    fn jwt_leeway_applies_to_exp_and_nbf() {
        let now = 1_700_000_000;
        let token = make_jwt(serde_json::json!({"exp": now - 5, "nbf": now + 5}));
        let opts = JwtValidationOptions {
            leeway_seconds: 10,
            ..JwtValidationOptions::default()
        };

        let out = check_bearer_jwt_at(&token, "t", opts, now).unwrap();
        assert_eq!(out, JwtCheck::Ok);
    }

    #[test]
    fn jwt_validation_can_be_disabled() {
        let now = 1_700_000_000;
        let opts = JwtValidationOptions {
            enabled: false,
            ..JwtValidationOptions::default()
        };

        let out = check_bearer_jwt_at("not.a.jwt", "t", opts, now).unwrap();
        assert_eq!(out, JwtCheck::Ok);
    }
}
