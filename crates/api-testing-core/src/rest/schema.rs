use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct RestHeaders {
    pub accept_key_present: bool,
    pub user_headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestMultipartPart {
    pub name: String,
    pub value: Option<String>,
    pub file_path: Option<String>,
    pub base64: Option<String>,
    pub filename: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestExpect {
    pub status: u16,
    pub jq: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestCleanup {
    pub method: String,
    pub path_template: String,
    pub vars: BTreeMap<String, String>,
    pub expect_status: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestRequest {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, Vec<String>>,
    pub headers: RestHeaders,
    pub body: Option<serde_json::Value>,
    pub multipart: Option<Vec<RestMultipartPart>>,
    pub expect: Option<RestExpect>,
    pub cleanup: Option<RestCleanup>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestRequestFile {
    pub path: PathBuf,
    pub request: RestRequest,
}

fn json_scalar_to_string(value: &serde_json::Value) -> Result<String> {
    match value {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Null => anyhow::bail!("query values must be scalar or array of scalars"),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            anyhow::bail!("query values must be scalar or array of scalars")
        }
    }
}

fn json_value_to_string_loose(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        _ => value.to_string(),
    }
}

fn is_valid_header_key(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn uri_encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for &b in raw.as_bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~');
        if unreserved {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

impl RestRequest {
    pub fn query_string(&self) -> String {
        let mut pairs: Vec<(String, String)> = Vec::new();
        for (k, values) in &self.query {
            for v in values {
                pairs.push((uri_encode_component(k), uri_encode_component(v)));
            }
        }
        pairs
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&")
    }
}

impl RestRequestFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .with_context(|| format!("read request file: {}", path.display()))?;
        let raw: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("Request file is not valid JSON: {}", path.display()))?;
        let request = parse_rest_request_json(raw)?;
        Ok(Self {
            path: std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            request,
        })
    }
}

pub fn parse_rest_request_json(raw: serde_json::Value) -> Result<RestRequest> {
    let obj = raw
        .as_object()
        .context("Request file must be a JSON object")?;

    let method_raw = obj
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if method_raw.is_empty() {
        anyhow::bail!("Request is missing required field: method");
    }
    let method = method_raw.to_ascii_uppercase();
    if !method.chars().all(|c| c.is_ascii_alphabetic()) {
        anyhow::bail!("Invalid HTTP method: {method_raw}");
    }

    let path = obj
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if path.is_empty() {
        anyhow::bail!("Request is missing required field: path");
    }
    if !path.starts_with('/') {
        anyhow::bail!("Invalid path (must start with '/'): {path}");
    }
    if path.contains("://") {
        anyhow::bail!("Invalid path (must be relative, no scheme/host): {path}");
    }
    if path.contains('?') {
        anyhow::bail!("Invalid path (do not include query string; use .query): {path}");
    }

    let mut query: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(query_value) = obj.get("query")
        && !query_value.is_null()
    {
        let q = query_value.as_object().context("query must be an object")?;
        for (k, v) in q {
            let values = match v {
                serde_json::Value::Null => continue,
                serde_json::Value::Array(arr) => {
                    let mut out: Vec<String> = Vec::new();
                    for el in arr {
                        if el.is_null() {
                            continue;
                        }
                        match el {
                            serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                                anyhow::bail!("query array elements must be scalars");
                            }
                            _ => out.push(json_scalar_to_string(el)?),
                        }
                    }
                    out
                }
                serde_json::Value::Object(_) => {
                    anyhow::bail!(
                        "query values must be scalars or arrays (objects are not allowed)"
                    );
                }
                _ => vec![json_scalar_to_string(v)?],
            };

            if !values.is_empty() {
                query.insert(k.to_string(), values);
            }
        }
    }

    let mut accept_key_present = false;
    let mut user_headers: Vec<(String, String)> = Vec::new();
    if let Some(headers_value) = obj.get("headers")
        && !headers_value.is_null()
    {
        let h = headers_value
            .as_object()
            .context("headers must be an object")?;
        accept_key_present = h.keys().any(|k| k.eq_ignore_ascii_case("accept"));

        for (k, v) in h {
            if v.is_null() {
                continue;
            }

            let key_lower = k.to_ascii_lowercase();
            if key_lower == "authorization" || key_lower == "content-type" {
                continue;
            }

            if !is_valid_header_key(k) {
                anyhow::bail!("invalid header key: {k}");
            }

            if matches!(
                v,
                serde_json::Value::Object(_) | serde_json::Value::Array(_)
            ) {
                anyhow::bail!("header values must be scalars: {k}");
            }

            user_headers.push((k.to_string(), json_value_to_string_loose(v)));
        }
    }

    let body = if obj.contains_key("body") {
        Some(obj.get("body").cloned().unwrap_or(serde_json::Value::Null))
    } else {
        None
    };

    let multipart = if obj.contains_key("multipart") {
        let mut parts: Vec<RestMultipartPart> = Vec::new();
        match obj.get("multipart") {
            Some(serde_json::Value::Null) | None => {}
            Some(serde_json::Value::Array(arr)) => {
                for part in arr {
                    let part_obj = part
                        .as_object()
                        .context("multipart parts must be objects")?;

                    let name = part_obj
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    if name.is_empty() {
                        anyhow::bail!("Multipart part is missing required field: name");
                    }

                    let value = part_obj.get("value").and_then(|v| {
                        if v.is_null() {
                            None
                        } else {
                            let s = json_value_to_string_loose(v);
                            let s = s.trim().to_string();
                            (!s.is_empty()).then_some(s)
                        }
                    });
                    let file_path = part_obj
                        .get("filePath")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    let base64 = part_obj
                        .get("base64")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    let filename = part_obj
                        .get("filename")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());
                    let content_type = part_obj
                        .get("contentType")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());

                    parts.push(RestMultipartPart {
                        name,
                        value,
                        file_path,
                        base64,
                        filename,
                        content_type,
                    });
                }
            }
            Some(_) => anyhow::bail!("multipart must be an array"),
        }
        Some(parts)
    } else {
        None
    };

    if body.is_some() && multipart.is_some() {
        anyhow::bail!("Request cannot include both body and multipart.");
    }

    let expect = if obj.contains_key("expect") {
        let expect_value = obj.get("expect").unwrap_or(&serde_json::Value::Null);
        let expect_obj = expect_value.as_object();
        let status_value = expect_obj
            .and_then(|o| o.get("status"))
            .unwrap_or(&serde_json::Value::Null);
        let status_raw = if status_value.is_null() {
            String::new()
        } else {
            json_value_to_string_loose(status_value)
        };
        let status_raw = status_raw.trim().to_string();
        if status_raw.is_empty() {
            anyhow::bail!("Request includes expect but is missing expect.status");
        }
        if !status_raw.chars().all(|c| c.is_ascii_digit()) {
            anyhow::bail!("Invalid expect.status (must be an integer): {status_raw}");
        }
        let status: u16 = status_raw
            .parse()
            .with_context(|| format!("Invalid expect.status (must be an integer): {status_raw}"))?;

        let jq = match expect_obj.and_then(|o| o.get("jq")) {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(s)) => {
                let trimmed = s.trim().to_string();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            Some(_) => anyhow::bail!("expect.jq must be a string"),
        };

        Some(RestExpect { status, jq })
    } else {
        None
    };

    let cleanup = if obj.contains_key("cleanup") {
        let cleanup_value = obj.get("cleanup").unwrap_or(&serde_json::Value::Null);
        let cleanup_obj = cleanup_value
            .as_object()
            .context("cleanup must be an object")?;

        let method_raw = cleanup_obj
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("DELETE")
            .trim()
            .to_string();
        if method_raw.is_empty() {
            anyhow::bail!("cleanup.method is empty");
        }
        let method = method_raw.to_ascii_uppercase();

        let path_template = cleanup_obj
            .get("pathTemplate")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if path_template.is_empty() {
            anyhow::bail!("cleanup.pathTemplate is required");
        }

        let vars_value = cleanup_obj.get("vars").unwrap_or(&serde_json::Value::Null);
        let mut vars: BTreeMap<String, String> = BTreeMap::new();
        if !vars_value.is_null() {
            let vars_obj = vars_value
                .as_object()
                .context("cleanup.vars must be an object")?;
            for (k, v) in vars_obj {
                let expr = v
                    .as_str()
                    .with_context(|| format!("cleanup var '{k}' must be a string"))?;
                vars.insert(k.to_string(), expr.to_string());
            }
        }

        let expect_status_raw = cleanup_obj
            .get("expectStatus")
            .and_then(|v| (!v.is_null()).then(|| json_value_to_string_loose(v)))
            .unwrap_or_default();
        let expect_status_raw = expect_status_raw.trim().to_string();
        let expect_status_raw = if expect_status_raw.is_empty() {
            if method == "DELETE" {
                "204".to_string()
            } else {
                "200".to_string()
            }
        } else {
            expect_status_raw
        };
        if !expect_status_raw.chars().all(|c| c.is_ascii_digit()) {
            anyhow::bail!("Invalid cleanup.expectStatus (must be an integer): {expect_status_raw}");
        }
        let expect_status: u16 = expect_status_raw.parse().with_context(|| {
            format!("Invalid cleanup.expectStatus (must be an integer): {expect_status_raw}")
        })?;

        Some(RestCleanup {
            method,
            path_template,
            vars,
            expect_status,
        })
    } else {
        None
    };

    Ok(RestRequest {
        method,
        path,
        query,
        headers: RestHeaders {
            accept_key_present,
            user_headers,
        },
        body,
        multipart,
        expect,
        cleanup,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn rest_schema_parses_minimal_request() {
        let raw = serde_json::json!({"method":"get","path":"/health"});
        let req = parse_rest_request_json(raw).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/health");
    }

    #[test]
    fn rest_schema_query_string_sorts_keys_and_omits_nulls() {
        let raw = serde_json::json!({
            "method":"GET",
            "path":"/q",
            "query": { "b": [2, null, 3], "a": true, "z": null }
        });
        let req = parse_rest_request_json(raw).unwrap();
        assert_eq!(req.query_string(), "a=true&b=2&b=3");
    }

    #[test]
    fn rest_schema_rejects_body_and_multipart() {
        let raw = serde_json::json!({
            "method":"POST",
            "path":"/x",
            "body": {"a": 1},
            "multipart": []
        });
        let err = parse_rest_request_json(raw).unwrap_err();
        assert!(err.to_string().contains("both body and multipart"));
    }

    #[test]
    fn rest_schema_headers_ignore_reserved_and_validate_keys() {
        let raw = serde_json::json!({
            "method":"GET",
            "path":"/h",
            "headers": {
                "Authorization": "bad",
                "Content-Type": "bad2",
                "X-OK": 1
            }
        });
        let req = parse_rest_request_json(raw).unwrap();
        assert_eq!(
            req.headers.user_headers,
            vec![("X-OK".to_string(), "1".to_string())]
        );
    }

    #[test]
    fn rest_schema_expect_status_accepts_string() {
        let raw = serde_json::json!({
            "method":"GET",
            "path":"/h",
            "expect": {"status": "204"}
        });
        let req = parse_rest_request_json(raw).unwrap();
        assert_eq!(req.expect.unwrap().status, 204);
    }

    #[test]
    fn rest_schema_rejects_bad_expect_status() {
        let raw = serde_json::json!({
            "method":"GET",
            "path":"/h",
            "expect": {"status": "oops"}
        });
        let err = parse_rest_request_json(raw).unwrap_err();
        assert!(err.to_string().contains("Invalid expect.status"));
    }
}
