use std::process::Command;

use crate::auth::output;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum LoginMethod {
    GeminiBrowser,
    GeminiDeviceCode,
    ApiKey,
}

pub fn run(api_key: bool, device_code: bool) -> i32 {
    run_with_json(api_key, device_code, false)
}

pub fn run_with_json(api_key: bool, device_code: bool, output_json: bool) -> i32 {
    let method = match resolve_method(api_key, device_code) {
        Ok(method) => method,
        Err((code, message, details)) => {
            if output_json {
                let _ = output::emit_error("auth login", "invalid-usage", message, details);
            } else {
                eprintln!("{message}");
            }
            return code;
        }
    };

    let args = method.gemini_args();
    if output_json {
        let proc = match Command::new("gemini").args(args).output() {
            Ok(output) => output,
            Err(_) => {
                let _ = output::emit_error(
                    "auth login",
                    "login-exec-failed",
                    format!(
                        "gemini-login: failed to run gemini login for method {}",
                        method.as_str()
                    ),
                    Some(output::obj(vec![("method", output::s(method.as_str()))])),
                );
                return 1;
            }
        };

        if !proc.status.success() {
            let details = if let Some(code) = proc.status.code() {
                output::obj(vec![
                    ("method", output::s(method.as_str())),
                    ("exit_code", output::n(code as i64)),
                ])
            } else {
                output::obj(vec![("method", output::s(method.as_str()))])
            };
            let _ = output::emit_error(
                "auth login",
                "login-failed",
                format!("gemini-login: login failed for method {}", method.as_str()),
                Some(details),
            );
            return proc.status.code().unwrap_or(1).max(1);
        }

        let _ = output::emit_result(
            "auth login",
            output::obj(vec![
                ("method", output::s(method.as_str())),
                ("provider", output::s(method.provider())),
                ("completed", output::b(true)),
            ]),
        );
        return 0;
    }

    let status = match Command::new("gemini").args(args).status() {
        Ok(status) => status,
        Err(_) => {
            eprintln!(
                "gemini-login: failed to run gemini login for method {}",
                method.as_str()
            );
            return 1;
        }
    };

    if !status.success() {
        eprintln!("gemini-login: login failed for method {}", method.as_str());
        return status.code().unwrap_or(1).max(1);
    }

    println!("gemini: login complete (method: {})", method.as_str());
    0
}

fn resolve_method(
    api_key: bool,
    device_code: bool,
) -> std::result::Result<LoginMethod, ErrorTriplet> {
    if api_key && device_code {
        return Err((
            64,
            "gemini-login: --api-key cannot be combined with --device-code".to_string(),
            Some(output::obj(vec![
                ("api_key", output::b(true)),
                ("device_code", output::b(true)),
            ])),
        ));
    }

    if api_key {
        return Ok(LoginMethod::ApiKey);
    }
    if device_code {
        return Ok(LoginMethod::GeminiDeviceCode);
    }
    Ok(LoginMethod::GeminiBrowser)
}

type ErrorTriplet = (i32, String, Option<output::JsonValue>);

impl LoginMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::GeminiBrowser => "gemini-browser",
            Self::GeminiDeviceCode => "gemini-device-code",
            Self::ApiKey => "api-key",
        }
    }

    fn provider(self) -> &'static str {
        match self {
            Self::GeminiBrowser | Self::GeminiDeviceCode => "gemini",
            Self::ApiKey => "gemini-api",
        }
    }

    fn gemini_args(self) -> &'static [&'static str] {
        match self {
            Self::GeminiBrowser => &["login"],
            Self::GeminiDeviceCode => &["login", "--device-auth"],
            Self::ApiKey => &["login", "--with-api-key"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LoginMethod, resolve_method};

    #[test]
    fn resolve_method_defaults_to_gemini_browser() {
        assert_eq!(
            resolve_method(false, false).expect("method"),
            LoginMethod::GeminiBrowser
        );
    }

    #[test]
    fn resolve_method_selects_device_code_and_api_key() {
        assert_eq!(
            resolve_method(false, true).expect("method"),
            LoginMethod::GeminiDeviceCode
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
