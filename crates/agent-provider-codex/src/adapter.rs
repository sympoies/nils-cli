use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateResponse, AuthStateStatus, CapabilitiesRequest,
    CapabilitiesResponse, Capability, ExecuteRequest, ExecuteResponse, HealthStatus,
    HealthcheckRequest, HealthcheckResponse, LimitsRequest, LimitsResponse, ProviderError,
    ProviderErrorCategory, ProviderMaturity, ProviderMetadata, ProviderResult,
};
use codex_cli::{agent, auth, paths};
use nils_common::process;
use serde_json::json;
use std::path::Path;
use std::time::Instant;

const PROVIDER_ID: &str = "codex";
const EXECUTE_CALLER: &str = "agent-provider-codex:execute";

#[derive(Debug, Clone, Default)]
pub struct CodexProviderAdapter;

impl CodexProviderAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderAdapterV1 for CodexProviderAdapter {
    fn metadata(&self) -> ProviderMetadata {
        ProviderMetadata::new(PROVIDER_ID).with_maturity(ProviderMaturity::Stable)
    }

    fn capabilities(&self, request: CapabilitiesRequest) -> ProviderResult<CapabilitiesResponse> {
        let codex_available = process::cmd_exists("codex");
        let (policy_enabled, _) = allow_dangerous_status("agent-provider-codex:capabilities");
        let execute_available = codex_available && policy_enabled;

        let mut capabilities = vec![
            Capability {
                name: "execute".to_string(),
                available: execute_available,
                description: Some(execute_description(codex_available, policy_enabled)),
            },
            Capability::available("healthcheck"),
            Capability::available("limits"),
            Capability::available("auth-state"),
        ];

        if request.include_experimental {
            capabilities.push(Capability {
                name: "diag.rate-limits".to_string(),
                available: codex_available,
                description: Some(
                    "Expose codex-cli rate-limit diagnostics (JSON: codex-cli.diag.rate-limits.v1 via --json/--format json)".to_string(),
                ),
            });
            capabilities.push(Capability {
                name: "auth.commands".to_string(),
                available: true,
                description: Some(
                    "Expose codex-cli auth flows (JSON: codex-cli.auth.v1 via --json/--format json)"
                        .to_string(),
                ),
            });
        }

        Ok(CapabilitiesResponse { capabilities })
    }

    fn healthcheck(&self, request: HealthcheckRequest) -> ProviderResult<HealthcheckResponse> {
        let codex_available = process::cmd_exists("codex");
        let (policy_enabled, policy_message) =
            allow_dangerous_status("agent-provider-codex:healthcheck");
        let auth_file = paths::resolve_auth_file();
        let auth_file_exists = auth_file.as_ref().is_some_and(|path| path.is_file());

        let status = if !codex_available {
            HealthStatus::Unhealthy
        } else if !policy_enabled || !auth_file_exists {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };

        let summary = match status {
            HealthStatus::Healthy => "codex adapter is ready",
            HealthStatus::Degraded => "codex adapter is partially ready",
            HealthStatus::Unhealthy => "codex adapter is unavailable",
            HealthStatus::Unknown => "codex adapter health unknown",
        }
        .to_string();

        let details = json!({
            "codex_binary_available": codex_available,
            "dangerous_policy_enabled": policy_enabled,
            "dangerous_policy_message": policy_message,
            "auth_file": auth_file.as_ref().map(|path| path.to_string_lossy().to_string()),
            "auth_file_exists": auth_file_exists,
            "requested_timeout_ms": request.timeout_ms,
        });

        Ok(HealthcheckResponse {
            status,
            summary: Some(summary),
            details: Some(details),
        })
    }

    fn execute(&self, request: ExecuteRequest) -> ProviderResult<ExecuteResponse> {
        let prompt = request
            .input
            .as_deref()
            .filter(|input| !input.trim().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| request.task.trim().to_string());

        if prompt.is_empty() {
            return Err(Box::new(ProviderError::new(
                ProviderErrorCategory::Validation,
                "missing-task",
                "execute task/input is required",
            )));
        }

        if !process::cmd_exists("codex") {
            return Err(Box::new(
                ProviderError::new(
                    ProviderErrorCategory::Dependency,
                    "missing-binary",
                    "codex binary is not available on PATH",
                )
                .with_details(json!({ "binary": "codex" })),
            ));
        }

        let (policy_enabled, policy_message) =
            allow_dangerous_status("agent-provider-codex:policy-check");
        if !policy_enabled {
            return Err(Box::new(
                ProviderError::new(
                    ProviderErrorCategory::Validation,
                    "disabled-policy",
                    policy_message.unwrap_or_else(|| {
                        "execution disabled (set CODEX_ALLOW_DANGEROUS_ENABLED=true)".to_string()
                    }),
                )
                .with_retryable(false),
            ));
        }

        let mut stderr = Vec::new();
        let started_at = Instant::now();
        let exit_code = agent::exec::exec_dangerous(&prompt, EXECUTE_CALLER, &mut stderr);
        let duration_ms = as_millis(started_at.elapsed());
        let stderr_text = stderr_to_string(&stderr);

        if exit_code == 0 {
            return Ok(ExecuteResponse {
                exit_code,
                stdout: String::new(),
                stderr: stderr_text,
                duration_ms,
            });
        }

        Err(Box::new(
            ProviderError::new(
                ProviderErrorCategory::Internal,
                "execute-failed",
                "codex execution returned non-zero exit code",
            )
            .with_details(json!({
                "exit_code": exit_code,
                "stderr": stderr_text,
                "task": request.task,
            })),
        ))
    }

    fn limits(&self, _request: LimitsRequest) -> ProviderResult<LimitsResponse> {
        Ok(LimitsResponse {
            max_concurrency: Some(1),
            max_timeout_ms: None,
            max_input_bytes: None,
        })
    }

    fn auth_state(&self, _request: AuthStateRequest) -> ProviderResult<AuthStateResponse> {
        let Some(auth_file) = paths::resolve_auth_file() else {
            return Ok(AuthStateResponse {
                state: AuthStateStatus::Unknown,
                subject: None,
                scopes: Vec::new(),
                expires_at: None,
            });
        };

        if !auth_file.is_file() {
            return Ok(AuthStateResponse {
                state: AuthStateStatus::Unauthenticated,
                subject: None,
                scopes: Vec::new(),
                expires_at: None,
            });
        }

        let email = auth::email_from_auth_file(&auth_file)
            .map_err(|err| invalid_auth_file_error(&auth_file, err.to_string()))?;
        let identity = auth::identity_from_auth_file(&auth_file)
            .map_err(|err| invalid_auth_file_error(&auth_file, err.to_string()))?;
        let account_id = auth::account_id_from_auth_file(&auth_file)
            .map_err(|err| invalid_auth_file_error(&auth_file, err.to_string()))?;

        let subject = email.or(identity).or(account_id);
        let state = if subject.is_some() {
            AuthStateStatus::Authenticated
        } else {
            AuthStateStatus::Unauthenticated
        };

        Ok(AuthStateResponse {
            state,
            subject,
            scopes: Vec::new(),
            expires_at: None,
        })
    }
}

fn allow_dangerous_status(caller: &str) -> (bool, Option<String>) {
    let mut stderr = Vec::new();
    let enabled = agent::exec::require_allow_dangerous(Some(caller), &mut stderr);
    (enabled, non_empty(stderr_to_string(&stderr)))
}

fn execute_description(codex_available: bool, policy_enabled: bool) -> String {
    if !codex_available {
        return "codex binary is not available on PATH".to_string();
    }

    if !policy_enabled {
        return "execution requires CODEX_ALLOW_DANGEROUS_ENABLED=true".to_string();
    }

    "codex execution is enabled".to_string()
}

fn invalid_auth_file_error(path: &Path, error: String) -> Box<ProviderError> {
    Box::new(
        ProviderError::new(
            ProviderErrorCategory::Auth,
            "invalid-auth-file",
            format!("failed to parse auth file: {}", path.display()),
        )
        .with_retryable(false)
        .with_details(json!({
            "path": path.to_string_lossy(),
            "error": error,
        })),
    )
}

fn stderr_to_string(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr).trim_end().to_string()
}

fn non_empty(value: String) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    Some(value)
}

fn as_millis(duration: std::time::Duration) -> Option<u64> {
    u64::try_from(duration.as_millis()).ok()
}
