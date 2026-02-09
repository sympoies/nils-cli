use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Result, auth_env, cli_util, env_file, jwt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphqlAuthSourceUsed {
    None,
    JwtProfile { name: String },
    EnvFallback { env_name: String },
}

#[derive(Debug, Clone)]
pub struct GraphqlAuthResolution {
    pub bearer_token: Option<String>,
    pub source: GraphqlAuthSourceUsed,
    pub warnings: Vec<String>,
}

fn extract_login_root_field_name(operation_text: &str) -> Option<String> {
    let mut in_sel = false;
    for raw_line in operation_text.lines() {
        let line = raw_line.trim_end_matches('\r');
        let line = line.trim_start();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        let mut s = line;
        if !in_sel {
            if let Some(pos) = s.find('{') {
                in_sel = true;
                s = &s[pos + 1..];
            } else {
                continue;
            }
        }

        let s = s.trim_start();
        if s.is_empty() || s.starts_with('}') {
            continue;
        }

        let mut chars = s.chars();
        let Some(first) = chars.next() else {
            continue;
        };
        if !(first == '_' || first.is_ascii_alphabetic()) {
            continue;
        }
        let mut out = String::new();
        out.push(first);
        for c in chars {
            if c == '_' || c.is_ascii_alphanumeric() {
                out.push(c);
            } else {
                break;
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

fn find_login_operation(setup_dir: &Path, profile: &str) -> Option<PathBuf> {
    let candidates = [
        setup_dir.to_path_buf(),
        setup_dir.join("operations"),
        setup_dir.join("ops"),
    ];

    for dir in candidates {
        if !dir.is_dir() {
            continue;
        }

        let prof = dir.join(format!("login.{profile}.graphql"));
        if prof.is_file() {
            return Some(prof);
        }
        let generic = dir.join("login.graphql");
        if generic.is_file() {
            return Some(generic);
        }
    }

    None
}

fn find_login_variables(login_op: &Path, profile: &str) -> Option<PathBuf> {
    let dir = login_op.parent().unwrap_or_else(|| Path::new("."));
    let candidates = [
        dir.join(format!("login.{profile}.variables.local.json")),
        dir.join(format!("login.{profile}.variables.json")),
        dir.join("login.variables.local.json"),
        dir.join("login.variables.json"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

fn find_token_in_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        serde_json::Value::Array(values) => values.iter().find_map(find_token_in_value),
        serde_json::Value::Object(map) => {
            if let Some(v) = map.get("accessToken").or_else(|| map.get("token"))
                && let Some(t) = find_token_in_value(v)
            {
                return Some(t);
            }
            for v in map.values() {
                if let Some(t) = find_token_in_value(v) {
                    return Some(t);
                }
            }
            None
        }
        _ => None,
    }
}

fn maybe_auto_login(
    setup_dir: &Path,
    endpoint_url: &str,
    profile: &str,
    op_path: Option<&Path>,
) -> Result<Option<String>> {
    let Some(login_op) = find_login_operation(setup_dir, profile) else {
        return Ok(None);
    };

    if let Some(op_path) = op_path {
        let op_abs = std::fs::canonicalize(op_path).unwrap_or_else(|_| op_path.to_path_buf());
        let login_abs = std::fs::canonicalize(&login_op).unwrap_or_else(|_| login_op.to_path_buf());
        if op_abs == login_abs {
            return Ok(None);
        }
    }

    let login_vars = find_login_variables(&login_op, profile);
    let op_file = crate::graphql::schema::GraphqlOperationFile::load(&login_op)?;
    let vars_json = match login_vars.as_deref() {
        None => None,
        Some(path) => {
            let vars = crate::graphql::vars::GraphqlVariablesFile::load(path, 0)?;
            Some(vars.variables)
        }
    };

    let executed = crate::graphql::runner::execute_graphql_request(
        endpoint_url,
        None,
        &op_file.operation,
        vars_json.as_ref(),
    )?;

    let root_field = extract_login_root_field_name(&op_file.operation).ok_or_else(|| {
        anyhow::anyhow!(
            "Failed to determine login root field from: {}",
            login_op.display()
        )
    })?;

    let body_json: serde_json::Value = serde_json::from_slice(&executed.response.body)
        .ok()
        .unwrap_or(serde_json::Value::Null);
    let token = body_json
        .get("data")
        .and_then(|d| d.get(&root_field))
        .and_then(find_token_in_value);

    if let Some(token) = token {
        return Ok(Some(token));
    }

    anyhow::bail!("Failed to extract JWT from login response (field: {root_field}).");
}

pub fn resolve_bearer_token(
    setup_dir: &Path,
    endpoint_url: &str,
    operation_file: Option<&Path>,
    jwt_name_arg: Option<&str>,
    stderr: &mut dyn Write,
) -> Result<GraphqlAuthResolution> {
    let mut warnings = Vec::new();

    let jwts_env = setup_dir.join("jwts.env");
    let jwts_local = setup_dir.join("jwts.local.env");
    let jwts_files: Vec<&Path> = if jwts_env.is_file() || jwts_local.is_file() {
        vec![&jwts_env, &jwts_local]
    } else {
        Vec::new()
    };

    let jwt_name_file = if !jwts_files.is_empty() {
        env_file::read_var_last_wins("GQL_JWT_NAME", &jwts_files)?
    } else {
        None
    };
    let jwt_name_env = std::env::var("GQL_JWT_NAME")
        .ok()
        .and_then(|s| cli_util::trim_non_empty(&s));
    let jwt_name_arg = jwt_name_arg.and_then(cli_util::trim_non_empty);

    let jwt_profile_selected =
        jwt_name_arg.is_some() || jwt_name_env.is_some() || jwt_name_file.is_some();

    let (bearer_token, source) = if jwt_profile_selected {
        let jwt_name = jwt_name_arg
            .or(jwt_name_env)
            .or(jwt_name_file)
            .unwrap_or_else(|| "default".to_string())
            .to_ascii_lowercase();

        let token = env_file::read_prefixed_var("GQL_JWT_", &jwt_name, &jwts_files)?
            .and_then(|s| cli_util::trim_non_empty(&s));

        let token = if let Some(token) = token {
            token
        } else if let Some(token) =
            maybe_auto_login(setup_dir, endpoint_url, &jwt_name, operation_file)?
        {
            token
        } else {
            anyhow::bail!(
                "JWT profile '{jwt_name}' is selected but no token was found and auto-login is not configured."
            );
        };

        (
            Some(token),
            GraphqlAuthSourceUsed::JwtProfile { name: jwt_name },
        )
    } else if let Some((token, env_name)) =
        auth_env::resolve_env_fallback(&["ACCESS_TOKEN", "SERVICE_TOKEN"])
    {
        (Some(token), GraphqlAuthSourceUsed::EnvFallback { env_name })
    } else {
        (None, GraphqlAuthSourceUsed::None)
    };

    if let Some(token) = bearer_token.as_deref() {
        let enabled = cli_util::bool_from_env(
            std::env::var("GQL_JWT_VALIDATE_ENABLED").ok(),
            "GQL_JWT_VALIDATE_ENABLED",
            true,
            None,
            &mut warnings,
        );
        let strict = cli_util::bool_from_env(
            std::env::var("GQL_JWT_VALIDATE_STRICT").ok(),
            "GQL_JWT_VALIDATE_STRICT",
            false,
            None,
            &mut warnings,
        );
        let leeway_seconds = cli_util::parse_u64_default(
            std::env::var("GQL_JWT_VALIDATE_LEEWAY_SECONDS").ok(),
            0,
            0,
        );

        let label = match &source {
            GraphqlAuthSourceUsed::JwtProfile { name } => format!("jwt profile '{name}'"),
            GraphqlAuthSourceUsed::EnvFallback { env_name } => env_name.to_string(),
            GraphqlAuthSourceUsed::None => "token".to_string(),
        };

        let opts = jwt::JwtValidationOptions {
            enabled,
            strict,
            leeway_seconds: i64::try_from(leeway_seconds).unwrap_or(i64::MAX),
        };

        match jwt::check_bearer_jwt(token, &label, opts)? {
            jwt::JwtCheck::Ok => {}
            jwt::JwtCheck::Warn(msg) => {
                let _ = writeln!(stderr, "api-gql: warning: {msg}");
            }
        }
    }

    for w in warnings {
        let _ = writeln!(stderr, "api-gql: warning: {w}");
    }

    Ok(GraphqlAuthResolution {
        bearer_token,
        source,
        warnings: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use nils_test_support::http::{HttpResponse, LoopbackServer};
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use tempfile::TempDir;

    fn write_file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, contents).expect("write");
    }

    #[test]
    fn graphql_auth_extracts_root_field_name_best_effort() {
        let op = r#"
# comment
query Login {
  login {
    accessToken
  }
}
"#;
        assert_eq!(extract_login_root_field_name(op).as_deref(), Some("login"));
    }

    #[test]
    fn graphql_auth_login_file_search_prefers_profile_specific() {
        let tmp = TempDir::new().expect("tmp");
        let setup = tmp.path().join("setup/graphql");
        std::fs::create_dir_all(&setup).expect("mkdir");
        write_file(
            &setup.join("login.admin.graphql"),
            "query Login { login { accessToken } }",
        );
        write_file(
            &setup.join("login.graphql"),
            "query Login { login { token } }",
        );

        let found = find_login_operation(&setup, "admin").expect("found");
        assert!(found.ends_with("login.admin.graphql"));
    }

    #[test]
    fn graphql_auth_helper_parsers_cover_defaults() {
        let mut warnings = Vec::new();
        assert!(cli_util::bool_from_env(
            Some("true".into()),
            "X",
            false,
            None,
            &mut warnings
        ));
        assert!(!cli_util::bool_from_env(
            Some("false".into()),
            "X",
            true,
            None,
            &mut warnings
        ));

        let mut warnings = Vec::new();
        assert!(!cli_util::bool_from_env(
            Some("nope".into()),
            "X",
            true,
            None,
            &mut warnings
        ));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("X must be true|false"));

        assert_eq!(cli_util::parse_u64_default(Some("".into()), 5, 1), 5);
        assert_eq!(cli_util::parse_u64_default(Some("nope".into()), 5, 1), 5);
        assert_eq!(cli_util::parse_u64_default(Some("0".into()), 5, 1), 1);
        assert_eq!(cli_util::parse_u64_default(Some("10".into()), 5, 1), 10);
    }

    #[test]
    fn graphql_auth_find_token_in_value_handles_nested_structures() {
        let value = serde_json::json!({
            "data": {
                "login": {
                    "token": "abc"
                }
            }
        });
        assert_eq!(find_token_in_value(&value), Some("abc".to_string()));

        let array_value = serde_json::json!([{"accessToken": "def"}]);
        assert_eq!(find_token_in_value(&array_value), Some("def".to_string()));

        let blank = serde_json::json!("  ");
        assert_eq!(find_token_in_value(&blank), None);
    }

    #[test]
    fn graphql_auth_resolve_uses_access_token_env() {
        let lock = GlobalStateLock::new();
        let _access = EnvGuard::set(&lock, "ACCESS_TOKEN", "env-token");
        let _service = EnvGuard::set(&lock, "SERVICE_TOKEN", "service-token");
        let _jwt_enabled = EnvGuard::set(&lock, "GQL_JWT_VALIDATE_ENABLED", "false");
        let _jwt_name = EnvGuard::remove(&lock, "GQL_JWT_NAME");

        let tmp = TempDir::new().expect("tmp");
        let mut stderr = Vec::new();
        let out = resolve_bearer_token(
            tmp.path(),
            "http://localhost/graphql",
            None,
            None,
            &mut stderr,
        )
        .expect("resolve");

        assert_eq!(out.bearer_token.as_deref(), Some("env-token"));
        assert_eq!(
            out.source,
            GraphqlAuthSourceUsed::EnvFallback {
                env_name: "ACCESS_TOKEN".to_string()
            }
        );
    }

    #[test]
    fn graphql_auth_resolve_falls_back_to_service_token_env() {
        let lock = GlobalStateLock::new();
        let _access = EnvGuard::set(&lock, "ACCESS_TOKEN", "  ");
        let _service = EnvGuard::set(&lock, "SERVICE_TOKEN", "service-token");
        let _jwt_enabled = EnvGuard::set(&lock, "GQL_JWT_VALIDATE_ENABLED", "false");
        let _jwt_name = EnvGuard::remove(&lock, "GQL_JWT_NAME");

        let tmp = TempDir::new().expect("tmp");
        let mut stderr = Vec::new();
        let out = resolve_bearer_token(
            tmp.path(),
            "http://localhost/graphql",
            None,
            None,
            &mut stderr,
        )
        .expect("resolve");

        assert_eq!(out.bearer_token.as_deref(), Some("service-token"));
        assert_eq!(
            out.source,
            GraphqlAuthSourceUsed::EnvFallback {
                env_name: "SERVICE_TOKEN".to_string()
            }
        );
    }

    #[test]
    fn graphql_auth_resolve_ignores_blank_service_token() {
        let lock = GlobalStateLock::new();
        let _access = EnvGuard::remove(&lock, "ACCESS_TOKEN");
        let _service = EnvGuard::set(&lock, "SERVICE_TOKEN", "  ");
        let _jwt_enabled = EnvGuard::set(&lock, "GQL_JWT_VALIDATE_ENABLED", "false");
        let _jwt_name = EnvGuard::remove(&lock, "GQL_JWT_NAME");

        let tmp = TempDir::new().expect("tmp");
        let mut stderr = Vec::new();
        let out = resolve_bearer_token(
            tmp.path(),
            "http://localhost/graphql",
            None,
            None,
            &mut stderr,
        )
        .expect("resolve");

        assert_eq!(out.bearer_token, None);
        assert_eq!(out.source, GraphqlAuthSourceUsed::None);
    }

    #[test]
    fn graphql_auth_resolve_prefers_profile_token_from_files() {
        let lock = GlobalStateLock::new();
        let _access = EnvGuard::remove(&lock, "ACCESS_TOKEN");
        let _jwt_enabled = EnvGuard::set(&lock, "GQL_JWT_VALIDATE_ENABLED", "false");
        let _jwt_name = EnvGuard::remove(&lock, "GQL_JWT_NAME");

        let tmp = TempDir::new().expect("tmp");
        write_file(
            &tmp.path().join("jwts.env"),
            "GQL_JWT_ADMIN=token-from-file\n",
        );

        let mut stderr = Vec::new();
        let out = resolve_bearer_token(
            tmp.path(),
            "http://localhost/graphql",
            None,
            Some("admin"),
            &mut stderr,
        )
        .expect("resolve");

        assert_eq!(out.bearer_token.as_deref(), Some("token-from-file"));
        assert_eq!(
            out.source,
            GraphqlAuthSourceUsed::JwtProfile {
                name: "admin".to_string()
            }
        );
    }

    #[test]
    fn graphql_auth_auto_login_fetches_token_and_vars() {
        let lock = GlobalStateLock::new();
        let _access = EnvGuard::remove(&lock, "ACCESS_TOKEN");
        let _jwt_enabled = EnvGuard::set(&lock, "GQL_JWT_VALIDATE_ENABLED", "false");
        let _jwt_name = EnvGuard::remove(&lock, "GQL_JWT_NAME");

        let tmp = TempDir::new().expect("tmp");
        let setup = tmp.path().join("setup/graphql");
        std::fs::create_dir_all(&setup).expect("mkdir");
        write_file(
            &setup.join("login.admin.graphql"),
            "query Login($user: String!) { login { accessToken } }",
        );
        write_file(
            &setup.join("login.admin.variables.json"),
            r#"{"user":"alice"}"#,
        );

        let server = LoopbackServer::new().expect("server");
        server.add_route(
            "POST",
            "/graphql",
            HttpResponse::new(200, r#"{"data":{"login":{"accessToken":"auto-token"}}}"#)
                .with_header("Content-Type", "application/json"),
        );

        let endpoint = format!("{}/graphql", server.url());
        let mut stderr = Vec::new();
        let out = resolve_bearer_token(&setup, &endpoint, None, Some("admin"), &mut stderr)
            .expect("resolve");

        assert_eq!(out.bearer_token.as_deref(), Some("auto-token"));
        assert_eq!(
            out.source,
            GraphqlAuthSourceUsed::JwtProfile {
                name: "admin".to_string()
            }
        );

        let requests = server.take_requests();
        assert_eq!(requests.len(), 1);
        let body = requests[0].body_text();
        assert!(body.contains("\"variables\""));
        assert!(body.contains("\"user\":\"alice\""));
    }
}
