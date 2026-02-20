use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonEnvelopeResult<T: Serialize> {
    pub schema_version: String,
    pub command: String,
    pub ok: bool,
    pub result: T,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonEnvelopeResults<T: Serialize> {
    pub schema_version: String,
    pub command: String,
    pub ok: bool,
    pub results: Vec<T>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonEnvelopeError {
    pub schema_version: String,
    pub command: String,
    pub ok: bool,
    pub error: ErrorEnvelope,
}

pub fn emit_json<T: Serialize>(payload: &T) -> Result<()> {
    println!("{}", serde_json::to_string(payload)?);
    Ok(())
}

pub fn emit_success_result<T: Serialize>(
    schema_version: &str,
    command: &str,
    result: T,
) -> Result<()> {
    emit_json(&JsonEnvelopeResult {
        schema_version: schema_version.to_string(),
        command: command.to_string(),
        ok: true,
        result,
    })
}

pub fn emit_success_results<T: Serialize>(
    schema_version: &str,
    command: &str,
    results: Vec<T>,
) -> Result<()> {
    emit_json(&JsonEnvelopeResults {
        schema_version: schema_version.to_string(),
        command: command.to_string(),
        ok: true,
        results,
    })
}

pub fn emit_error(
    schema_version: &str,
    command: &str,
    code: &str,
    message: impl Into<String>,
    details: Option<Value>,
) -> Result<()> {
    emit_json(&JsonEnvelopeError {
        schema_version: schema_version.to_string(),
        command: command.to_string(),
        ok: false,
        error: ErrorEnvelope {
            code: code.to_string(),
            message: message.into(),
            details,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, to_value};

    #[test]
    fn error_envelope_serialization_omits_details_when_none() {
        let envelope = JsonEnvelopeError {
            schema_version: "gemini-cli.test.v1".to_string(),
            command: "diag test".to_string(),
            ok: false,
            error: ErrorEnvelope {
                code: "bad-input".to_string(),
                message: "invalid".to_string(),
                details: None,
            },
        };
        let value = to_value(envelope).expect("serialize");
        assert_eq!(value["ok"], false);
        assert!(value["error"].get("details").is_none());
    }

    #[test]
    fn emit_helpers_return_ok() {
        assert!(
            emit_success_result("gemini-cli.test.v1", "diag test", json!({"status":"ok"})).is_ok()
        );
        assert!(
            emit_success_results(
                "gemini-cli.test.v1",
                "diag test",
                vec![json!({"item":1}), json!({"item":2})]
            )
            .is_ok()
        );
        assert!(
            emit_error(
                "gemini-cli.test.v1",
                "diag test",
                "failure",
                "boom",
                Some(json!({"hint":"retry"})),
            )
            .is_ok()
        );
    }
}
