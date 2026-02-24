use serde_json::Value;

pub fn print_success(schema_version: &str, command: &str, payload: &Value) -> Result<(), String> {
    let payload_json = serde_json::to_string(payload)
        .map_err(|err| format!("failed to serialize payload: {err}"))?;

    println!("schema_version: {schema_version}");
    println!("command: {command}");
    println!("status: ok");
    println!("payload: {payload_json}");

    Ok(())
}

pub fn print_error(
    schema_version: &str,
    command: &str,
    code: &str,
    message: &str,
) -> Result<(), String> {
    eprintln!("schema_version: {schema_version}");
    eprintln!("command: {command}");
    eprintln!("status: error");
    eprintln!("code: {code}");
    eprintln!("message: {message}");
    Ok(())
}
