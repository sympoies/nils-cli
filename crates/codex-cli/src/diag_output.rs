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
