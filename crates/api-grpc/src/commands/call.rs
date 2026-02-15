use std::io::Write;
use std::path::{Path, PathBuf};

use api_testing_core::{Result, auth_env, cli_endpoint, cli_util, config, env_file, history, jwt};
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};

use crate::cli::CallArgs;
use api_testing_core::cli_util::{
    history_timestamp_now, list_available_suffixes, maybe_relpath, parse_u64_default, shell_quote,
    to_env_key, trim_non_empty,
};

#[derive(Debug, Clone)]
pub(crate) struct EndpointSelection {
    pub(crate) grpc_target: String,
    pub(crate) endpoint_label_used: String,
    pub(crate) endpoint_value_used: String,
}

#[derive(Debug, Clone)]
pub(crate) enum AuthSourceUsed {
    None,
    TokenProfile,
    EnvFallback { env_name: String },
}

#[derive(Debug, Clone)]
pub(crate) struct AuthSelection {
    pub(crate) bearer_token: Option<String>,
    pub(crate) token_name: String,
    pub(crate) auth_source_used: AuthSourceUsed,
}

pub(crate) fn resolve_endpoint_for_call(
    args: &CallArgs,
    setup: &api_testing_core::config::ResolvedSetup,
) -> Result<EndpointSelection> {
    let endpoints_env = &setup.endpoints_env;
    let endpoints_local = &setup.endpoints_local_env;
    let endpoints_files = setup.endpoints_files();

    let selection = cli_endpoint::resolve_cli_endpoint(cli_endpoint::EndpointConfig {
        explicit_url: args.url.as_deref(),
        env_name: args.env.as_deref(),
        endpoints_env,
        endpoints_local,
        endpoints_files: &endpoints_files,
        url_env_var: "GRPC_URL",
        env_default_var: "GRPC_ENV_DEFAULT",
        url_prefix: "GRPC_URL_",
        default_url: "127.0.0.1:50051",
        setup_dir_label: "setup/grpc/",
    })?;

    Ok(EndpointSelection {
        grpc_target: selection.url,
        endpoint_label_used: selection.endpoint_label_used,
        endpoint_value_used: selection.endpoint_value_used,
    })
}

pub(crate) fn resolve_auth_for_call(
    args: &CallArgs,
    setup: &api_testing_core::config::ResolvedSetup,
) -> Result<AuthSelection> {
    let tokens_env = setup.tokens_env.as_ref().expect("tokens_env");
    let tokens_local = setup.tokens_local_env.as_ref().expect("tokens_local_env");
    let tokens_files = setup.tokens_files();

    let token_name_arg = args.token.as_deref().and_then(trim_non_empty);
    let token_name_env = std::env::var("GRPC_TOKEN_NAME")
        .ok()
        .and_then(|s| trim_non_empty(&s));
    let token_name_file = if !tokens_files.is_empty() {
        env_file::read_var_last_wins("GRPC_TOKEN_NAME", &tokens_files)?
    } else {
        None
    };

    let token_profile_selected =
        token_name_arg.is_some() || token_name_env.is_some() || token_name_file.is_some();
    let token_name = token_name_arg
        .or(token_name_env)
        .or(token_name_file)
        .unwrap_or_else(|| "default".to_string())
        .to_ascii_lowercase();

    if token_profile_selected {
        let token_key = to_env_key(&token_name);
        let token_var = format!("GRPC_TOKEN_{token_key}");
        let bearer_token = env_file::read_var_last_wins(&token_var, &tokens_files)?;
        let Some(bearer_token) = bearer_token else {
            let mut available = list_available_suffixes(tokens_env, "GRPC_TOKEN_");
            if tokens_local.is_file() {
                available.extend(list_available_suffixes(tokens_local, "GRPC_TOKEN_"));
                available.sort();
                available.dedup();
            }
            available.retain(|t| t != "name");
            let available = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(" ")
            };
            anyhow::bail!(
                "Token profile '{token_name}' is empty/missing (available: {available}). Set it in setup/grpc/tokens.local.env or use ACCESS_TOKEN without selecting a token profile."
            );
        };

        return Ok(AuthSelection {
            bearer_token: Some(bearer_token),
            token_name: token_name.clone(),
            auth_source_used: AuthSourceUsed::TokenProfile,
        });
    }

    if let Some((token, env_name)) =
        auth_env::resolve_env_fallback(&["ACCESS_TOKEN", "SERVICE_TOKEN"])
    {
        return Ok(AuthSelection {
            bearer_token: Some(token),
            token_name,
            auth_source_used: AuthSourceUsed::EnvFallback { env_name },
        });
    }

    Ok(AuthSelection {
        bearer_token: None,
        token_name,
        auth_source_used: AuthSourceUsed::None,
    })
}

pub(crate) fn validate_bearer_token_if_jwt(
    bearer_token: &str,
    auth_source: &AuthSourceUsed,
    token_name: &str,
    stderr: &mut dyn Write,
) -> Result<()> {
    let enabled = cli_util::bool_from_env(
        std::env::var("GRPC_JWT_VALIDATE_ENABLED").ok(),
        "GRPC_JWT_VALIDATE_ENABLED",
        true,
        Some("api-grpc"),
        stderr,
    );
    let strict = cli_util::bool_from_env(
        std::env::var("GRPC_JWT_VALIDATE_STRICT").ok(),
        "GRPC_JWT_VALIDATE_STRICT",
        false,
        Some("api-grpc"),
        stderr,
    );
    let leeway_seconds =
        parse_u64_default(std::env::var("GRPC_JWT_VALIDATE_LEEWAY_SECONDS").ok(), 0, 0);

    let label = match auth_source {
        AuthSourceUsed::TokenProfile => format!("token profile '{token_name}'"),
        AuthSourceUsed::EnvFallback { env_name } => env_name.to_string(),
        AuthSourceUsed::None => "token".to_string(),
    };

    let opts = jwt::JwtValidationOptions {
        enabled,
        strict,
        leeway_seconds: i64::try_from(leeway_seconds).unwrap_or(i64::MAX),
    };

    match jwt::check_bearer_jwt(bearer_token, &label, opts)? {
        jwt::JwtCheck::Ok => Ok(()),
        jwt::JwtCheck::Warn(msg) => {
            let _ = writeln!(stderr, "api-grpc: warning: {msg}");
            Ok(())
        }
    }
}

pub(crate) fn cmd_call(
    args: &CallArgs,
    invocation_dir: &Path,
    stdout_is_tty: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    cmd_call_internal(args, invocation_dir, stdout_is_tty, true, stdout, stderr)
}

pub(crate) fn cmd_call_internal(
    args: &CallArgs,
    invocation_dir: &Path,
    stdout_is_tty: bool,
    history_enabled_by_command: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let request_path = PathBuf::from(&args.request);
    if !request_path.is_file() {
        let _ = writeln!(stderr, "Request file not found: {}", request_path.display());
        return 1;
    }

    let request_file = match api_testing_core::grpc::schema::GrpcRequestFile::load(&request_path) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };

    let config_dir = args
        .config_dir
        .as_deref()
        .and_then(trim_non_empty)
        .map(PathBuf::from);
    let setup_dir = match config::resolve_grpc_setup_dir_for_call(
        invocation_dir,
        invocation_dir,
        &request_path,
        config_dir.as_deref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };

    let mut exit_code = 1;

    let history_enabled = history_enabled_by_command
        && !args.no_history
        && cli_util::bool_from_env(
            std::env::var("GRPC_HISTORY_ENABLED").ok(),
            "GRPC_HISTORY_ENABLED",
            true,
            Some("api-grpc"),
            stderr,
        );

    let history_file_override = std::env::var("GRPC_HISTORY_FILE")
        .ok()
        .and_then(|s| trim_non_empty(&s))
        .map(PathBuf::from);

    let setup = api_testing_core::config::ResolvedSetup::grpc(
        setup_dir.clone(),
        history_file_override.as_deref(),
    );

    let rotation = history::RotationPolicy {
        max_mb: parse_u64_default(std::env::var("GRPC_HISTORY_MAX_MB").ok(), 10, 0),
        keep: parse_u64_default(std::env::var("GRPC_HISTORY_ROTATE_COUNT").ok(), 5, 1)
            .try_into()
            .unwrap_or(u32::MAX),
    };

    let log_url = cli_util::bool_from_env(
        std::env::var("GRPC_HISTORY_LOG_URL_ENABLED").ok(),
        "GRPC_HISTORY_LOG_URL_ENABLED",
        true,
        Some("api-grpc"),
        stderr,
    );

    let history_writer = history::HistoryWriter::new(setup.history_file.clone(), rotation);

    let mut history_ctx = CallHistoryContext {
        enabled: history_enabled,
        setup_dir: setup_dir.clone(),
        history_writer,
        invocation_dir: invocation_dir.to_path_buf(),
        request_arg: args.request.clone(),
        endpoint_label_used: String::new(),
        endpoint_value_used: String::new(),
        log_url,
        auth_source_used: AuthSourceUsed::None,
        token_name_for_log: String::new(),
    };

    let endpoint = match resolve_endpoint_for_call(args, &setup) {
        Ok(v) => {
            history_ctx.endpoint_label_used = v.endpoint_label_used.clone();
            history_ctx.endpoint_value_used = v.endpoint_value_used.clone();
            v
        }
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            append_history_best_effort(&history_ctx, exit_code);
            return 1;
        }
    };

    let auth = match resolve_auth_for_call(args, &setup) {
        Ok(v) => {
            history_ctx.auth_source_used = v.auth_source_used.clone();
            history_ctx.token_name_for_log = v.token_name.clone();
            v
        }
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            append_history_best_effort(&history_ctx, exit_code);
            return 1;
        }
    };

    if let Some(token) = auth.bearer_token.as_deref()
        && let Err(err) = validate_bearer_token_if_jwt(
            token,
            &history_ctx.auth_source_used,
            &auth.token_name,
            stderr,
        )
    {
        let _ = writeln!(stderr, "{err}");
        append_history_best_effort(&history_ctx, exit_code);
        return 1;
    }

    let spinner = Progress::spinner(
        ProgressOptions::default()
            .with_prefix("api-grpc ")
            .with_finish(ProgressFinish::Clear),
    );
    spinner.set_message("request");
    spinner.tick();

    let executed = match api_testing_core::grpc::runner::execute_grpc_request(
        &request_file,
        &endpoint.grpc_target,
        auth.bearer_token.as_deref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            spinner.finish_and_clear();
            let _ = writeln!(stderr, "{err}");
            append_history_best_effort(&history_ctx, exit_code);
            return 1;
        }
    };

    let _ = stdout.write_all(&executed.response_body);

    if let Err(err) =
        api_testing_core::grpc::expect::evaluate_main_response(&request_file.request, &executed)
    {
        spinner.finish_and_clear();
        let _ = writeln!(stderr, "{err}");
        maybe_print_failure_body_to_stderr(&executed.response_body, 8192, stdout_is_tty, stderr);
        append_history_best_effort(&history_ctx, exit_code);
        return 1;
    }

    spinner.finish_and_clear();
    exit_code = 0;
    append_history_best_effort(&history_ctx, exit_code);

    exit_code
}

#[derive(Debug, Clone)]
struct CallHistoryContext {
    enabled: bool,
    setup_dir: PathBuf,
    history_writer: history::HistoryWriter,
    invocation_dir: PathBuf,
    request_arg: String,
    endpoint_label_used: String,
    endpoint_value_used: String,
    log_url: bool,
    auth_source_used: AuthSourceUsed,
    token_name_for_log: String,
}

fn append_history_best_effort(ctx: &CallHistoryContext, exit_code: i32) {
    if !ctx.enabled {
        return;
    }

    let history_writer = &ctx.history_writer;

    let stamp = history_timestamp_now().unwrap_or_default();
    let setup_rel = maybe_relpath(&ctx.setup_dir, &ctx.invocation_dir);

    let mut record = String::new();
    record.push_str(&format!("# {stamp} exit={exit_code} setup_dir={setup_rel}"));
    if !ctx.endpoint_label_used.is_empty() {
        if ctx.endpoint_label_used == "url" && !ctx.log_url {
            record.push_str(" url=<omitted>");
        } else {
            record.push_str(&format!(
                " {}={}",
                ctx.endpoint_label_used, ctx.endpoint_value_used
            ));
        }
    }

    match &ctx.auth_source_used {
        AuthSourceUsed::TokenProfile => {
            if !ctx.token_name_for_log.is_empty() {
                record.push_str(&format!(" token={}", ctx.token_name_for_log));
            }
        }
        AuthSourceUsed::EnvFallback { env_name } => {
            if !env_name.is_empty() {
                record.push_str(&format!(" auth={env_name}"));
            }
        }
        AuthSourceUsed::None => {}
    }

    record.push('\n');

    let config_rel = maybe_relpath(&ctx.setup_dir, &ctx.invocation_dir);
    let req_arg_path = Path::new(&ctx.request_arg);
    let req_rel = if req_arg_path.is_absolute() {
        maybe_relpath(req_arg_path, &ctx.invocation_dir)
    } else {
        ctx.request_arg.clone()
    };

    record.push_str("api-grpc call \\\n");
    record.push_str(&format!("  --config-dir {} \\\n", shell_quote(&config_rel)));

    if ctx.endpoint_label_used == "env" && !ctx.endpoint_value_used.is_empty() {
        record.push_str(&format!(
            "  --env {} \\\n",
            shell_quote(&ctx.endpoint_value_used)
        ));
    } else if ctx.endpoint_label_used == "url" && !ctx.endpoint_value_used.is_empty() && ctx.log_url
    {
        record.push_str(&format!(
            "  --url {} \\\n",
            shell_quote(&ctx.endpoint_value_used)
        ));
    }

    if matches!(ctx.auth_source_used, AuthSourceUsed::TokenProfile)
        && !ctx.token_name_for_log.is_empty()
    {
        record.push_str(&format!(
            "  --token {} \\\n",
            shell_quote(&ctx.token_name_for_log)
        ));
    }

    record.push_str(&format!("  {} \\\n", shell_quote(&req_rel)));
    record.push_str("| jq .\n\n");

    let _ = history_writer.append(&record);
}

fn maybe_print_failure_body_to_stderr(
    body: &[u8],
    max_bytes: usize,
    stdout_is_tty: bool,
    stderr: &mut dyn Write,
) {
    if stdout_is_tty || body.is_empty() {
        return;
    }

    if serde_json::from_slice::<serde_json::Value>(body).is_ok() {
        return;
    }

    let _ = writeln!(stderr, "Response body (non-JSON; first {max_bytes} bytes):");
    let _ = stderr.write_all(&body[..body.len().min(max_bytes)]);
    let _ = writeln!(stderr);
}
