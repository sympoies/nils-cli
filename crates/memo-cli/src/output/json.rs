use serde::Serialize;

use crate::errors::AppError;

#[derive(Debug, Serialize)]
struct JsonResultEnvelope<'a, T>
where
    T: Serialize,
{
    schema_version: &'a str,
    command: &'a str,
    ok: bool,
    result: T,
}

#[derive(Debug, Serialize)]
struct JsonResultsEnvelope<'a, T>
where
    T: Serialize,
{
    schema_version: &'a str,
    command: &'a str,
    ok: bool,
    results: T,
}

#[derive(Debug, Serialize)]
struct JsonResultsEnvelopeWithMeta<'a, T>
where
    T: Serialize,
{
    schema_version: &'a str,
    command: &'a str,
    ok: bool,
    results: T,
    #[serde(skip_serializing_if = "Option::is_none")]
    pagination: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct JsonErrorEnvelope<'a> {
    schema_version: &'a str,
    command: &'a str,
    ok: bool,
    error: crate::errors::JsonError<'a>,
}

pub fn emit_json_result<T>(schema_version: &str, command: &str, result: T) -> Result<(), AppError>
where
    T: Serialize,
{
    let envelope = JsonResultEnvelope {
        schema_version,
        command,
        ok: true,
        result,
    };
    print_json(&envelope)
}

pub fn emit_json_results<T>(schema_version: &str, command: &str, results: T) -> Result<(), AppError>
where
    T: Serialize,
{
    let envelope = JsonResultsEnvelope {
        schema_version,
        command,
        ok: true,
        results,
    };
    print_json(&envelope)
}

pub fn emit_json_results_with_meta<T>(
    schema_version: &str,
    command: &str,
    results: T,
    pagination: Option<serde_json::Value>,
    meta: Option<serde_json::Value>,
) -> Result<(), AppError>
where
    T: Serialize,
{
    let envelope = JsonResultsEnvelopeWithMeta {
        schema_version,
        command,
        ok: true,
        results,
        pagination,
        meta,
    };
    print_json(&envelope)
}

pub fn emit_json_error(
    schema_version: &str,
    command: &str,
    err: &AppError,
) -> Result<(), AppError> {
    let envelope = JsonErrorEnvelope {
        schema_version,
        command,
        ok: false,
        error: err.json_error(),
    };
    print_json(&envelope)
}

fn print_json<T>(value: &T) -> Result<(), AppError>
where
    T: Serialize,
{
    let encoded = serde_json::to_string(value).map_err(|err| {
        AppError::runtime(format!("failed to serialize JSON output: {err}"))
            .with_code("internal-error")
    })?;
    println!("{encoded}");
    Ok(())
}
