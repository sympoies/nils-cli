use anyhow::Result;
use nils_common::process as shared_process;
use serde_json::json;

use crate::auth::output::{self, AuthLoginResult};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum LoginMethod {
    ChatgptBrowser,
    ChatgptDeviceCode,
    ApiKey,
}

pub fn run(api_key: bool, device_code: bool) -> Result<i32> {
    run_with_json(api_key, device_code, false)
}

pub fn run_with_json(api_key: bool, device_code: bool, output_json: bool) -> Result<i32> {
    let method = match resolve_method(api_key, device_code) {
        Ok(method) => method,
        Err((code, message, details)) => {
            if output_json {
                output::emit_error("auth login", "invalid-usage", message, details)?;
            } else {
                eprintln!("{message}");
            }
            return Ok(code);
        }
    };

    let args = method.codex_args();
    if output_json {
        let proc = match shared_process::run_output("codex", &args) {
            Ok(output) => output,
            Err(_) => {
                output::emit_error(
                    "auth login",
                    "login-exec-failed",
                    format!(
                        "codex-login: failed to run codex login for method {}",
                        method.as_str()
                    ),
                    Some(json!({
                        "method": method.as_str(),
                    })),
                )?;
                return Ok(1);
            }
        };

        if !proc.status.success() {
            output::emit_error(
                "auth login",
                "login-failed",
                format!("codex-login: login failed for method {}", method.as_str()),
                Some(json!({
                    "method": method.as_str(),
                    "exit_code": proc.status.code(),
                })),
            )?;
            return Ok(proc.status.code().unwrap_or(1).max(1));
        }

        output::emit_result(
            "auth login",
            AuthLoginResult {
                method: method.as_str().to_string(),
                provider: method.provider().to_string(),
                completed: true,
            },
        )?;
        return Ok(0);
    }

    let status = match shared_process::run_status_inherit("codex", &args) {
        Ok(status) => status,
        Err(_) => {
            eprintln!(
                "codex-login: failed to run codex login for method {}",
                method.as_str()
            );
            return Ok(1);
        }
    };

    if !status.success() {
        eprintln!("codex-login: login failed for method {}", method.as_str());
        return Ok(status.code().unwrap_or(1).max(1));
    }

    println!("codex: login complete (method: {})", method.as_str());
    Ok(0)
}

fn resolve_method(
    api_key: bool,
    device_code: bool,
) -> std::result::Result<LoginMethod, ErrorTriplet> {
    if api_key && device_code {
        return Err((
            64,
            "codex-login: --api-key cannot be combined with --device-code".to_string(),
            Some(json!({
                "api_key": true,
                "device_code": true,
            })),
        ));
    }

    if api_key {
        return Ok(LoginMethod::ApiKey);
    }
    if device_code {
        return Ok(LoginMethod::ChatgptDeviceCode);
    }
    Ok(LoginMethod::ChatgptBrowser)
}

type ErrorTriplet = (i32, String, Option<serde_json::Value>);

impl LoginMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::ChatgptBrowser => "chatgpt-browser",
            Self::ChatgptDeviceCode => "chatgpt-device-code",
            Self::ApiKey => "api-key",
        }
    }

    fn provider(self) -> &'static str {
        match self {
            Self::ChatgptBrowser | Self::ChatgptDeviceCode => "chatgpt",
            Self::ApiKey => "openai-api",
        }
    }

    fn codex_args(self) -> Vec<&'static str> {
        match self {
            Self::ChatgptBrowser => vec!["login"],
            Self::ChatgptDeviceCode => vec!["login", "--device-auth"],
            Self::ApiKey => vec!["login", "--with-api-key"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LoginMethod, resolve_method};
    use pretty_assertions::assert_eq;

    #[test]
    fn resolve_method_defaults_to_chatgpt_browser() {
        assert_eq!(
            resolve_method(false, false).expect("method"),
            LoginMethod::ChatgptBrowser
        );
    }

    #[test]
    fn resolve_method_selects_device_code_and_api_key() {
        assert_eq!(
            resolve_method(false, true).expect("method"),
            LoginMethod::ChatgptDeviceCode
        );
        assert_eq!(
            resolve_method(true, false).expect("method"),
            LoginMethod::ApiKey
        );
    }

    #[test]
    fn resolve_method_rejects_conflicting_flags() {
        let err = resolve_method(true, true).expect_err("conflict should fail");
        assert_eq!(err.0, 64);
        assert!(err.1.contains("--api-key"));
    }
}
