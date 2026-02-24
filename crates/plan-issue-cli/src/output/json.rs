use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
struct JsonSuccessEnvelope<'a> {
    schema_version: &'a str,
    command: &'a str,
    status: &'a str,
    payload: &'a Value,
}

#[derive(Debug, Serialize)]
struct JsonErrorEnvelope<'a> {
    schema_version: &'a str,
    command: &'a str,
    status: &'a str,
    error: JsonError<'a>,
}

#[derive(Debug, Serialize)]
struct JsonError<'a> {
    code: &'a str,
    message: &'a str,
}

pub fn print_success(schema_version: &str, command: &str, payload: &Value) -> Result<(), String> {
    let envelope = JsonSuccessEnvelope {
        schema_version,
        command,
        status: "ok",
        payload,
    };

    let rendered = serde_json::to_string(&envelope)
        .map_err(|err| format!("failed to serialize JSON output: {err}"))?;
    println!("{rendered}");
    Ok(())
}

pub fn print_error(
    schema_version: &str,
    command: &str,
    code: &str,
    message: &str,
) -> Result<(), String> {
    let envelope = JsonErrorEnvelope {
        schema_version,
        command,
        status: "error",
        error: JsonError { code, message },
    };

    let rendered = serde_json::to_string(&envelope)
        .map_err(|err| format!("failed to serialize JSON error output: {err}"))?;
    println!("{rendered}");
    Ok(())
}
