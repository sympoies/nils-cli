use std::io;

pub const AUTH_SCHEMA_VERSION: &str = "gemini-cli.auth.v1";

#[derive(Clone, Debug)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(i64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        match self {
            JsonValue::Null => out.push_str("null"),
            JsonValue::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            JsonValue::Number(value) => out.push_str(&value.to_string()),
            JsonValue::String(value) => {
                out.push('"');
                out.push_str(&escape_json(value));
                out.push('"');
            }
            JsonValue::Array(values) => {
                out.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    value.write_json(out);
                }
                out.push(']');
            }
            JsonValue::Object(fields) => {
                out.push('{');
                for (index, (key, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    out.push_str(&escape_json(key));
                    out.push_str("\":");
                    value.write_json(out);
                }
                out.push('}');
            }
        }
    }
}

pub fn s(value: impl Into<String>) -> JsonValue {
    JsonValue::String(value.into())
}

pub fn b(value: bool) -> JsonValue {
    JsonValue::Bool(value)
}

pub fn n(value: i64) -> JsonValue {
    JsonValue::Number(value)
}

pub fn null() -> JsonValue {
    JsonValue::Null
}

pub fn arr(values: Vec<JsonValue>) -> JsonValue {
    JsonValue::Array(values)
}

pub fn obj(fields: Vec<(&str, JsonValue)>) -> JsonValue {
    JsonValue::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

pub fn obj_dynamic(fields: Vec<(String, JsonValue)>) -> JsonValue {
    JsonValue::Object(fields)
}

pub fn emit_result(command: &str, result: JsonValue) -> io::Result<()> {
    println!("{}", render_result(command, result));
    Ok(())
}

pub fn emit_error(
    command: &str,
    code: &str,
    message: impl Into<String>,
    details: Option<JsonValue>,
) -> io::Result<()> {
    println!("{}", render_error(command, code, message, details));
    Ok(())
}

pub fn render_result(command: &str, result: JsonValue) -> String {
    obj(vec![
        ("schema_version", s(AUTH_SCHEMA_VERSION)),
        ("command", s(command)),
        ("ok", b(true)),
        ("result", result),
    ])
    .to_json_string()
}

pub fn render_error(
    command: &str,
    code: &str,
    message: impl Into<String>,
    details: Option<JsonValue>,
) -> String {
    let mut error_fields = vec![
        ("code".to_string(), s(code)),
        ("message".to_string(), s(message)),
    ];
    if let Some(details) = details {
        error_fields.push(("details".to_string(), details));
    }

    obj_dynamic(vec![
        ("schema_version".to_string(), s(AUTH_SCHEMA_VERSION)),
        ("command".to_string(), s(command)),
        ("ok".to_string(), b(false)),
        ("error".to_string(), JsonValue::Object(error_fields)),
    ])
    .to_json_string()
}

fn escape_json(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{obj, render_error, render_result};

    #[test]
    fn render_result_contains_schema_and_command() {
        let rendered = render_result("auth login", obj(vec![("completed", super::b(true))]));
        assert!(rendered.contains("\"schema_version\":\"gemini-cli.auth.v1\""));
        assert!(rendered.contains("\"command\":\"auth login\""));
        assert!(rendered.contains("\"ok\":true"));
    }

    #[test]
    fn render_error_omits_details_when_absent() {
        let rendered = render_error("auth save", "invalid-usage", "bad", None);
        assert!(rendered.contains("\"ok\":false"));
        assert!(!rendered.contains("\"details\""));
    }

    #[test]
    fn render_error_escapes_strings() {
        let rendered = render_error("auth use", "invalid", "a\"b", None);
        assert!(rendered.contains("a\\\"b"));
    }
}
