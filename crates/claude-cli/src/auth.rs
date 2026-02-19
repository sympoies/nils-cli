use agent_provider_claude::ClaudeProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::AuthStateRequest;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct AuthStateEnvelope {
    schema_version: &'static str,
    command: &'static str,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<AuthStateResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<AuthStateError>,
}

#[derive(Debug, Serialize)]
struct AuthStateResult {
    state: String,
    subject: Option<String>,
    scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AuthStateError {
    code: String,
    message: String,
}

pub fn show(json: bool) -> i32 {
    let adapter = ClaudeProviderAdapter::new();
    match adapter.auth_state(AuthStateRequest::default()) {
        Ok(state) => {
            if json {
                let payload = AuthStateEnvelope {
                    schema_version: "claude-cli.auth-state.v1",
                    command: "auth-state show",
                    ok: true,
                    result: Some(AuthStateResult {
                        state: auth_state_label(state.state).to_string(),
                        subject: state.subject,
                        scopes: state.scopes,
                    }),
                    error: None,
                };
                match serde_json::to_string_pretty(&payload) {
                    Ok(text) => {
                        println!("{text}");
                        0
                    }
                    Err(err) => {
                        eprintln!("claude-cli auth-state: failed to encode json: {err}");
                        1
                    }
                }
            } else {
                println!("state: {}", auth_state_label(state.state));
                println!(
                    "subject: {}",
                    state.subject.unwrap_or_else(|| "<none>".to_string())
                );
                if state.scopes.is_empty() {
                    println!("scopes: <none>");
                } else {
                    println!("scopes: {}", state.scopes.join(","));
                }
                0
            }
        }
        Err(error) => {
            if json {
                let payload = AuthStateEnvelope {
                    schema_version: "claude-cli.auth-state.v1",
                    command: "auth-state show",
                    ok: false,
                    result: None,
                    error: Some(AuthStateError {
                        code: error.code.clone(),
                        message: error.message.clone(),
                    }),
                };
                if let Ok(text) = serde_json::to_string_pretty(&payload) {
                    println!("{text}");
                }
            }
            eprintln!("claude-cli auth-state: {} ({})", error.message, error.code);
            1
        }
    }
}

fn auth_state_label(state: agent_runtime_core::schema::AuthStateStatus) -> &'static str {
    use agent_runtime_core::schema::AuthStateStatus;

    match state {
        AuthStateStatus::Authenticated => "authenticated",
        AuthStateStatus::Unauthenticated => "unauthenticated",
        AuthStateStatus::Expired => "expired",
        AuthStateStatus::Unknown => "unknown",
    }
}
