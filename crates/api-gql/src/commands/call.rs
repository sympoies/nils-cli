use std::io::Write;
use std::path::{Path, PathBuf};

use api_testing_core::{config, env_file, history, Result};
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};

use crate::cli::CallArgs;
use crate::util::{
    bool_from_env, history_timestamp_now, list_available_suffixes, maybe_relpath,
    parse_u64_default, shell_quote, to_env_key, trim_non_empty,
};

#[derive(Debug, Clone)]
pub(crate) struct EndpointSelection {
    pub(crate) gql_url: String,
    pub(crate) endpoint_label_used: String,
    pub(crate) endpoint_value_used: String,
}

pub(crate) fn resolve_endpoint_for_call(
    args: &CallArgs,
    setup: &api_testing_core::config::ResolvedSetup,
) -> Result<EndpointSelection> {
    let endpoints_env = &setup.endpoints_env;
    let endpoints_local = &setup.endpoints_local_env;
    let endpoints_files = setup.endpoints_files();

    let gql_env_default = if !endpoints_files.is_empty() {
        env_file::read_var_last_wins("GQL_ENV_DEFAULT", &endpoints_files)?
    } else {
        None
    };

    let env_name = args.env.as_deref().and_then(trim_non_empty);
    let explicit_url = args.url.as_deref().and_then(trim_non_empty);

    let (gql_url, endpoint_label_used, endpoint_value_used) = if let Some(url) = explicit_url {
        (url.clone(), "url".to_string(), url)
    } else if let Some(env_value) = env_name.as_deref() {
        if env_value.starts_with("http://") || env_value.starts_with("https://") {
            (
                env_value.to_string(),
                "url".to_string(),
                env_value.to_string(),
            )
        } else {
            if endpoints_files.is_empty() {
                anyhow::bail!("endpoints.env not found (expected under setup/graphql/)");
            }

            let env_key = to_env_key(env_value);
            let key = format!("GQL_URL_{env_key}");
            let found = env_file::read_var_last_wins(&key, &endpoints_files)?;
            let Some(found) = found else {
                let mut available = list_available_suffixes(endpoints_env, "GQL_URL_");
                if endpoints_local.is_file() {
                    available.extend(list_available_suffixes(endpoints_local, "GQL_URL_"));
                    available.sort();
                    available.dedup();
                }
                let available = if available.is_empty() {
                    "none".to_string()
                } else {
                    available.join(" ")
                };
                anyhow::bail!("Unknown --env '{env_value}' (available: {available})");
            };

            (found, "env".to_string(), env_value.to_string())
        }
    } else if let Some(v) = std::env::var("GQL_URL")
        .ok()
        .and_then(|s| trim_non_empty(&s))
    {
        (v.clone(), "url".to_string(), v)
    } else if let Some(default_env) = gql_env_default {
        if endpoints_files.is_empty() {
            anyhow::bail!("GQL_ENV_DEFAULT is set but endpoints.env not found (expected under setup/graphql/)");
        }
        let env_key = to_env_key(&default_env);
        let key = format!("GQL_URL_{env_key}");
        let found = env_file::read_var_last_wins(&key, &endpoints_files)?;
        let Some(found) = found else {
            anyhow::bail!(
                "GQL_ENV_DEFAULT is '{}' but no matching GQL_URL_* was found.",
                default_env
            );
        };
        (found, "env".to_string(), default_env)
    } else {
        let gql_url = "http://localhost:6700/graphql".to_string();
        (gql_url.clone(), "url".to_string(), gql_url)
    };

    Ok(EndpointSelection {
        gql_url,
        endpoint_label_used,
        endpoint_value_used,
    })
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
    _stdout_is_tty: bool,
    history_enabled_by_command: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let config_dir = args
        .config_dir
        .as_deref()
        .and_then(trim_non_empty)
        .map(PathBuf::from);

    let op_path = args.operation.as_deref().map(PathBuf::from);
    let vars_path = args.variables.as_deref().map(PathBuf::from);

    let setup_dir = match config::resolve_gql_setup_dir_for_call(
        invocation_dir,
        invocation_dir,
        op_path.as_deref(),
        config_dir.as_deref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };

    let history_file_override = std::env::var("GQL_HISTORY_FILE")
        .ok()
        .and_then(|s| trim_non_empty(&s))
        .map(PathBuf::from);
    let setup = api_testing_core::config::ResolvedSetup::graphql(
        setup_dir.clone(),
        history_file_override.as_deref(),
    );

    if args.list_envs {
        let endpoints_env = &setup.endpoints_env;
        let endpoints_local = &setup.endpoints_local_env;
        if !endpoints_env.is_file() {
            let _ = writeln!(
                stderr,
                "endpoints.env not found (expected under setup/graphql/)"
            );
            return 1;
        }
        let mut out = list_available_suffixes(endpoints_env, "GQL_URL_");
        if endpoints_local.is_file() {
            out.extend(list_available_suffixes(endpoints_local, "GQL_URL_"));
            out.sort();
            out.dedup();
        }
        for v in out {
            let _ = writeln!(stdout, "{v}");
        }
        return 0;
    }

    if args.list_jwts {
        let jwts_env = setup.jwts_env.as_ref().expect("jwts_env");
        let jwts_local = setup.jwts_local_env.as_ref().expect("jwts_local_env");
        if !jwts_env.is_file() && !jwts_local.is_file() {
            let _ = writeln!(
                stderr,
                "jwts(.local).env not found (expected under setup/graphql/)"
            );
            return 1;
        }
        let mut out = list_available_suffixes(jwts_env, "GQL_JWT_");
        if jwts_local.is_file() {
            out.extend(list_available_suffixes(jwts_local, "GQL_JWT_"));
            out.sort();
            out.dedup();
        }
        out.retain(|t| t != "name");
        for v in out {
            let _ = writeln!(stdout, "{v}");
        }
        return 0;
    }

    let Some(op_path) = op_path else {
        let _ = writeln!(stderr, "error: missing operation file");
        return 1;
    };
    if !op_path.is_file() {
        let _ = writeln!(stderr, "Operation file not found: {}", op_path.display());
        return 1;
    }
    if let Some(vars_path) = vars_path.as_deref() {
        if !vars_path.is_file() {
            let _ = writeln!(stderr, "Variables file not found: {}", vars_path.display());
            return 1;
        }
    }

    let mut exit_code = 1;

    let history_enabled = history_enabled_by_command
        && !args.no_history
        && bool_from_env(
            std::env::var("GQL_HISTORY_ENABLED").ok(),
            "GQL_HISTORY_ENABLED",
            true,
            stderr,
        );

    let rotation = history::RotationPolicy {
        max_mb: parse_u64_default(std::env::var("GQL_HISTORY_MAX_MB").ok(), 10, 0),
        keep: parse_u64_default(std::env::var("GQL_HISTORY_ROTATE_COUNT").ok(), 5, 1)
            .try_into()
            .unwrap_or(u32::MAX),
    };

    let log_url = bool_from_env(
        std::env::var("GQL_HISTORY_LOG_URL_ENABLED").ok(),
        "GQL_HISTORY_LOG_URL_ENABLED",
        true,
        stderr,
    );

    let history_writer = history::HistoryWriter::new(setup.history_file.clone(), rotation);

    let mut history_ctx = CallHistoryContext {
        enabled: history_enabled,
        setup_dir: setup_dir.clone(),
        history_writer,
        invocation_dir: invocation_dir.to_path_buf(),
        op_arg: args.operation.clone().unwrap_or_default(),
        vars_arg: args.variables.clone(),
        endpoint_label_used: String::new(),
        endpoint_value_used: String::new(),
        log_url,
        auth_source_used: api_testing_core::graphql::auth::GraphqlAuthSourceUsed::None,
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

    let auth = match api_testing_core::graphql::auth::resolve_bearer_token(
        &setup.setup_dir,
        &endpoint.gql_url,
        Some(&op_path),
        args.jwt.as_deref(),
        stderr,
    ) {
        Ok(v) => {
            history_ctx.auth_source_used = v.source.clone();
            v
        }
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            append_history_best_effort(&history_ctx, exit_code);
            return 1;
        }
    };

    let vars_min_limit = parse_u64_default(std::env::var("GQL_VARS_MIN_LIMIT").ok(), 5, 0);
    let vars = match vars_path.as_deref() {
        None => None,
        Some(path) => {
            match api_testing_core::graphql::vars::GraphqlVariablesFile::load(path, vars_min_limit)
            {
                Ok(v) => Some(v.variables),
                Err(err) => {
                    let _ = writeln!(stderr, "{err}");
                    append_history_best_effort(&history_ctx, exit_code);
                    return 1;
                }
            }
        }
    };

    let op = match api_testing_core::graphql::schema::GraphqlOperationFile::load(&op_path) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            append_history_best_effort(&history_ctx, exit_code);
            return 1;
        }
    };

    let spinner = Progress::spinner(
        ProgressOptions::default()
            .with_prefix("api-gql ")
            .with_finish(ProgressFinish::Clear),
    );
    spinner.set_message("request");
    spinner.tick();

    let executed = match api_testing_core::graphql::runner::execute_graphql_request(
        &endpoint.gql_url,
        auth.bearer_token.as_deref(),
        &op.operation,
        vars.as_ref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            spinner.finish_and_clear();
            let _ = writeln!(stderr, "{err}");
            append_history_best_effort(&history_ctx, exit_code);
            return 1;
        }
    };

    spinner.finish_and_clear();
    let _ = stdout.write_all(&executed.response.body);

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
    op_arg: String,
    vars_arg: Option<String>,
    endpoint_label_used: String,
    endpoint_value_used: String,
    log_url: bool,
    auth_source_used: api_testing_core::graphql::auth::GraphqlAuthSourceUsed,
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
        api_testing_core::graphql::auth::GraphqlAuthSourceUsed::JwtProfile { name } => {
            if !name.is_empty() {
                record.push_str(&format!(" jwt={name}"));
            }
        }
        api_testing_core::graphql::auth::GraphqlAuthSourceUsed::EnvAccessToken => {
            record.push_str(" token=ACCESS_TOKEN");
        }
        api_testing_core::graphql::auth::GraphqlAuthSourceUsed::None => {}
    }

    let op_arg_path = Path::new(&ctx.op_arg);
    let op_rel = if op_arg_path.is_absolute() {
        maybe_relpath(op_arg_path, &ctx.invocation_dir)
    } else {
        ctx.op_arg.clone()
    };

    record.push_str("\n\napi-gql call \\\n");
    if !ctx.endpoint_label_used.is_empty() && (ctx.endpoint_label_used != "url" || ctx.log_url) {
        record.push_str(&format!(
            "  --{} {} \\\n",
            ctx.endpoint_label_used,
            shell_quote(&ctx.endpoint_value_used)
        ));
    }

    if let api_testing_core::graphql::auth::GraphqlAuthSourceUsed::JwtProfile { name } =
        &ctx.auth_source_used
    {
        if !name.is_empty() {
            record.push_str(&format!("  --jwt {} \\\n", shell_quote(name)));
        }
    }

    if let Some(vars_arg) = ctx.vars_arg.as_deref() {
        let vars_arg_path = Path::new(vars_arg);
        let vars_rel = if vars_arg_path.is_absolute() {
            maybe_relpath(vars_arg_path, &ctx.invocation_dir)
        } else {
            vars_arg.to_string()
        };
        record.push_str(&format!("  {} \\\n", shell_quote(&op_rel)));
        record.push_str(&format!("  {} \\\n", shell_quote(&vars_rel)));
        record.push_str("| jq .\n\n");
    } else {
        record.push_str(&format!("  {} \\\n", shell_quote(&op_rel)));
        record.push_str("| jq .\n\n");
    }

    let _ = history_writer.append(&record);
}
