use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcExpect {
    pub status: Option<i32>,
    pub jq: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrpcRequest {
    pub method: String,
    pub body: serde_json::Value,
    pub metadata: Vec<(String, String)>,
    pub proto: Option<String>,
    pub import_paths: Vec<String>,
    pub plaintext: bool,
    pub authority: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub expect: Option<GrpcExpect>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrpcRequestFile {
    pub path: PathBuf,
    pub request: GrpcRequest,
}

impl GrpcRequestFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .with_context(|| format!("read gRPC request file: {}", path.display()))?;
        let raw: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!("gRPC request file is not valid JSON: {}", path.display())
        })?;
        let request = parse_grpc_request_json(raw)?;
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
            anyhow::bail!("metadata values must be scalar")
        }
    }
}

fn parse_status(raw: &serde_json::Value) -> Result<Option<i32>> {
    match raw {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(n) => {
            let Some(i) = n.as_i64() else {
                anyhow::bail!("expect.status must be an integer");
            };
            Ok(Some(i.try_into().unwrap_or(i32::MAX)))
        }
        serde_json::Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                return Ok(None);
            }
            let parsed: i32 = s
                .parse()
                .with_context(|| format!("expect.status is not an integer: {s}"))?;
            Ok(Some(parsed))
        }
        _ => anyhow::bail!("expect.status must be an integer"),
    }
}

pub fn parse_grpc_request_json(raw: serde_json::Value) -> Result<GrpcRequest> {
    let obj = raw
        .as_object()
        .context("gRPC request file must be a JSON object")?;

    let method = obj
        .get("method")
        .or_else(|| obj.get("rpc"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if method.is_empty() {
        anyhow::bail!("gRPC request is missing required field: method");
    }
    if !method.contains('/') {
        anyhow::bail!("Invalid gRPC method (expected service/method): {method}");
    }

    let body = obj
        .get("body")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let body = if body.is_null() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        body
    };
    if !matches!(body, serde_json::Value::Object(_)) {
        anyhow::bail!("gRPC request .body must be a JSON object");
    }

    let mut metadata: Vec<(String, String)> = Vec::new();
    if let Some(v) = obj.get("metadata")
        && !v.is_null()
    {
        let m = v
            .as_object()
            .context("gRPC request .metadata must be an object")?;
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
        metadata.extend(sorted);
    }

    let proto = obj
        .get("proto")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let import_paths = match obj.get("importPaths") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect(),
        Some(_) => anyhow::bail!("gRPC request .importPaths must be an array"),
    };

    let plaintext = match obj.get("plaintext") {
        None | Some(serde_json::Value::Null) => true,
        Some(serde_json::Value::Bool(v)) => *v,
        Some(_) => anyhow::bail!("gRPC request .plaintext must be boolean"),
    };

    let authority = obj
        .get("authority")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let timeout_seconds =
        match obj.get("timeoutSeconds") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Number(n)) => n.as_u64(),
            Some(serde_json::Value::String(s)) => {
                let s = s.trim();
                if s.is_empty() {
                    None
                } else {
                    Some(s.parse::<u64>().with_context(|| {
                        format!("timeoutSeconds is not a positive integer: {s}")
                    })?)
                }
            }
            Some(_) => anyhow::bail!("gRPC request .timeoutSeconds must be integer"),
        };

    let expect = match obj.get("expect") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => {
            let e = v
                .as_object()
                .context("gRPC request .expect must be an object")?;
            let status = parse_status(e.get("status").unwrap_or(&serde_json::Value::Null))?;
            let jq = e
                .get("jq")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            Some(GrpcExpect { status, jq })
        }
    };

    Ok(GrpcRequest {
        method,
        body,
        metadata,
        proto,
        import_paths,
        plaintext,
        authority,
        timeout_seconds,
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
    fn grpc_schema_parses_minimal_request() {
        let req = parse_grpc_request_json(serde_json::json!({
            "method": "health.HealthService/Check",
            "body": {}
        }))
        .unwrap();

        assert_eq!(req.method, "health.HealthService/Check");
        assert_eq!(req.body, serde_json::json!({}));
        assert!(req.metadata.is_empty());
        assert!(req.plaintext);
    }

    #[test]
    fn grpc_schema_rejects_missing_method() {
        let err = parse_grpc_request_json(serde_json::json!({"body":{}})).unwrap_err();
        assert!(format!("{err:#}").contains("missing required field: method"));
    }

    #[test]
    fn grpc_schema_load_reads_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("health.grpc.json");
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "method": "health.HealthService/Check",
                "body": {"ping":"pong"},
                "metadata": {"x-trace-id":"abc"},
                "expect": {"status": 0, "jq": ".ok == true"}
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = GrpcRequestFile::load(&path).unwrap();
        assert_eq!(loaded.request.method, "health.HealthService/Check");
        assert_eq!(loaded.request.metadata.len(), 1);
        assert_eq!(loaded.request.expect.as_ref().unwrap().status, Some(0));
    }
}
