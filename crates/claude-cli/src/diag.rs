use agent_provider_claude::ClaudeProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::HealthcheckRequest;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
struct DiagEnvelope {
    schema_version: &'static str,
    command: &'static str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<DiagResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<DiagError>,
}

#[derive(Debug, Serialize)]
struct DiagResult {
    provider: &'static str,
    status: String,
    summary: Option<String>,
    details: Option<Value>,
}

#[derive(Debug, Serialize)]
struct DiagError {
    code: &'static str,
    message: &'static str,
}

pub fn healthcheck(json: bool, timeout_ms: Option<u64>) -> i32 {
    let adapter = ClaudeProviderAdapter::new();
    match adapter.healthcheck(HealthcheckRequest { timeout_ms }) {
        Ok(response) => {
            if json {
                let payload = DiagEnvelope {
                    schema_version: "claude-cli.diag.v1",
                    command: "diag healthcheck",
                    ok: true,
                    result: Some(DiagResult {
                        provider: "claude",
                        status: health_status_label(response.status).to_string(),
                        summary: response.summary,
                        details: response.details,
                    }),
                    error: None,
                };
                match serde_json::to_string_pretty(&payload) {
                    Ok(text) => {
                        println!("{text}");
                        return 0;
                    }
                    Err(err) => {
                        eprintln!("claude-cli diag: failed to encode json: {err}");
                        return 1;
                    }
                }
            }

            println!("provider: claude");
            println!("status: {}", health_status_label(response.status));
            println!(
                "summary: {}",
                response.summary.unwrap_or_else(|| "<none>".to_string())
            );
            if let Some(details) = response.details {
                match serde_json::to_string_pretty(&details) {
                    Ok(text) => println!("details: {text}"),
                    Err(_) => println!("details: <unavailable>"),
                }
            } else {
                println!("details: <none>");
            }
            0
        }
        Err(error) => {
            if json {
                let payload = DiagEnvelope {
                    schema_version: "claude-cli.diag.v1",
                    command: "diag healthcheck",
                    ok: false,
                    result: None,
                    error: Some(DiagError {
                        code: "provider-error",
                        message: "provider healthcheck failed",
                    }),
                };
                if let Ok(text) = serde_json::to_string_pretty(&payload) {
                    println!("{text}");
                }
            }
            eprintln!("claude-cli diag: {} ({})", error.message, error.code);
            1
        }
    }
}

pub fn rate_limits_unsupported(json: bool) -> i32 {
    let code = "unsupported-codex-only-command";
    let message = "claude-cli: `diag rate-limits` is codex-only; use `diag healthcheck` or `agentctl diag doctor --provider claude`";

    if json {
        let payload = DiagEnvelope {
            schema_version: "claude-cli.diag.v1",
            command: "diag rate-limits",
            ok: false,
            result: None,
            error: Some(DiagError { code, message }),
        };
        if let Ok(text) = serde_json::to_string_pretty(&payload) {
            println!("{text}");
        }
    }

    eprintln!("{message}");
    64
}

fn health_status_label(status: agent_runtime_core::schema::HealthStatus) -> &'static str {
    use agent_runtime_core::schema::HealthStatus;

    match status {
        HealthStatus::Healthy => "healthy",
        HealthStatus::Degraded => "degraded",
        HealthStatus::Unhealthy => "unhealthy",
        HealthStatus::Unknown => "unknown",
    }
}
