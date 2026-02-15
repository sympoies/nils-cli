use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

use crate::Result;
use crate::grpc::schema::GrpcRequestFile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcExecutedRequest {
    pub target: String,
    pub method: String,
    pub grpc_status: i32,
    pub response_body: Vec<u8>,
    pub stderr: String,
}

fn resolve_local_path(request_file: &Path, raw: &str) -> PathBuf {
    let p = Path::new(raw);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        request_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(p)
    }
}

fn looks_like_authorization_metadata(metadata: &[(String, String)]) -> bool {
    metadata
        .iter()
        .any(|(k, _)| k.trim().eq_ignore_ascii_case("authorization"))
}

pub fn execute_grpc_request(
    request_file: &GrpcRequestFile,
    target: &str,
    bearer_token: Option<&str>,
) -> Result<GrpcExecutedRequest> {
    let target = target.trim();
    if target.is_empty() {
        anyhow::bail!("gRPC target URL/endpoint is empty");
    }

    let grpcurl_bin = std::env::var("GRPCURL_BIN")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "grpcurl".to_string());

    let mut cmd = Command::new(&grpcurl_bin);
    cmd.arg("-format").arg("json");

    if request_file.request.plaintext {
        cmd.arg("-plaintext");
    }

    if let Some(authority) = request_file.request.authority.as_deref() {
        cmd.arg("-authority").arg(authority);
    }
    if let Some(timeout) = request_file.request.timeout_seconds {
        cmd.arg("-max-time").arg(timeout.to_string());
    }
    for p in &request_file.request.import_paths {
        let path = resolve_local_path(&request_file.path, p);
        cmd.arg("-import-path").arg(path);
    }
    if let Some(proto) = request_file.request.proto.as_deref() {
        let path = resolve_local_path(&request_file.path, proto);
        cmd.arg("-proto").arg(path);
    }

    for (k, v) in &request_file.request.metadata {
        cmd.arg("-H").arg(format!("{k}: {v}"));
    }
    if let Some(token) = bearer_token
        && !looks_like_authorization_metadata(&request_file.request.metadata)
    {
        cmd.arg("-H").arg(format!("authorization: Bearer {token}"));
    }

    let body = serde_json::to_string(&request_file.request.body)
        .context("failed to serialize gRPC request body")?;
    cmd.arg("-d").arg(body);
    cmd.arg(target);
    cmd.arg(&request_file.request.method);

    let output = cmd
        .output()
        .with_context(|| format!("failed to run gRPC transport command '{}'", grpcurl_bin))?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        let code = output.status.code().unwrap_or(1);
        let detail = stderr.trim();
        if detail.is_empty() {
            anyhow::bail!("gRPC request failed (grpcurl exit={code})");
        }
        anyhow::bail!("gRPC request failed (grpcurl exit={code}): {detail}");
    }

    Ok(GrpcExecutedRequest {
        target: target.to_string(),
        method: request_file.request.method.clone(),
        grpc_status: 0,
        response_body: output.stdout,
        stderr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grpc::schema::GrpcRequestFile;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn grpc_runner_executes_mock_grpcurl_script() {
        let tmp = TempDir::new().unwrap();
        let script = tmp.path().join("grpcurl-mock.sh");
        std::fs::write(&script, "#!/bin/sh\necho '{\"ok\":true}'\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }

        let req_path = tmp.path().join("health.grpc.json");
        std::fs::write(
            &req_path,
            serde_json::to_vec(&serde_json::json!({
                "method":"health.HealthService/Check",
                "body":{"ping":"pong"}
            }))
            .unwrap(),
        )
        .unwrap();
        let req = GrpcRequestFile::load(&req_path).unwrap();

        // SAFETY: test-only process env mutation in isolated test process.
        unsafe { std::env::set_var("GRPCURL_BIN", &script) };
        let executed = execute_grpc_request(&req, "127.0.0.1:50051", None).unwrap();
        // SAFETY: test-only process env mutation in isolated test process.
        unsafe { std::env::remove_var("GRPCURL_BIN") };

        assert_eq!(executed.grpc_status, 0);
        assert_eq!(
            String::from_utf8_lossy(&executed.response_body).trim(),
            "{\"ok\":true}"
        );
    }

    #[test]
    fn grpc_runner_surfaces_non_zero_exit() {
        let tmp = TempDir::new().unwrap();
        let script = tmp.path().join("grpcurl-fail.sh");
        std::fs::write(&script, "#!/bin/sh\necho 'rpc error' 1>&2\nexit 7\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }
        let req_path = tmp.path().join("health.grpc.json");
        std::fs::write(
            &req_path,
            serde_json::to_vec(&serde_json::json!({
                "method":"health.HealthService/Check",
                "body":{}
            }))
            .unwrap(),
        )
        .unwrap();
        let req = GrpcRequestFile::load(&req_path).unwrap();

        // SAFETY: test-only process env mutation in isolated test process.
        unsafe { std::env::set_var("GRPCURL_BIN", &script) };
        let err = execute_grpc_request(&req, "127.0.0.1:50051", None).unwrap_err();
        // SAFETY: test-only process env mutation in isolated test process.
        unsafe { std::env::remove_var("GRPCURL_BIN") };

        let msg = format!("{err:#}");
        assert!(msg.contains("grpcurl exit=7"));
        assert!(msg.contains("rpc error"));
    }
}
