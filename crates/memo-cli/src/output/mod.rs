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

pub fn format_item_id(item_id: i64) -> String {
    format!("itm_{item_id:08}")
}

fn print_json<T>(value: &T) -> Result<(), AppError>
where
    T: Serialize,
{
    let encoded = serde_json::to_string(value)
        .map_err(|err| AppError::runtime(format!("failed to serialize JSON output: {err}")))?;
    println!("{encoded}");
    Ok(())
}
