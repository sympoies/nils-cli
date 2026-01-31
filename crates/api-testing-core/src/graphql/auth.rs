use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{env_file, jwt, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphqlAuthSourceUsed {
    None,
    JwtProfile { name: String },
    EnvAccessToken,
}

#[derive(Debug, Clone)]
pub struct GraphqlAuthResolution {
    pub bearer_token: Option<String>,
    pub source: GraphqlAuthSourceUsed,
    pub warnings: Vec<String>,
}

fn trim_non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn to_env_key(s: &str) -> String {
    let s = s.trim().to_ascii_uppercase();
    let mut out = String::new();
    let mut prev_us = false;
    for c in s.chars() {
        let ok = c.is_ascii_alphanumeric();
        if ok {
            out.push(c);
            prev_us = false;
            continue;
        }

        if !out.is_empty() && !prev_us {
            out.push('_');
            prev_us = true;
        }
    }

    while out.ends_with('_') {
        out.pop();
    }

    out
}

fn bool_from_env(
    raw: Option<String>,
    name: &str,
    default: bool,
    warnings: &mut Vec<String>,
) -> bool {
    let raw = raw.unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return default;
    }
    match raw.to_ascii_lowercase().as_str() {
        "true" => true,
        "false" => false,
        _ => {
            warnings.push(format!(
                "{name} must be true|false (got: {raw}); treating as false"
            ));
            false
        }
    }
}

fn parse_u64_default(raw: Option<String>, default: u64, min: u64) -> u64 {
    let raw = raw.unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return default;
    }
    if !raw.chars().all(|c| c.is_ascii_digit()) {
        return default;
    }
    let Ok(v) = raw.parse::<u64>() else {
        return default;
    };
    v.max(min)
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
            if let Some(v) = map.get("accessToken").or_else(|| map.get("token")) {
                if let Some(t) = find_token_in_value(v) {
                    return Some(t);
                }
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
        .and_then(|s| trim_non_empty(&s));
    let jwt_name_arg = jwt_name_arg.and_then(trim_non_empty);

    let jwt_profile_selected =
        jwt_name_arg.is_some() || jwt_name_env.is_some() || jwt_name_file.is_some();

    let (bearer_token, source) = if jwt_profile_selected {
        let jwt_name = jwt_name_arg
            .or(jwt_name_env)
            .or(jwt_name_file)
            .unwrap_or_else(|| "default".to_string())
            .to_ascii_lowercase();

        let jwt_key = to_env_key(&jwt_name);
        let token_var = format!("GQL_JWT_{jwt_key}");
        let token =
            env_file::read_var_last_wins(&token_var, &jwts_files)?.and_then(|s| trim_non_empty(&s));

        let token = if let Some(token) = token {
            token
        } else if let Some(token) =
            maybe_auto_login(setup_dir, endpoint_url, &jwt_name, operation_file)?
        {
            token
        } else {
            anyhow::bail!("JWT profile '{jwt_name}' is selected but no token was found and auto-login is not configured.");
        };

        (
            Some(token),
            GraphqlAuthSourceUsed::JwtProfile { name: jwt_name },
        )
    } else if let Some(t) = std::env::var("ACCESS_TOKEN")
        .ok()
        .and_then(|s| trim_non_empty(&s))
    {
        (Some(t), GraphqlAuthSourceUsed::EnvAccessToken)
    } else {
        (None, GraphqlAuthSourceUsed::None)
    };

    if let Some(token) = bearer_token.as_deref() {
        let enabled = bool_from_env(
            std::env::var("GQL_JWT_VALIDATE_ENABLED").ok(),
            "GQL_JWT_VALIDATE_ENABLED",
            true,
            &mut warnings,
        );
        let strict = bool_from_env(
            std::env::var("GQL_JWT_VALIDATE_STRICT").ok(),
            "GQL_JWT_VALIDATE_STRICT",
            false,
            &mut warnings,
        );
        let leeway_seconds =
            parse_u64_default(std::env::var("GQL_JWT_VALIDATE_LEEWAY_SECONDS").ok(), 0, 0);

        let label = match &source {
            GraphqlAuthSourceUsed::JwtProfile { name } => format!("jwt profile '{name}'"),
            GraphqlAuthSourceUsed::EnvAccessToken => "ACCESS_TOKEN".to_string(),
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
}
