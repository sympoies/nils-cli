use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebsocketExpect {
    pub jq: Option<String>,
    pub text_contains: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebsocketStep {
    Send {
        text: String,
    },
    Receive {
        timeout_seconds: Option<u64>,
        expect: Option<WebsocketExpect>,
    },
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebsocketRequest {
    pub url: Option<String>,
    pub headers: Vec<(String, String)>,
    pub connect_timeout_seconds: Option<u64>,
    pub steps: Vec<WebsocketStep>,
    pub expect: Option<WebsocketExpect>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebsocketRequestFile {
    pub path: PathBuf,
    pub request: WebsocketRequest,
}

impl WebsocketRequestFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .with_context(|| format!("read websocket request file: {}", path.display()))?;
        let raw: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!(
                "websocket request file is not valid JSON: {}",
                path.display()
            )
        })?;
        let request = parse_websocket_request_json(raw)?;
        Ok(Self {
            path: std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            request,
        })
    }
}

fn scalar_to_string(value: &serde_json::Value) -> Result<String> {
    match value {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        serde_json::Value::Null => Ok(String::new()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            anyhow::bail!("headers values must be scalar")
        }
    }
}

fn parse_optional_u64(path_label: &str, raw: Option<&serde_json::Value>) -> Result<Option<u64>> {
    match raw {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{path_label} must be a positive integer")),
        Some(serde_json::Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() {
                Ok(None)
            } else {
                Ok(Some(s.parse::<u64>().with_context(|| {
                    format!("{path_label} is not a positive integer: {s}")
                })?))
            }
        }
        _ => anyhow::bail!("{path_label} must be a positive integer"),
    }
}

fn parse_expect(
    raw: Option<&serde_json::Value>,
    path_label: &str,
) -> Result<Option<WebsocketExpect>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }

    let obj = raw
        .as_object()
        .with_context(|| format!("{path_label} must be an object"))?;

    let jq = obj
        .get("jq")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let text_contains = obj
        .get("textContains")
        .or_else(|| obj.get("contains"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    if jq.is_none() && text_contains.is_none() {
        return Ok(None);
    }

    Ok(Some(WebsocketExpect { jq, text_contains }))
}

fn parse_send_text(raw: &serde_json::Value) -> Result<String> {
    match raw {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Object(_)
        | serde_json::Value::Array(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::Bool(_)
        | serde_json::Value::Null => {
            serde_json::to_string(raw).context("failed to serialize websocket send payload to text")
        }
    }
}

fn parse_steps(raw_steps: Option<&serde_json::Value>) -> Result<Vec<WebsocketStep>> {
    let raw_steps = raw_steps.context("websocket request .steps is required")?;
    let arr = raw_steps
        .as_array()
        .context("websocket request .steps must be an array")?;
    if arr.is_empty() {
        anyhow::bail!("websocket request .steps must include at least one step");
    }

    let mut out = Vec::new();
    for (idx, raw_step) in arr.iter().enumerate() {
        let obj = raw_step
            .as_object()
            .with_context(|| format!("websocket request .steps[{idx}] must be an object"))?;

        let step_type = obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();

        match step_type.as_str() {
            "send" => {
                let send_raw = obj
                    .get("text")
                    .or_else(|| obj.get("json"))
                    .or_else(|| obj.get("payload"))
                    .with_context(|| {
                        format!(
                            "websocket request .steps[{idx}] send step requires text/json/payload"
                        )
                    })?;
                out.push(WebsocketStep::Send {
                    text: parse_send_text(send_raw)?,
                });
            }
            "receive" => {
                let timeout_seconds = parse_optional_u64(
                    &format!("websocket request .steps[{idx}].timeoutSeconds"),
                    obj.get("timeoutSeconds"),
                )?;
                let expect = parse_expect(
                    obj.get("expect"),
                    &format!("websocket request .steps[{idx}].expect"),
                )?;
                out.push(WebsocketStep::Receive {
                    timeout_seconds,
                    expect,
                });
            }
            "close" => out.push(WebsocketStep::Close),
            _ => {
                anyhow::bail!(
                    "websocket request .steps[{idx}] has unsupported type '{}'",
                    step_type
                );
            }
        }
    }

    Ok(out)
}

pub fn parse_websocket_request_json(raw: serde_json::Value) -> Result<WebsocketRequest> {
    let obj = raw
        .as_object()
        .context("websocket request file must be a JSON object")?;

    let url = obj
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let mut headers: Vec<(String, String)> = Vec::new();
    if let Some(v) = obj.get("headers")
        && !v.is_null()
    {
        let m = v
            .as_object()
            .context("websocket request .headers must be an object")?;
        let mut sorted: BTreeMap<String, String> = BTreeMap::new();
        for (k, raw_v) in m {
            let key = k.trim();
            if key.is_empty() {
                continue;
            }
            let value = scalar_to_string(raw_v)?;
            if !value.trim().is_empty() {
                sorted.insert(key.to_string(), value);
            }
        }
        headers.extend(sorted);
    }

    let connect_timeout_seconds = parse_optional_u64(
        "websocket request .connectTimeoutSeconds",
        obj.get("connectTimeoutSeconds"),
    )?;

    let steps = parse_steps(obj.get("steps"))?;

    let expect = parse_expect(obj.get("expect"), "websocket request .expect")?;

    Ok(WebsocketRequest {
        url,
        headers,
        connect_timeout_seconds,
        steps,
        expect,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn websocket_schema_parses_steps_request() {
        let req = parse_websocket_request_json(serde_json::json!({
            "url": "ws://127.0.0.1:9001/ws",
            "steps": [
                { "type": "send", "text": "{\"ping\":true}" },
                { "type": "receive", "timeoutSeconds": 2, "expect": { "jq": ".ok == true" } },
                { "type": "close" }
            ]
        }))
        .unwrap();

        assert_eq!(req.steps.len(), 3);
        assert_eq!(req.url.as_deref(), Some("ws://127.0.0.1:9001/ws"));
    }

    #[test]
    fn websocket_schema_rejects_missing_steps() {
        let err = parse_websocket_request_json(serde_json::json!({})).unwrap_err();
        assert!(format!("{err:#}").contains(".steps is required"));
    }

    #[test]
    fn websocket_schema_rejects_empty_steps() {
        let err = parse_websocket_request_json(serde_json::json!({"steps": []})).unwrap_err();
        assert!(format!("{err:#}").contains("must include at least one step"));
    }

    #[test]
    fn websocket_schema_load_reads_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("health.ws.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "url": "ws://127.0.0.1:9001/ws",
                "steps": [
                    { "type": "send", "text": "ping" },
                    { "type": "receive", "expect": {"textContains": "ok"} }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = WebsocketRequestFile::load(&path).unwrap();
        assert_eq!(loaded.request.steps.len(), 2);
    }
}
