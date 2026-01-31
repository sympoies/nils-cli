use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;

use crate::suite::auth::{AuthInit, SuiteAuthManager};
use crate::suite::cleanup::{run_case_cleanup, CleanupContext};
use crate::suite::filter::selection_skip_reason;
use crate::suite::resolve::{
    resolve_gql_url_for_env, resolve_path_from_repo_root, resolve_rest_base_url_for_env, write_file,
};
use crate::suite::results::{SuiteCaseResult, SuiteRunResults, SuiteRunSummary};
use crate::suite::safety::{
    graphql_safety_decision, rest_method_is_write, writes_enabled,
    MSG_WRITE_CAPABLE_REQUIRES_ALLOW_WRITE_TRUE, MSG_WRITE_CASES_DISABLED,
};
use crate::suite::schema::{LoadedSuite, SuiteDefaults};
use crate::Result;

fn trim_non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let mut out = String::from("'");
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn mask_args_for_command_snippet(args: &[String]) -> String {
    if args.is_empty() {
        return String::new();
    }

    let mut out: Vec<String> = Vec::new();
    let mut mask_next = false;
    for a in args {
        if mask_next {
            out.push("REDACTED".to_string());
            mask_next = false;
            continue;
        }
        if a == "--token" || a == "--jwt" {
            out.push(a.clone());
            mask_next = true;
            continue;
        }
        if let Some((k, _v)) = a.split_once('=') {
            if k == "--token" || k == "--jwt" {
                out.push(format!("{k}=REDACTED"));
                continue;
            }
        }
        out.push(a.clone());
    }

    out.into_iter()
        .map(|a| shell_quote(&a))
        .collect::<Vec<_>>()
        .join(" ")
}

fn sanitize_id(id: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in id.chars() {
        let ok = c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-';
        if ok {
            out.push(c);
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "case".to_string()
    } else {
        out
    }
}

fn time_run_id_now() -> Result<String> {
    let format = time::format_description::parse("[year][month][day]-[hour][minute][second]Z")?;
    Ok(time::OffsetDateTime::now_utc().format(&format)?)
}

fn time_iso_now() -> Result<String> {
    let format = time::format_description::parse("[year]-[month]-[day]T[hour]:[minute]:[second]Z")?;
    Ok(time::OffsetDateTime::now_utc().format(&format)?)
}

fn path_relative_to_repo_or_abs(repo_root: &Path, path: &Path) -> String {
    if let Ok(stripped) = path.strip_prefix(repo_root) {
        let s = stripped.to_string_lossy();
        if s.is_empty() {
            ".".to_string()
        } else {
            s.to_string()
        }
    } else {
        path.to_string_lossy().to_string()
    }
}

fn resolve_rest_base_url(
    repo_root: &Path,
    setup_dir_raw: &str,
    url_override: &str,
    env_value: &str,
    suite_defaults: &SuiteDefaults,
    env_rest_url: &str,
) -> Result<String> {
    let url_override = url_override.trim();
    if !url_override.is_empty() {
        return Ok(url_override.to_string());
    }
    let default_url = suite_defaults.rest.url.trim();
    if !default_url.is_empty() {
        return Ok(default_url.to_string());
    }
    let env_rest_url = env_rest_url.trim();
    if !env_rest_url.is_empty() {
        return Ok(env_rest_url.to_string());
    }
    let env_value = env_value.trim();
    if !env_value.is_empty() {
        let setup_dir = resolve_path_from_repo_root(repo_root, setup_dir_raw);
        return resolve_rest_base_url_for_env(&setup_dir, env_value);
    }
    Ok("http://localhost:6700".to_string())
}

fn resolve_gql_url(
    repo_root: &Path,
    setup_dir_raw: &str,
    url_override: &str,
    env_value: &str,
    suite_defaults: &SuiteDefaults,
    env_gql_url: &str,
) -> Result<String> {
    let url_override = url_override.trim();
    if !url_override.is_empty() {
        return Ok(url_override.to_string());
    }
    let default_url = suite_defaults.graphql.url.trim();
    if !default_url.is_empty() {
        return Ok(default_url.to_string());
    }
    let env_gql_url = env_gql_url.trim();
    if !env_gql_url.is_empty() {
        return Ok(env_gql_url.to_string());
    }
    let env_value = env_value.trim();
    if !env_value.is_empty() {
        let setup_dir = resolve_path_from_repo_root(repo_root, setup_dir_raw);
        return resolve_gql_url_for_env(&setup_dir, env_value);
    }
    Ok("http://localhost:6700/graphql".to_string())
}

fn resolve_rest_token_profile(setup_dir: &Path, profile: &str) -> Result<String> {
    let tokens_env = setup_dir.join("tokens.env");
    let tokens_local = setup_dir.join("tokens.local.env");
    let files: Vec<&Path> = if tokens_env.is_file() || tokens_local.is_file() {
        vec![&tokens_env, &tokens_local]
    } else {
        Vec::new()
    };

    let key = profile.trim().to_ascii_uppercase();
    let mut env_key = String::new();
    for c in key.chars() {
        if c.is_ascii_alphanumeric() {
            env_key.push(c);
        } else if !env_key.ends_with('_') {
            env_key.push('_');
        }
    }
    while env_key.ends_with('_') {
        env_key.pop();
    }

    let var = format!("REST_TOKEN_{env_key}");
    let found = crate::env_file::read_var_last_wins(&var, &files)?;
    found.ok_or_else(|| anyhow::anyhow!("Token profile '{profile}' is empty/missing."))
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

#[derive(Debug, Clone)]
pub struct SuiteRunOptions {
    pub required_tags: Vec<String>,
    pub only_ids: HashSet<String>,
    pub skip_ids: HashSet<String>,
    pub allow_writes_flag: bool,
    pub fail_fast: bool,
    pub output_dir_base: PathBuf,
    pub env_rest_url: String,
    pub env_gql_url: String,
}

#[derive(Debug, Clone)]
pub struct SuiteRunOutput {
    pub run_dir_abs: PathBuf,
    pub results: SuiteRunResults,
}

fn suite_display_name(loaded: &LoadedSuite) -> String {
    let name = loaded.manifest.name.trim();
    if !name.is_empty() {
        return name.to_string();
    }
    loaded
        .suite_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("suite")
        .to_string()
}

fn case_type_normalized(case_type_raw: &str) -> String {
    case_type_raw.trim().to_ascii_lowercase()
}

fn default_rest_flow_token_jq() -> String {
    ".. | objects | (.accessToken? // .access_token? // .token? // empty) | select(type==\"string\" and length>0) | .".to_string()
}

pub fn run_suite(
    repo_root: &Path,
    loaded: LoadedSuite,
    options: SuiteRunOptions,
) -> Result<SuiteRunOutput> {
    let run_id = time_run_id_now()?;
    let started_at = time_iso_now()?;

    let run_dir_abs = options.output_dir_base.join(&run_id);
    std::fs::create_dir_all(&run_dir_abs)
        .with_context(|| format!("create output dir: {}", run_dir_abs.display()))?;

    let suite_file_rel = path_relative_to_repo_or_abs(repo_root, &loaded.suite_path);
    let output_dir_rel = path_relative_to_repo_or_abs(repo_root, &run_dir_abs);
    let suite_name_value = suite_display_name(&loaded);

    let defaults = &loaded.manifest.defaults;

    let mut auth_init_message: Option<String> = None;
    let mut auth_manager: Option<SuiteAuthManager> = match loaded.manifest.auth.clone() {
        None => None,
        Some(auth) => match SuiteAuthManager::init_from_suite(auth, defaults)? {
            AuthInit::Disabled { message } => {
                auth_init_message = message;
                None
            }
            AuthInit::Enabled(mgr) => Some(*mgr),
        },
    };

    let mut total: u32 = 0;
    let mut passed: u32 = 0;
    let mut failed: u32 = 0;
    let mut skipped: u32 = 0;

    let mut cases_out: Vec<SuiteCaseResult> = Vec::new();

    for c in &loaded.manifest.cases {
        total += 1;

        let id = c.id.trim().to_string();
        let safe_id = sanitize_id(&id);

        let tags = c.tags.clone();
        let ty = case_type_normalized(&c.case_type);

        let effective_env = c.env.trim().to_string();
        let effective_env = if effective_env.is_empty() {
            defaults.env.trim().to_string()
        } else {
            effective_env
        };

        let effective_no_history = c.no_history.unwrap_or(defaults.no_history);

        if let Some(reason) = selection_skip_reason(
            &id,
            &tags,
            &options.required_tags,
            &options.only_ids,
            &options.skip_ids,
        ) {
            skipped += 1;
            cases_out.push(SuiteCaseResult {
                id,
                case_type: ty,
                status: "skipped".to_string(),
                duration_ms: 0,
                tags,
                command: None,
                message: Some(reason.as_str().to_string()),
                assertions: None,
                stdout_file: None,
                stderr_file: None,
            });
            continue;
        }

        let start = Instant::now();

        let mut status = "pending".to_string();
        let mut message: Option<String> = None;
        let mut assertions: Option<serde_json::Value> = None;
        let mut command_snippet: Option<String> = None;
        let mut stdout_file_abs: Option<PathBuf> = None;
        let mut stderr_file_abs: Option<PathBuf> = None;

        let mut rest_config_dir = String::new();
        let mut rest_url = String::new();
        let mut rest_token = String::new();
        let mut gql_config_dir = String::new();
        let mut gql_url = String::new();
        let mut gql_jwt = String::new();
        let mut access_token_for_case = String::new();

        match ty.as_str() {
            "rest" => {
                let request_rel = c.request.trim();
                if request_rel.is_empty() {
                    anyhow::bail!("REST case '{id}' is missing request");
                }
                let request_abs = resolve_path_from_repo_root(repo_root, request_rel);
                if !request_abs.is_file() {
                    anyhow::bail!("REST case '{id}' request not found: {request_rel}");
                }

                rest_config_dir = trim_non_empty(&c.config_dir)
                    .unwrap_or_else(|| defaults.rest.config_dir.clone());
                rest_token =
                    trim_non_empty(&c.token).unwrap_or_else(|| defaults.rest.token.clone());

                rest_url = trim_non_empty(&c.url).unwrap_or_else(|| defaults.rest.url.clone());
                if rest_url.trim().is_empty() && !options.env_rest_url.trim().is_empty() {
                    rest_url = options.env_rest_url.clone();
                }

                let request_file = crate::rest::schema::RestRequestFile::load(&request_abs)?;

                if rest_method_is_write(&request_file.request.method) {
                    if !c.allow_write {
                        status = "failed".to_string();
                        message = Some(MSG_WRITE_CAPABLE_REQUIRES_ALLOW_WRITE_TRUE.to_string());
                        failed += 1;
                    } else if !writes_enabled(options.allow_writes_flag, &effective_env) {
                        status = "skipped".to_string();
                        message = Some(MSG_WRITE_CASES_DISABLED.to_string());
                        skipped += 1;
                    }
                }

                if status != "failed" && status != "skipped" {
                    // Auth injection (optional)
                    if let Some(mgr) = auth_manager.as_mut() {
                        if !rest_token.trim().is_empty() {
                            match mgr.ensure_token(
                                &rest_token,
                                repo_root,
                                defaults,
                                &options.env_rest_url,
                                &options.env_gql_url,
                            ) {
                                Ok(t) => access_token_for_case = t,
                                Err(err) => {
                                    status = "failed".to_string();
                                    message = Some(err);
                                    failed += 1;
                                }
                            }
                        }
                    }
                }

                if status != "failed" && status != "skipped" {
                    let stdout_path = run_dir_abs.join(format!("{safe_id}.response.json"));
                    let stderr_path = run_dir_abs.join(format!("{safe_id}.stderr.log"));
                    write_file(&stdout_path, b"")?;
                    write_file(&stderr_path, b"")?;
                    stdout_file_abs = Some(stdout_path.clone());
                    stderr_file_abs = Some(stderr_path.clone());

                    let base_url = match resolve_rest_base_url(
                        repo_root,
                        &rest_config_dir,
                        &rest_url,
                        &effective_env,
                        defaults,
                        &options.env_rest_url,
                    ) {
                        Ok(v) => v,
                        Err(err) => {
                            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                            status = "failed".to_string();
                            message = Some("rest_runner_failed".to_string());
                            failed += 1;
                            // still produce case result below
                            String::new()
                        }
                    };

                    if status != "failed" {
                        let setup_dir_abs =
                            resolve_path_from_repo_root(repo_root, &rest_config_dir);
                        let bearer = if !access_token_for_case.trim().is_empty() {
                            Some(access_token_for_case.clone())
                        } else if !rest_token.trim().is_empty() {
                            match resolve_rest_token_profile(&setup_dir_abs, &rest_token) {
                                Ok(t) => Some(t),
                                Err(err) => {
                                    write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                    status = "failed".to_string();
                                    message = Some("rest_runner_failed".to_string());
                                    failed += 1;
                                    None
                                }
                            }
                        } else {
                            None
                        };

                        if status == "failed" {
                            // token resolution failure already recorded
                        } else {
                            match crate::rest::runner::execute_rest_request(
                                &request_file,
                                &base_url,
                                bearer.as_deref(),
                            ) {
                                Ok(executed) => {
                                    write_file(&stdout_path, &executed.response.body)?;
                                    if let Err(err) = crate::rest::expect::evaluate_main_response(
                                        &request_file.request,
                                        &executed,
                                    ) {
                                        write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                        status = "failed".to_string();
                                        message = Some("rest_runner_failed".to_string());
                                        failed += 1;
                                    } else {
                                        status = "passed".to_string();
                                        passed += 1;
                                    }
                                }
                                Err(err) => {
                                    write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                    status = "failed".to_string();
                                    message = Some("rest_runner_failed".to_string());
                                    failed += 1;
                                }
                            }
                        }
                    }

                    let mut argv: Vec<String> = vec![
                        "api-rest".to_string(),
                        "call".to_string(),
                        "--config-dir".to_string(),
                        rest_config_dir.clone(),
                    ];
                    if effective_no_history {
                        argv.push("--no-history".to_string());
                    }
                    if !rest_url.trim().is_empty() {
                        argv.push("--url".to_string());
                        argv.push(rest_url.clone());
                    } else if !effective_env.trim().is_empty() {
                        argv.push("--env".to_string());
                        argv.push(effective_env.clone());
                    }
                    if !rest_token.trim().is_empty() && access_token_for_case.trim().is_empty() {
                        argv.push("--token".to_string());
                        argv.push(rest_token.clone());
                    }
                    argv.push(path_relative_to_repo_or_abs(repo_root, &request_abs));

                    let args = mask_args_for_command_snippet(&argv[1..]);
                    let env_prefix = if !access_token_for_case.trim().is_empty() {
                        "ACCESS_TOKEN=REDACTED REST_TOKEN_NAME= GQL_JWT_NAME="
                    } else {
                        ""
                    };
                    let snippet = if env_prefix.is_empty() {
                        format!("{} {}", shell_quote("api-rest"), args)
                    } else {
                        format!("{env_prefix} {} {}", shell_quote("api-rest"), args)
                    };
                    command_snippet = Some(snippet.trim().to_string());
                }
            }
            "rest-flow" | "rest_flow" => {
                let login_rel = c.login_request.trim();
                let request_rel = c.request.trim();
                if login_rel.is_empty() {
                    anyhow::bail!("rest-flow case '{id}' is missing loginRequest");
                }
                if request_rel.is_empty() {
                    anyhow::bail!("rest-flow case '{id}' is missing request");
                }
                let login_abs = resolve_path_from_repo_root(repo_root, login_rel);
                let request_abs = resolve_path_from_repo_root(repo_root, request_rel);
                if !login_abs.is_file() {
                    anyhow::bail!("rest-flow case '{id}' loginRequest not found: {login_rel}");
                }
                if !request_abs.is_file() {
                    anyhow::bail!("rest-flow case '{id}' request not found: {request_rel}");
                }

                rest_config_dir = trim_non_empty(&c.config_dir)
                    .unwrap_or_else(|| defaults.rest.config_dir.clone());
                rest_url = trim_non_empty(&c.url).unwrap_or_else(|| defaults.rest.url.clone());
                if rest_url.trim().is_empty() && !options.env_rest_url.trim().is_empty() {
                    rest_url = options.env_rest_url.clone();
                }

                let token_jq =
                    trim_non_empty(&c.token_jq).unwrap_or_else(default_rest_flow_token_jq);

                let login_request_file = crate::rest::schema::RestRequestFile::load(&login_abs)?;
                let main_request_file = crate::rest::schema::RestRequestFile::load(&request_abs)?;

                if rest_method_is_write(&login_request_file.request.method)
                    || rest_method_is_write(&main_request_file.request.method)
                {
                    if !c.allow_write {
                        status = "failed".to_string();
                        message = Some(MSG_WRITE_CAPABLE_REQUIRES_ALLOW_WRITE_TRUE.to_string());
                        failed += 1;
                    } else if !writes_enabled(options.allow_writes_flag, &effective_env) {
                        status = "skipped".to_string();
                        message = Some(MSG_WRITE_CASES_DISABLED.to_string());
                        skipped += 1;
                    }
                }

                if status != "failed" && status != "skipped" {
                    let stderr_path = run_dir_abs.join(format!("{safe_id}.stderr.log"));
                    write_file(&stderr_path, b"")?;
                    stderr_file_abs = Some(stderr_path.clone());

                    let base_url = match resolve_rest_base_url(
                        repo_root,
                        &rest_config_dir,
                        &rest_url,
                        &effective_env,
                        defaults,
                        &options.env_rest_url,
                    ) {
                        Ok(v) => v,
                        Err(err) => {
                            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                            status = "failed".to_string();
                            message = Some("rest_flow_login_failed".to_string());
                            failed += 1;
                            cases_out.push(case_result(
                                &id,
                                &ty,
                                &tags,
                                &status,
                                start.elapsed(),
                                command_snippet.clone(),
                                message.clone(),
                                assertions.clone(),
                                None,
                                Some(&stderr_path),
                                repo_root,
                            ));
                            if options.fail_fast && status == "failed" {
                                break;
                            }
                            continue;
                        }
                    };

                    // Command snippet (parity intent; uses jq extraction).
                    let mut login_args: Vec<String> = Vec::new();
                    login_args.push("call".to_string());
                    login_args.push("--config-dir".to_string());
                    login_args.push(rest_config_dir.clone());
                    if effective_no_history {
                        login_args.push("--no-history".to_string());
                    }
                    if !rest_url.trim().is_empty() {
                        login_args.push("--url".to_string());
                        login_args.push(rest_url.clone());
                    } else if !effective_env.trim().is_empty() {
                        login_args.push("--env".to_string());
                        login_args.push(effective_env.clone());
                    }
                    login_args.push(path_relative_to_repo_or_abs(repo_root, &login_abs));

                    let mut main_args: Vec<String> = Vec::new();
                    main_args.push("call".to_string());
                    main_args.push("--config-dir".to_string());
                    main_args.push(rest_config_dir.clone());
                    if effective_no_history {
                        main_args.push("--no-history".to_string());
                    }
                    if !rest_url.trim().is_empty() {
                        main_args.push("--url".to_string());
                        main_args.push(rest_url.clone());
                    } else if !effective_env.trim().is_empty() {
                        main_args.push("--env".to_string());
                        main_args.push(effective_env.clone());
                    }
                    main_args.push(path_relative_to_repo_or_abs(repo_root, &request_abs));

                    let login_args_snip = mask_args_for_command_snippet(&login_args);
                    let main_args_snip = mask_args_for_command_snippet(&main_args);
                    let token_expr_q = shell_quote(&token_jq);
                    command_snippet = Some(format!(
                        "ACCESS_TOKEN=\"$(REST_TOKEN_NAME= ACCESS_TOKEN= {} {} | jq -r {token_expr_q})\" REST_TOKEN_NAME= {} {}",
                        shell_quote("api-rest"),
                        login_args_snip,
                        shell_quote("api-rest"),
                        main_args_snip
                    ));

                    // Login request
                    let login_executed = match crate::rest::runner::execute_rest_request(
                        &login_request_file,
                        &base_url,
                        None,
                    ) {
                        Ok(v) => v,
                        Err(err) => {
                            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                            status = "failed".to_string();
                            message = Some("rest_flow_login_failed".to_string());
                            failed += 1;
                            cases_out.push(case_result(
                                &id,
                                &ty,
                                &tags,
                                &status,
                                start.elapsed(),
                                command_snippet,
                                message,
                                assertions,
                                None,
                                Some(&stderr_path),
                                repo_root,
                            ));
                            if options.fail_fast && status == "failed" {
                                break;
                            }
                            continue;
                        }
                    };

                    if let Err(err) = crate::rest::expect::evaluate_main_response(
                        &login_request_file.request,
                        &login_executed,
                    ) {
                        write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                        status = "failed".to_string();
                        message = Some("rest_flow_login_failed".to_string());
                        failed += 1;
                        cases_out.push(case_result(
                            &id,
                            &ty,
                            &tags,
                            &status,
                            start.elapsed(),
                            command_snippet,
                            message,
                            assertions,
                            None,
                            Some(&stderr_path),
                            repo_root,
                        ));
                        if options.fail_fast && status == "failed" {
                            break;
                        }
                        continue;
                    }

                    let login_json: serde_json::Value =
                        match serde_json::from_slice(&login_executed.response.body) {
                            Ok(v) => v,
                            Err(_) => serde_json::Value::Null,
                        };
                    let token = crate::jq::query_raw(&login_json, &token_jq)
                        .ok()
                        .and_then(|lines| lines.into_iter().next())
                        .unwrap_or_default();
                    let token = token.trim().to_string();
                    if token.is_empty() || token == "null" {
                        let hint = "Failed to extract token from login response.\nHint: set cases[i].tokenJq to the token field (e.g. .accessToken).\n";
                        write_file(&stderr_path, hint.as_bytes())?;
                        status = "failed".to_string();
                        message = Some("rest_flow_token_extract_failed".to_string());
                        failed += 1;
                        cases_out.push(case_result(
                            &id,
                            &ty,
                            &tags,
                            &status,
                            start.elapsed(),
                            command_snippet,
                            message,
                            assertions,
                            None,
                            Some(&stderr_path),
                            repo_root,
                        ));
                        if options.fail_fast && status == "failed" {
                            break;
                        }
                        continue;
                    }

                    // Main request with extracted token
                    let stdout_path = run_dir_abs.join(format!("{safe_id}.response.json"));
                    write_file(&stdout_path, b"")?;
                    stdout_file_abs = Some(stdout_path.clone());

                    match crate::rest::runner::execute_rest_request(
                        &main_request_file,
                        &base_url,
                        Some(&token),
                    ) {
                        Ok(executed) => {
                            write_file(&stdout_path, &executed.response.body)?;
                            if let Err(err) = crate::rest::expect::evaluate_main_response(
                                &main_request_file.request,
                                &executed,
                            ) {
                                write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                status = "failed".to_string();
                                message = Some("rest_flow_request_failed".to_string());
                                failed += 1;
                            } else {
                                status = "passed".to_string();
                                passed += 1;
                            }
                        }
                        Err(err) => {
                            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                            status = "failed".to_string();
                            message = Some("rest_flow_request_failed".to_string());
                            failed += 1;
                        }
                    }
                }
            }
            "graphql" => {
                let op_rel = c.op.trim();
                if op_rel.is_empty() {
                    anyhow::bail!("GraphQL case '{id}' is missing op");
                }
                let op_abs = resolve_path_from_repo_root(repo_root, op_rel);
                if !op_abs.is_file() {
                    anyhow::bail!("GraphQL case '{id}' op not found: {op_rel}");
                }

                let vars_abs = c
                    .vars
                    .as_deref()
                    .and_then(trim_non_empty)
                    .map(|p| resolve_path_from_repo_root(repo_root, &p));
                if let Some(vp) = vars_abs.as_deref() {
                    if !vp.is_file() {
                        anyhow::bail!("GraphQL case '{id}' vars not found: {}", vp.display());
                    }
                }

                gql_config_dir = trim_non_empty(&c.config_dir)
                    .unwrap_or_else(|| defaults.graphql.config_dir.clone());
                gql_jwt = trim_non_empty(&c.jwt).unwrap_or_else(|| defaults.graphql.jwt.clone());
                gql_url = trim_non_empty(&c.url).unwrap_or_else(|| defaults.graphql.url.clone());
                if gql_url.trim().is_empty() && !options.env_gql_url.trim().is_empty() {
                    gql_url = options.env_gql_url.clone();
                }

                match graphql_safety_decision(
                    &op_abs,
                    c.allow_write,
                    options.allow_writes_flag,
                    &effective_env,
                )? {
                    crate::suite::safety::SafetyDecision::Fail(msg) => {
                        status = "failed".to_string();
                        message = Some(msg.to_string());
                        failed += 1;
                    }
                    crate::suite::safety::SafetyDecision::Skip(msg) => {
                        status = "skipped".to_string();
                        message = Some(msg.to_string());
                        skipped += 1;
                    }
                    crate::suite::safety::SafetyDecision::Allow => {}
                }

                if status != "failed" && status != "skipped" {
                    // Auth injection (optional)
                    if let Some(mgr) = auth_manager.as_mut() {
                        if !gql_jwt.trim().is_empty() {
                            match mgr.ensure_token(
                                &gql_jwt,
                                repo_root,
                                defaults,
                                &options.env_rest_url,
                                &options.env_gql_url,
                            ) {
                                Ok(t) => access_token_for_case = t,
                                Err(err) => {
                                    status = "failed".to_string();
                                    message = Some(err);
                                    failed += 1;
                                }
                            }
                        }
                    }
                }

                if status != "failed" && status != "skipped" {
                    let stdout_path = run_dir_abs.join(format!("{safe_id}.response.json"));
                    let stderr_path = run_dir_abs.join(format!("{safe_id}.stderr.log"));
                    write_file(&stdout_path, b"")?;
                    write_file(&stderr_path, b"")?;
                    stdout_file_abs = Some(stdout_path.clone());
                    stderr_file_abs = Some(stderr_path.clone());

                    let endpoint_url = match resolve_gql_url(
                        repo_root,
                        &gql_config_dir,
                        &gql_url,
                        &effective_env,
                        defaults,
                        &options.env_gql_url,
                    ) {
                        Ok(v) => v,
                        Err(err) => {
                            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                            status = "failed".to_string();
                            message = Some("graphql_runner_failed".to_string());
                            failed += 1;
                            String::new()
                        }
                    };

                    let vars_min_limit =
                        parse_u64_default(std::env::var("GQL_VARS_MIN_LIMIT").ok(), 5, 0);
                    let vars_json = match vars_abs.as_deref() {
                        None => None,
                        Some(path) => match crate::graphql::vars::GraphqlVariablesFile::load(
                            path,
                            vars_min_limit,
                        ) {
                            Ok(v) => Some(v.variables),
                            Err(err) => {
                                write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                status = "failed".to_string();
                                message = Some("graphql_runner_failed".to_string());
                                failed += 1;
                                None
                            }
                        },
                    };

                    let mut auth_stderr: Vec<u8> = Vec::new();
                    let setup_dir_abs = resolve_path_from_repo_root(repo_root, &gql_config_dir);
                    let bearer = if status == "failed" {
                        None
                    } else if !access_token_for_case.trim().is_empty() {
                        Some(access_token_for_case.clone())
                    } else if !gql_jwt.trim().is_empty() {
                        match resolve_graphql_bearer_token(
                            &setup_dir_abs,
                            &endpoint_url,
                            &op_abs,
                            &gql_jwt,
                            &mut auth_stderr,
                        ) {
                            Ok(v) => v,
                            Err(err) => {
                                write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                status = "failed".to_string();
                                message = Some("graphql_runner_failed".to_string());
                                failed += 1;
                                None
                            }
                        }
                    } else {
                        None
                    };

                    if !auth_stderr.is_empty() {
                        write_file(&stderr_path, &auth_stderr)?;
                    }

                    let op_file = match crate::graphql::schema::GraphqlOperationFile::load(&op_abs)
                    {
                        Ok(v) => v,
                        Err(err) => {
                            write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                            status = "failed".to_string();
                            message = Some("graphql_runner_failed".to_string());
                            failed += 1;
                            cases_out.push(SuiteCaseResult {
                                id: id.clone(),
                                case_type: ty.clone(),
                                status: status.clone(),
                                duration_ms: start
                                    .elapsed()
                                    .as_millis()
                                    .try_into()
                                    .unwrap_or(u64::MAX),
                                tags: tags.clone(),
                                command: command_snippet.clone(),
                                message: message.clone(),
                                assertions: assertions.clone(),
                                stdout_file: Some(path_relative_to_repo_or_abs(
                                    repo_root,
                                    &stdout_path,
                                )),
                                stderr_file: Some(path_relative_to_repo_or_abs(
                                    repo_root,
                                    &stderr_path,
                                )),
                            });
                            if options.fail_fast && status == "failed" {
                                break;
                            }
                            continue;
                        }
                    };

                    if status == "failed" {
                        // already recorded
                    } else {
                        match crate::graphql::runner::execute_graphql_request(
                            &endpoint_url,
                            bearer.as_deref(),
                            &op_file.operation,
                            vars_json.as_ref(),
                        ) {
                            Ok(executed) => {
                                write_file(&stdout_path, &executed.response.body)?;
                                let response_json: serde_json::Value =
                                    serde_json::from_slice(&executed.response.body)
                                        .unwrap_or(serde_json::Value::Null);

                                let expect_jq =
                                    c.expect.as_ref().map(|e| e.jq.trim()).unwrap_or("");
                                let expect_jq = (!expect_jq.is_empty()).then_some(expect_jq);

                                let a =
                                    crate::graphql::expect::evaluate_graphql_response_for_suite(
                                        &response_json,
                                        c.allow_errors,
                                        expect_jq,
                                    )
                                    .unwrap_or(
                                        crate::graphql::expect::GraphqlAssertions {
                                            default_no_errors: "failed".to_string(),
                                            default_has_data: None,
                                            jq: None,
                                        },
                                    );
                                assertions = Some(a.to_json());

                                let jq_failed = a.jq.as_deref() == Some("failed");
                                let has_data_failed =
                                    a.default_has_data.as_deref() == Some("failed");
                                let no_errors_failed = a.default_no_errors == "failed";

                                if no_errors_failed && !c.allow_errors {
                                    status = "failed".to_string();
                                    message = Some("graphql_errors_present".to_string());
                                    failed += 1;
                                } else if has_data_failed {
                                    status = "failed".to_string();
                                    message = Some("graphql_data_missing_or_null".to_string());
                                    failed += 1;
                                } else if jq_failed {
                                    status = "failed".to_string();
                                    message = Some("expect_jq_failed".to_string());
                                    failed += 1;
                                } else {
                                    status = "passed".to_string();
                                    passed += 1;
                                }
                            }
                            Err(err) => {
                                write_file(&stderr_path, format!("{err:#}\n").as_bytes())?;
                                status = "failed".to_string();
                                message = Some("graphql_runner_failed".to_string());
                                failed += 1;
                            }
                        }
                    }

                    let mut argv: Vec<String> = vec![
                        "api-gql".to_string(),
                        "call".to_string(),
                        "--config-dir".to_string(),
                        gql_config_dir.clone(),
                    ];
                    if effective_no_history {
                        argv.push("--no-history".to_string());
                    }
                    if !gql_url.trim().is_empty() {
                        argv.push("--url".to_string());
                        argv.push(gql_url.clone());
                    } else if !effective_env.trim().is_empty() {
                        argv.push("--env".to_string());
                        argv.push(effective_env.clone());
                    }
                    if !gql_jwt.trim().is_empty() && access_token_for_case.trim().is_empty() {
                        argv.push("--jwt".to_string());
                        argv.push(gql_jwt.clone());
                    }
                    argv.push(path_relative_to_repo_or_abs(repo_root, &op_abs));
                    if let Some(vp) = vars_abs.as_deref() {
                        argv.push(path_relative_to_repo_or_abs(repo_root, vp));
                    }

                    let args = mask_args_for_command_snippet(&argv[1..]);
                    let env_prefix = if !access_token_for_case.trim().is_empty() {
                        "ACCESS_TOKEN=REDACTED REST_TOKEN_NAME= GQL_JWT_NAME="
                    } else {
                        ""
                    };
                    let snippet = if env_prefix.is_empty() {
                        format!("{} {}", shell_quote("api-gql"), args)
                    } else {
                        format!("{env_prefix} {} {}", shell_quote("api-gql"), args)
                    };
                    command_snippet = Some(snippet.trim().to_string());
                }
            }
            other => {
                anyhow::bail!("Unknown case type '{other}' for case '{id}'");
            }
        }

        let duration_ms = start.elapsed().as_millis().try_into().unwrap_or(u64::MAX);

        // Cleanup hook (best-effort; can flip passed->failed).
        if status == "passed" || status == "failed" {
            let cleanup_ok = if let (Some(stderr_path), Some(cleanup)) =
                (stderr_file_abs.as_deref(), c.cleanup.as_ref())
            {
                let mut ctx = CleanupContext {
                    repo_root,
                    run_dir: &run_dir_abs,
                    case_id: &id,
                    safe_id: &safe_id,
                    main_response_file: stdout_file_abs.as_deref(),
                    main_stderr_file: stderr_path,
                    allow_writes_flag: options.allow_writes_flag,
                    effective_env: &effective_env,
                    effective_no_history,
                    suite_defaults: defaults,
                    env_rest_url: &options.env_rest_url,
                    env_gql_url: &options.env_gql_url,
                    rest_config_dir: &rest_config_dir,
                    rest_url: &rest_url,
                    rest_token: &rest_token,
                    gql_config_dir: &gql_config_dir,
                    gql_url: &gql_url,
                    gql_jwt: &gql_jwt,
                    access_token_for_case: &access_token_for_case,
                    auth_manager: auth_manager.as_mut(),
                    cleanup: Some(cleanup),
                };
                run_case_cleanup(&mut ctx)?
            } else {
                true
            };

            if !cleanup_ok && status == "passed" {
                status = "failed".to_string();
                message = Some("cleanup_failed".to_string());
                passed = passed.saturating_sub(1);
                failed += 1;
            }
        }

        if status == "pending" {
            status = "failed".to_string();
            message = Some("internal_error".to_string());
            failed += 1;
        }

        cases_out.push(SuiteCaseResult {
            id,
            case_type: ty,
            status: status.clone(),
            duration_ms,
            tags,
            command: command_snippet,
            message,
            assertions,
            stdout_file: stdout_file_abs
                .as_deref()
                .map(|p| path_relative_to_repo_or_abs(repo_root, p)),
            stderr_file: stderr_file_abs
                .as_deref()
                .map(|p| path_relative_to_repo_or_abs(repo_root, p)),
        });

        if options.fail_fast && status == "failed" {
            break;
        }
    }

    let finished_at = time_iso_now()?;

    let results = SuiteRunResults {
        version: 1,
        suite: suite_name_value,
        suite_file: suite_file_rel,
        run_id,
        started_at,
        finished_at,
        output_dir: output_dir_rel,
        summary: SuiteRunSummary {
            total,
            passed,
            failed,
            skipped,
        },
        cases: cases_out,
    };

    if let Some(msg) = auth_init_message {
        // Best-effort: write to runner stderr file in output dir, to avoid losing context.
        let path = run_dir_abs.join("auth.disabled.log");
        let _ = write_file(&path, format!("{msg}\n").as_bytes());
    }

    Ok(SuiteRunOutput {
        run_dir_abs,
        results,
    })
}

fn resolve_graphql_bearer_token(
    setup_dir: &Path,
    endpoint_url: &str,
    operation_file: &Path,
    jwt_profile: &str,
    stderr: &mut dyn Write,
) -> Result<Option<String>> {
    if jwt_profile.trim().is_empty() {
        return Ok(None);
    }
    let auth = crate::graphql::auth::resolve_bearer_token(
        setup_dir,
        endpoint_url,
        Some(operation_file),
        Some(jwt_profile),
        stderr,
    )?;
    Ok(auth.bearer_token)
}

#[allow(clippy::too_many_arguments)]
fn case_result(
    id: &str,
    case_type: &str,
    tags: &[String],
    status: &str,
    duration: std::time::Duration,
    command: Option<String>,
    message: Option<String>,
    assertions: Option<serde_json::Value>,
    stdout_file: Option<&Path>,
    stderr_file: Option<&Path>,
    repo_root: &Path,
) -> SuiteCaseResult {
    SuiteCaseResult {
        id: id.to_string(),
        case_type: case_type.to_string(),
        status: status.to_string(),
        duration_ms: duration.as_millis().try_into().unwrap_or(u64::MAX),
        tags: tags.to_vec(),
        command,
        message,
        assertions,
        stdout_file: stdout_file.map(|p| path_relative_to_repo_or_abs(repo_root, p)),
        stderr_file: stderr_file.map(|p| path_relative_to_repo_or_abs(repo_root, p)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn suite_runner_sanitize_id_matches_expected_replacements() {
        assert_eq!(sanitize_id("rest.health"), "rest.health");
        assert_eq!(sanitize_id("a b c"), "a-b-c");
        assert_eq!(sanitize_id(""), "case");
    }
}
