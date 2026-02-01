use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};

use api_testing_core::{config, env_file, history, jwt, Result};

#[derive(Parser)]
#[command(
    name = "api-rest",
    version,
    about = "REST API runner (call/history/report)",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Execute a request file and print the response body to stdout (default)
    Call(CallArgs),
    /// Print the last (or last N) history entries
    History(HistoryArgs),
    /// Generate a Markdown API test report
    Report(ReportArgs),
    /// Generate a Markdown API test report from a saved `call` command snippet
    ReportFromCmd(ReportFromCmdArgs),
}

#[derive(Args)]
struct CallArgs {
    /// Endpoint preset name (or literal URL if it starts with http(s)://)
    #[arg(short = 'e', long = "env")]
    env: Option<String>,

    /// Explicit REST base URL
    #[arg(short = 'u', long = "url")]
    url: Option<String>,

    /// Token profile name
    #[arg(long = "token")]
    token: Option<String>,

    /// REST setup dir (discovery seed)
    #[arg(long = "config-dir")]
    config_dir: Option<String>,

    /// Disable writing to .rest_history for this run
    #[arg(long = "no-history")]
    no_history: bool,

    /// Request file path (*.request.json)
    #[arg(value_name = "request.request.json")]
    request: String,
}

#[derive(Args)]
struct HistoryArgs {
    /// REST setup dir (discovery seed)
    #[arg(long = "config-dir")]
    config_dir: Option<String>,

    /// Explicit history file path (relative paths resolve under setup dir)
    #[arg(long = "file")]
    file: Option<String>,

    /// Print the last entry (default)
    #[arg(long = "last", conflicts_with = "tail")]
    last: bool,

    /// Print the last N entries (blank-line separated)
    #[arg(long = "tail")]
    tail: Option<u32>,

    /// Omit metadata lines (starting with "#") from each entry
    #[arg(long = "command-only")]
    command_only: bool,
}

#[derive(Args)]
struct ReportArgs {
    /// Report case name
    #[arg(long = "case")]
    case: String,

    /// Request file path (*.request.json)
    #[arg(long = "request")]
    request: String,

    /// Output report path (default: <project_root>/docs/<stamp>-<case>-api-test-report.md)
    #[arg(long = "out")]
    out: Option<String>,

    /// Endpoint preset name (passed through)
    #[arg(short = 'e', long = "env")]
    env: Option<String>,

    /// Explicit REST base URL (passed through)
    #[arg(short = 'u', long = "url")]
    url: Option<String>,

    /// Token profile name (passed through)
    #[arg(long = "token")]
    token: Option<String>,

    /// Execute the request and embed the response
    #[arg(
        long = "run",
        conflicts_with = "response",
        required_unless_present = "response"
    )]
    run: bool,

    /// Use an existing response file (or "-" for stdin)
    #[arg(
        long = "response",
        conflicts_with = "run",
        required_unless_present = "run"
    )]
    response: Option<String>,

    /// Do not redact secrets in request/response JSON blocks
    #[arg(long = "no-redact")]
    no_redact: bool,

    /// Omit the command snippet section
    #[arg(long = "no-command")]
    no_command: bool,

    /// When using --url, omit the URL value in the command snippet
    #[arg(long = "no-command-url")]
    no_command_url: bool,

    /// Override project root (default: git root or CWD)
    #[arg(long = "project-root")]
    project_root: Option<String>,

    /// REST setup dir (passed through)
    #[arg(long = "config-dir")]
    config_dir: Option<String>,
}

#[derive(Args)]
struct ReportFromCmdArgs {
    /// Report case name (default: derived from the snippet)
    #[arg(long = "case")]
    case: Option<String>,

    /// Output report path (default: <project_root>/docs/<stamp>-<case>-api-test-report.md)
    #[arg(long = "out")]
    out: Option<String>,

    /// Use an existing response file (or "-" for stdin)
    ///
    /// Note: when using "--response -", stdin is reserved for the response body; provide the snippet as a positional argument.
    #[arg(long = "response")]
    response: Option<String>,

    /// Allow generating a report with an empty/no-data response (no-op for api-rest; kept for parity)
    #[arg(long = "allow-empty", alias = "expect-empty")]
    allow_empty: bool,

    /// Print the equivalent `api-rest report ...` command and exit 0
    #[arg(long = "dry-run")]
    dry_run: bool,

    /// Read the command snippet from stdin
    #[arg(long = "stdin", conflicts_with = "snippet")]
    stdin: bool,

    /// Command snippet (e.g. from `api-rest history --command-only`)
    #[arg(value_name = "snippet", required_unless_present = "stdin")]
    snippet: Option<String>,
}

fn argv_with_default_command(raw_args: &[String]) -> Vec<String> {
    let mut argv = vec!["api-rest".to_string()];
    if raw_args.is_empty() {
        return argv;
    }

    let first = raw_args[0].as_str();
    let is_root_help = first == "-h" || first == "--help";
    let is_root_version = first == "-V" || first == "--version";

    let is_explicit_command = matches!(first, "call" | "history" | "report" | "report-from-cmd");
    if !is_explicit_command && !is_root_help && !is_root_version {
        argv.push("call".to_string());
    }

    argv.extend_from_slice(raw_args);
    argv
}

fn print_root_help() {
    println!("Usage: api-rest <command> [args]");
    println!();
    println!("Commands:");
    println!("  call      Execute a request file and print the response body to stdout (default)");
    println!("  history   Print the last (or last N) history entries");
    println!("  report    Generate a Markdown API test report");
    println!("  report-from-cmd  Generate a report from a saved `call` snippet");
    println!();
    println!("Common options (see subcommand help for full details):");
    println!("  --config-dir <dir>   Seed setup/rest discovery (call/history/report)");
    println!("  -h, --help           Print help");
    println!();
    println!("Examples:");
    println!("  api-rest --help");
    println!("  api-rest call --help");
    println!("  api-rest report --help");
    println!("  api-rest report-from-cmd --help");
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let is_root_help = raw_args.len() == 1 && (raw_args[0] == "-h" || raw_args[0] == "--help");
    if raw_args.is_empty() || is_root_help {
        print_root_help();
        return 0;
    }

    let argv = argv_with_default_command(&raw_args);

    let cli = match Cli::try_parse_from(argv) {
        Ok(v) => v,
        Err(err) => {
            let code = err.exit_code();
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                let _ = err.print();
                return 0;
            }
            let _ = err.print();
            return code;
        }
    };

    let invocation_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let invocation_dir = std::fs::canonicalize(&invocation_dir).unwrap_or(invocation_dir);

    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let stdout_is_tty = std::io::stdout().is_terminal();

    match cli.command {
        None => {
            print_root_help();
            0
        }
        Some(Command::Call(args)) => cmd_call(
            &args,
            &invocation_dir,
            stdout_is_tty,
            &mut stdout,
            &mut stderr,
        ),
        Some(Command::History(args)) => {
            cmd_history(&args, &invocation_dir, &mut stdout, &mut stderr)
        }
        Some(Command::Report(args)) => cmd_report(&args, &invocation_dir, &mut stdout, &mut stderr),
        Some(Command::ReportFromCmd(args)) => {
            cmd_report_from_cmd(&args, &invocation_dir, &mut stdout, &mut stderr)
        }
    }
}

fn trim_non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn bool_from_env(raw: Option<String>, name: &str, default: bool, stderr: &mut dyn Write) -> bool {
    let raw = raw.unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return default;
    }
    match raw.to_ascii_lowercase().as_str() {
        "true" => true,
        "false" => false,
        _ => {
            let _ = writeln!(
                stderr,
                "api-rest: warning: {name} must be true|false (got: {raw}); treating as false"
            );
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

fn slugify(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars() {
        let ok = c.is_ascii_alphanumeric();
        if ok {
            out.push(c);
            prev_dash = false;
            continue;
        }
        if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }

    out
}

fn maybe_relpath(path: &Path, base: &Path) -> String {
    if path == base {
        return ".".to_string();
    }

    if let Ok(stripped) = path.strip_prefix(base) {
        let s = stripped.to_string_lossy();
        if s.is_empty() {
            return ".".to_string();
        }
        return s.to_string();
    }

    path.to_string_lossy().to_string()
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

fn list_available_suffixes(file: &Path, prefix: &str) -> Vec<String> {
    if !file.is_file() {
        return Vec::new();
    }

    let Ok(content) = std::fs::read_to_string(file) else {
        return Vec::new();
    };

    let mut out: Vec<String> = Vec::new();
    for raw_line in content.lines() {
        let line = raw_line.trim_end_matches('\r');
        let mut line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("export") {
            if rest.starts_with(char::is_whitespace) {
                line = rest.trim();
            }
        }

        let Some((lhs, _rhs)) = line.split_once('=') else {
            continue;
        };
        let key = lhs.trim();
        let Some(suffix) = key.strip_prefix(prefix) else {
            continue;
        };
        if suffix.is_empty()
            || !suffix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            continue;
        }
        out.push(suffix.to_ascii_lowercase());
    }

    out.sort();
    out.dedup();
    out
}

fn find_git_root(start_dir: &Path) -> Option<PathBuf> {
    let mut dir = start_dir;
    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => return None,
        }
    }
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

#[derive(Debug, Clone)]
struct EndpointSelection {
    rest_url: String,
    endpoint_label_used: String,
    endpoint_value_used: String,
}

fn resolve_endpoint_for_call(args: &CallArgs, setup_dir: &Path) -> Result<EndpointSelection> {
    let endpoints_env = setup_dir.join("endpoints.env");
    let endpoints_local = setup_dir.join("endpoints.local.env");
    let endpoints_files: Vec<&Path> = if endpoints_env.is_file() {
        vec![&endpoints_env, &endpoints_local]
    } else {
        Vec::new()
    };

    let rest_env_default = if !endpoints_files.is_empty() {
        env_file::read_var_last_wins("REST_ENV_DEFAULT", &endpoints_files)?
    } else {
        None
    };

    let env_name = args.env.as_deref().and_then(trim_non_empty);
    let explicit_url = args.url.as_deref().and_then(trim_non_empty);

    let (rest_url, endpoint_label_used, endpoint_value_used) = if let Some(url) = explicit_url {
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
                anyhow::bail!("endpoints.env not found (expected under setup/rest/)");
            }

            let env_key = to_env_key(env_value);
            let key = format!("REST_URL_{env_key}");
            let found = env_file::read_var_last_wins(&key, &endpoints_files)?;
            let Some(found) = found else {
                let mut available = list_available_suffixes(&endpoints_env, "REST_URL_");
                if endpoints_local.is_file() {
                    available.extend(list_available_suffixes(&endpoints_local, "REST_URL_"));
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
    } else if let Some(v) = std::env::var("REST_URL")
        .ok()
        .and_then(|s| trim_non_empty(&s))
    {
        (v.clone(), "url".to_string(), v)
    } else if let Some(default_env) = rest_env_default {
        if endpoints_files.is_empty() {
            anyhow::bail!(
                "REST_ENV_DEFAULT is set but endpoints.env not found (expected under setup/rest/)"
            );
        }
        let env_key = to_env_key(&default_env);
        let key = format!("REST_URL_{env_key}");
        let found = env_file::read_var_last_wins(&key, &endpoints_files)?;
        let Some(found) = found else {
            anyhow::bail!(
                "REST_ENV_DEFAULT is '{}' but no matching REST_URL_* was found.",
                default_env
            );
        };
        (found, "env".to_string(), default_env)
    } else {
        let rest_url = "http://localhost:6700".to_string();
        (rest_url.clone(), "url".to_string(), rest_url)
    };

    Ok(EndpointSelection {
        rest_url,
        endpoint_label_used,
        endpoint_value_used,
    })
}

#[derive(Debug, Clone)]
enum AuthSourceUsed {
    None,
    TokenProfile,
    EnvFallback { env_name: String },
}

#[derive(Debug, Clone)]
struct AuthSelection {
    bearer_token: Option<String>,
    token_name: String,
    auth_source_used: AuthSourceUsed,
}

fn resolve_auth_for_call(args: &CallArgs, setup_dir: &Path) -> Result<AuthSelection> {
    let tokens_env = setup_dir.join("tokens.env");
    let tokens_local = setup_dir.join("tokens.local.env");
    let tokens_files: Vec<&Path> = if tokens_env.is_file() || tokens_local.is_file() {
        vec![&tokens_env, &tokens_local]
    } else {
        Vec::new()
    };

    let token_name_arg = args.token.as_deref().and_then(trim_non_empty);
    let token_name_env = std::env::var("REST_TOKEN_NAME")
        .ok()
        .and_then(|s| trim_non_empty(&s));
    let token_name_file = if !tokens_files.is_empty() {
        env_file::read_var_last_wins("REST_TOKEN_NAME", &tokens_files)?
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
        let token_var = format!("REST_TOKEN_{token_key}");
        let bearer_token = env_file::read_var_last_wins(&token_var, &tokens_files)?;
        let Some(bearer_token) = bearer_token else {
            let mut available = list_available_suffixes(&tokens_env, "REST_TOKEN_");
            if tokens_local.is_file() {
                available.extend(list_available_suffixes(&tokens_local, "REST_TOKEN_"));
                available.sort();
                available.dedup();
            }
            available.retain(|t| t != "name");
            let available = if available.is_empty() {
                "none".to_string()
            } else {
                available.join(" ")
            };
            anyhow::bail!("Token profile '{token_name}' is empty/missing (available: {available}). Set it in setup/rest/tokens.local.env or use ACCESS_TOKEN without selecting a token profile.");
        };

        return Ok(AuthSelection {
            bearer_token: Some(bearer_token),
            token_name: token_name.clone(),
            auth_source_used: AuthSourceUsed::TokenProfile,
        });
    }

    let access_token = std::env::var("ACCESS_TOKEN")
        .ok()
        .and_then(|s| trim_non_empty(&s));
    if let Some(t) = access_token {
        return Ok(AuthSelection {
            bearer_token: Some(t),
            token_name,
            auth_source_used: AuthSourceUsed::EnvFallback {
                env_name: "ACCESS_TOKEN".to_string(),
            },
        });
    }

    let service_token = std::env::var("SERVICE_TOKEN")
        .ok()
        .and_then(|s| trim_non_empty(&s));
    if let Some(t) = service_token {
        return Ok(AuthSelection {
            bearer_token: Some(t),
            token_name,
            auth_source_used: AuthSourceUsed::EnvFallback {
                env_name: "SERVICE_TOKEN".to_string(),
            },
        });
    }

    Ok(AuthSelection {
        bearer_token: None,
        token_name,
        auth_source_used: AuthSourceUsed::None,
    })
}

fn validate_bearer_token_if_jwt(
    bearer_token: &str,
    auth_source: &AuthSourceUsed,
    token_name: &str,
    stderr: &mut dyn Write,
) -> Result<()> {
    let enabled = bool_from_env(
        std::env::var("REST_JWT_VALIDATE_ENABLED").ok(),
        "REST_JWT_VALIDATE_ENABLED",
        true,
        stderr,
    );
    let strict = bool_from_env(
        std::env::var("REST_JWT_VALIDATE_STRICT").ok(),
        "REST_JWT_VALIDATE_STRICT",
        false,
        stderr,
    );
    let leeway_seconds =
        parse_u64_default(std::env::var("REST_JWT_VALIDATE_LEEWAY_SECONDS").ok(), 0, 0);

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
            let _ = writeln!(stderr, "api-rest: warning: {msg}");
            Ok(())
        }
    }
}

fn cmd_call(
    args: &CallArgs,
    invocation_dir: &Path,
    stdout_is_tty: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    cmd_call_internal(args, invocation_dir, stdout_is_tty, true, stdout, stderr)
}

fn cmd_call_internal(
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

    let request_file = match api_testing_core::rest::schema::RestRequestFile::load(&request_path) {
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
    let setup_dir = match config::resolve_rest_setup_dir_for_call(
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
        && bool_from_env(
            std::env::var("REST_HISTORY_ENABLED").ok(),
            "REST_HISTORY_ENABLED",
            true,
            stderr,
        );

    let history_file_override = std::env::var("REST_HISTORY_FILE")
        .ok()
        .and_then(|s| trim_non_empty(&s))
        .map(PathBuf::from);

    let rotation = history::RotationPolicy {
        max_mb: parse_u64_default(std::env::var("REST_HISTORY_MAX_MB").ok(), 10, 0),
        keep: parse_u64_default(std::env::var("REST_HISTORY_ROTATE_COUNT").ok(), 5, 1)
            .try_into()
            .unwrap_or(u32::MAX),
    };

    let log_url = bool_from_env(
        std::env::var("REST_HISTORY_LOG_URL_ENABLED").ok(),
        "REST_HISTORY_LOG_URL_ENABLED",
        true,
        stderr,
    );

    let mut history_ctx = CallHistoryContext {
        enabled: history_enabled,
        setup_dir: setup_dir.clone(),
        history_file_override,
        rotation,
        invocation_dir: invocation_dir.to_path_buf(),
        request_arg: args.request.clone(),
        endpoint_label_used: String::new(),
        endpoint_value_used: String::new(),
        log_url,
        auth_source_used: AuthSourceUsed::None,
        token_name_for_log: String::new(),
    };

    let endpoint = match resolve_endpoint_for_call(args, &setup_dir) {
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

    let auth = match resolve_auth_for_call(args, &setup_dir) {
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

    if let Some(token) = auth.bearer_token.as_deref() {
        if let Err(err) = validate_bearer_token_if_jwt(
            token,
            &history_ctx.auth_source_used,
            &auth.token_name,
            stderr,
        ) {
            let _ = writeln!(stderr, "{err}");
            append_history_best_effort(&history_ctx, exit_code);
            return 1;
        }
    }

    let executed = match api_testing_core::rest::runner::execute_rest_request(
        &request_file,
        &endpoint.rest_url,
        auth.bearer_token.as_deref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            append_history_best_effort(&history_ctx, exit_code);
            return 1;
        }
    };

    let _ = stdout.write_all(&executed.response.body);

    if let Err(err) =
        api_testing_core::rest::expect::evaluate_main_response(&request_file.request, &executed)
    {
        let _ = writeln!(stderr, "{err}");
        maybe_print_failure_body_to_stderr(&executed.response.body, 8192, stdout_is_tty, stderr);
        append_history_best_effort(&history_ctx, exit_code);
        return 1;
    }

    if let Some(cleanup) = &request_file.request.cleanup {
        if let Err(err) = api_testing_core::rest::cleanup::execute_cleanup(
            cleanup,
            &endpoint.rest_url,
            auth.bearer_token.as_deref(),
            &executed.response.body,
        ) {
            let _ = writeln!(stderr, "{err}");
            append_history_best_effort(&history_ctx, exit_code);
            return 1;
        }
    }

    exit_code = 0;
    append_history_best_effort(&history_ctx, exit_code);

    exit_code
}

#[derive(Debug, Clone)]
struct CallHistoryContext {
    enabled: bool,
    setup_dir: PathBuf,
    history_file_override: Option<PathBuf>,
    rotation: history::RotationPolicy,
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

    let history_file = history::resolve_history_file(
        &ctx.setup_dir,
        ctx.history_file_override.as_deref(),
        ".rest_history",
    );

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
        AuthSourceUsed::EnvFallback { .. } => {
            record.push_str(" auth=ACCESS_TOKEN");
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

    record.push_str("api-rest call \\\n");
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

    let _ = history::append_record(&history_file, &record, ctx.rotation);
}

fn history_timestamp_now() -> Result<String> {
    let format = time::format_description::parse(
        "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory][offset_minute]",
    )?;
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    Ok(now.format(&format)?)
}

fn cmd_history(
    args: &HistoryArgs,
    invocation_dir: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let config_dir = args
        .config_dir
        .as_deref()
        .and_then(trim_non_empty)
        .map(PathBuf::from);
    let setup_dir =
        match config::resolve_rest_setup_dir_for_history(invocation_dir, config_dir.as_deref()) {
            Ok(v) => v,
            Err(err) => {
                let _ = writeln!(stderr, "{err}");
                return 1;
            }
        };

    let file_override = args.file.as_deref().and_then(trim_non_empty).or_else(|| {
        std::env::var("REST_HISTORY_FILE")
            .ok()
            .and_then(|s| trim_non_empty(&s))
    });
    let file_override = file_override.as_deref().map(Path::new);
    let history_file = history::resolve_history_file(&setup_dir, file_override, ".rest_history");

    if !history_file.is_file() {
        let _ = writeln!(stderr, "History file not found: {}", history_file.display());
        return 1;
    }

    let records = match history::read_records(&history_file) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };
    if records.is_empty() {
        return 3;
    }

    let n = args.tail.unwrap_or(1).max(1) as usize;
    let start = records.len().saturating_sub(n);
    for record in &records[start..] {
        if args.command_only && record.starts_with('#') {
            let trimmed = record
                .split_once('\n')
                .map(|(_first, rest)| rest)
                .unwrap_or_default();
            let _ = stdout.write_all(trimmed.as_bytes());
            if trimmed.is_empty() {
                let _ = stdout.write_all(b"\n\n");
            }
        } else {
            let _ = stdout.write_all(record.as_bytes());
        }
    }

    0
}

fn cmd_report(
    args: &ReportArgs,
    invocation_dir: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let case_name = args.case.trim();
    if case_name.is_empty() {
        let _ = writeln!(stderr, "error: --case is required");
        return 1;
    }

    let request_path = PathBuf::from(&args.request);
    if !request_path.is_file() {
        let _ = writeln!(stderr, "Request file not found: {}", request_path.display());
        return 1;
    }

    let request_file = match api_testing_core::rest::schema::RestRequestFile::load(&request_path) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };

    let project_root = if let Some(p) = args.project_root.as_deref().and_then(trim_non_empty) {
        PathBuf::from(p)
    } else {
        find_git_root(invocation_dir).unwrap_or_else(|| invocation_dir.to_path_buf())
    };

    let include_command = !args.no_command
        && bool_from_env(
            std::env::var("REST_REPORT_INCLUDE_COMMAND_ENABLED").ok(),
            "REST_REPORT_INCLUDE_COMMAND_ENABLED",
            true,
            stderr,
        );
    let include_command_url = !args.no_command_url
        && bool_from_env(
            std::env::var("REST_REPORT_COMMAND_LOG_URL_ENABLED").ok(),
            "REST_REPORT_COMMAND_LOG_URL_ENABLED",
            true,
            stderr,
        );

    let out_path = match args.out.as_deref().and_then(trim_non_empty) {
        Some(p) => PathBuf::from(p),
        None => {
            let stamp = report_stamp_now().unwrap_or_else(|_| "00000000-0000".to_string());
            let case_slug = slugify(case_name);
            let case_slug = if case_slug.is_empty() {
                "case".to_string()
            } else {
                case_slug
            };

            let report_dir = std::env::var("REST_REPORT_DIR")
                .ok()
                .and_then(|s| trim_non_empty(&s));
            let report_dir = match report_dir {
                None => project_root.join("docs"),
                Some(d) => {
                    let p = PathBuf::from(d);
                    if p.is_absolute() {
                        p
                    } else {
                        project_root.join(p)
                    }
                }
            };

            report_dir.join(format!("{stamp}-{case_slug}-api-test-report.md"))
        }
    };

    if let Some(parent) = out_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let report_date = report_date_now().unwrap_or_else(|_| "0000-00-00".to_string());
    let generated_at = history_timestamp_now().unwrap_or_else(|_| "".to_string());

    let endpoint_note = if args.url.as_deref().and_then(trim_non_empty).is_some() {
        format!(
            "Endpoint: --url {}",
            args.url.as_deref().unwrap_or_default()
        )
    } else if args.env.as_deref().and_then(trim_non_empty).is_some() {
        format!(
            "Endpoint: --env {}",
            args.env.as_deref().unwrap_or_default()
        )
    } else {
        "Endpoint: (implicit; see REST_URL / REST_ENV_DEFAULT)".to_string()
    };

    let mut stderr_note: Option<String> = None;
    let mut run_exit_code: i32 = 0;

    let response_raw: Vec<u8> = if args.run {
        let mut run_stdout = Vec::new();
        let mut run_stderr = Vec::new();
        let call_args = CallArgs {
            env: args.env.clone(),
            url: args.url.clone(),
            token: args.token.clone(),
            config_dir: args.config_dir.clone(),
            no_history: true,
            request: args.request.clone(),
        };
        run_exit_code = cmd_call_internal(
            &call_args,
            invocation_dir,
            false,
            false,
            &mut run_stdout,
            &mut run_stderr,
        );
        if !run_stderr.is_empty() {
            stderr_note = Some(String::from_utf8_lossy(&run_stderr).to_string());
        }
        run_stdout
    } else {
        let Some(resp) = args.response.as_deref().and_then(trim_non_empty) else {
            let _ = writeln!(stderr, "error: Use either --run or --response.");
            return 1;
        };
        if resp == "-" {
            let mut buf = Vec::new();
            if std::io::stdin().read_to_end(&mut buf).is_err() {
                let _ = writeln!(stderr, "error: failed to read response from stdin");
                return 1;
            }
            buf
        } else {
            let resp_path = PathBuf::from(resp);
            if !resp_path.is_file() {
                let _ = writeln!(stderr, "Response file not found: {}", resp_path.display());
                return 1;
            }
            match std::fs::read(&resp_path) {
                Ok(v) => v,
                Err(_) => {
                    let _ = writeln!(
                        stderr,
                        "error: failed to read response file: {}",
                        resp_path.display()
                    );
                    return 1;
                }
            }
        }
    };

    let (response_lang, response_body, response_json_for_eval) =
        match serde_json::from_slice::<serde_json::Value>(&response_raw) {
            Ok(v) => {
                let eval_json = v.clone();
                let mut display_json = v;
                if !args.no_redact {
                    let _ = api_testing_core::redact::redact_json(&mut display_json);
                }
                let pretty = api_testing_core::markdown::format_json_pretty_sorted(&display_json)
                    .unwrap_or_else(|_| display_json.to_string());
                ("json".to_string(), pretty, Some(eval_json))
            }
            Err(_) => (
                "text".to_string(),
                String::from_utf8_lossy(&response_raw).to_string(),
                None,
            ),
        };

    let request_json = {
        let mut v = request_file.request.raw.clone();
        if !args.no_redact {
            let _ = api_testing_core::redact::redact_json(&mut v);
        }
        api_testing_core::markdown::format_json_pretty_sorted(&v).unwrap_or_else(|_| v.to_string())
    };

    let command_snippet = if include_command {
        Some(build_report_command_snippet(
            args,
            &project_root,
            include_command_url,
        ))
    } else {
        None
    };

    let result_note = if args.run {
        if run_exit_code == 0 {
            "Result: PASS".to_string()
        } else {
            format!("Result: FAIL (api-rest exit={run_exit_code})")
        }
    } else {
        "Result: (response provided; request not executed)".to_string()
    };

    let mut assertions: Vec<api_testing_core::rest::report::RestReportAssertion> = Vec::new();
    if let Some(expect) = &request_file.request.expect {
        let status_state = if args.run {
            if run_exit_code == 0 {
                "PASS"
            } else {
                "FAIL"
            }
        } else {
            "NOT_EVALUATED"
        };
        assertions.push(api_testing_core::rest::report::RestReportAssertion {
            label: format!("expect.status: {}", expect.status),
            state: status_state.to_string(),
        });

        if let Some(expr) = expect.jq.as_deref() {
            let jq_state = if args.run {
                if run_exit_code == 0 {
                    "PASS"
                } else {
                    "FAIL"
                }
            } else if let Some(json) = response_json_for_eval.as_ref() {
                if api_testing_core::jq::eval_exit_status(json, expr).unwrap_or(false) {
                    "PASS"
                } else {
                    "FAIL"
                }
            } else {
                "NOT_EVALUATED"
            };
            assertions.push(api_testing_core::rest::report::RestReportAssertion {
                label: format!("expect.jq: {expr}"),
                state: jq_state.to_string(),
            });
        }
    }

    let report = api_testing_core::rest::report::RestReport {
        report_date,
        case_name: case_name.to_string(),
        generated_at,
        endpoint_note,
        result_note,
        command_snippet,
        assertions,
        request_json,
        response_lang,
        response_body,
        stderr_note,
    };

    let markdown = api_testing_core::rest::report::render_rest_report_markdown(&report);
    if std::fs::write(&out_path, markdown).is_err() {
        let _ = writeln!(
            stderr,
            "error: failed to write report: {}",
            out_path.display()
        );
        return 1;
    }

    let _ = writeln!(stdout, "{}", out_path.display());
    0
}

fn cmd_report_from_cmd(
    args: &ReportFromCmdArgs,
    invocation_dir: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let _ = args.allow_empty;

    let response = args.response.as_deref().and_then(trim_non_empty);
    let response_is_stdin = matches!(response.as_deref(), Some("-"));

    if response_is_stdin && args.stdin {
        let _ = writeln!(
            stderr,
            "error: When using --response -, stdin is reserved for the response body; provide the snippet as a positional argument."
        );
        return 1;
    }

    let snippet = if args.stdin {
        let mut buf = Vec::new();
        if std::io::stdin().read_to_end(&mut buf).is_err() {
            let _ = writeln!(stderr, "error: failed to read command snippet from stdin");
            return 1;
        }
        String::from_utf8_lossy(&buf).to_string()
    } else {
        args.snippet.clone().unwrap_or_default()
    };

    let parsed = match api_testing_core::cmd_snippet::parse_report_from_cmd_snippet(&snippet) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "error: {err}");
            return 1;
        }
    };

    let api_testing_core::cmd_snippet::ReportFromCmd::Rest(rest) = parsed else {
        let _ = writeln!(
            stderr,
            "error: expected a REST call snippet (api-rest/rest.sh)"
        );
        return 1;
    };

    let api_testing_core::cmd_snippet::RestReportFromCmd {
        case: derived_case,
        config_dir,
        env,
        url,
        token,
        request,
    } = rest;

    let case_name = args
        .case
        .as_deref()
        .and_then(trim_non_empty)
        .unwrap_or_else(|| derived_case.clone());

    let report_args = ReportArgs {
        case: case_name,
        request,
        out: args.out.clone(),
        env,
        url,
        token,
        run: response.is_none(),
        response,
        no_redact: false,
        no_command: false,
        no_command_url: false,
        project_root: None,
        config_dir,
    };

    if args.dry_run {
        let cmd = build_report_from_cmd_dry_run_command(&report_args);
        let _ = writeln!(stdout, "{cmd}");
        return 0;
    }

    cmd_report(&report_args, invocation_dir, stdout, stderr)
}

fn build_report_from_cmd_dry_run_command(args: &ReportArgs) -> String {
    let mut cmd = String::from("api-rest report");

    cmd.push_str(" --case ");
    cmd.push_str(&shell_quote(&args.case));

    cmd.push_str(" --request ");
    cmd.push_str(&shell_quote(&args.request));

    if let Some(out) = args.out.as_deref().and_then(trim_non_empty) {
        cmd.push_str(" --out ");
        cmd.push_str(&shell_quote(&out));
    }

    if let Some(cfg) = args.config_dir.as_deref().and_then(trim_non_empty) {
        cmd.push_str(" --config-dir ");
        cmd.push_str(&shell_quote(&cfg));
    }

    if let Some(url) = args.url.as_deref().and_then(trim_non_empty) {
        cmd.push_str(" --url ");
        cmd.push_str(&shell_quote(&url));
    }
    if let Some(env) = args.env.as_deref().and_then(trim_non_empty) {
        if args.url.as_deref().and_then(trim_non_empty).is_none() {
            cmd.push_str(" --env ");
            cmd.push_str(&shell_quote(&env));
        }
    }
    if let Some(token) = args.token.as_deref().and_then(trim_non_empty) {
        cmd.push_str(" --token ");
        cmd.push_str(&shell_quote(&token));
    }

    if args.run {
        cmd.push_str(" --run");
    } else if let Some(resp) = args.response.as_deref().and_then(trim_non_empty) {
        cmd.push_str(" --response ");
        cmd.push_str(&shell_quote(&resp));
    } else {
        cmd.push_str(" --run");
    }

    cmd
}

fn build_report_command_snippet(
    args: &ReportArgs,
    project_root: &Path,
    include_command_url: bool,
) -> String {
    let req_arg = PathBuf::from(&args.request);
    let req_arg = if req_arg.is_absolute() {
        maybe_relpath(&req_arg, project_root)
    } else {
        args.request.clone()
    };

    let config_arg = args
        .config_dir
        .as_deref()
        .and_then(trim_non_empty)
        .map(PathBuf::from);
    let config_arg = config_arg.map(|p| {
        if p.is_absolute() {
            maybe_relpath(&p, project_root)
        } else {
            p.to_string_lossy().to_string()
        }
    });

    let mut out = String::new();
    out.push_str("api-rest call \\\n");
    if let Some(cfg) = config_arg {
        out.push_str(&format!("  --config-dir {} \\\n", shell_quote(&cfg)));
    }

    if let Some(url) = args.url.as_deref().and_then(trim_non_empty) {
        let value = if include_command_url {
            url
        } else {
            "<omitted>".to_string()
        };
        out.push_str(&format!("  --url {} \\\n", shell_quote(&value)));
    }
    if let Some(env) = args.env.as_deref().and_then(trim_non_empty) {
        if args.url.as_deref().and_then(trim_non_empty).is_none() {
            out.push_str(&format!("  --env {} \\\n", shell_quote(&env)));
        }
    }
    if let Some(token) = args.token.as_deref().and_then(trim_non_empty) {
        out.push_str(&format!("  --token {} \\\n", shell_quote(&token)));
    }

    out.push_str(&format!("  {} \\\n", shell_quote(&req_arg)));
    out.push_str("| jq .\n");
    out
}

fn report_stamp_now() -> Result<String> {
    let format = time::format_description::parse("[year][month][day]-[hour][minute]")?;
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    Ok(now.format(&format)?)
}

fn report_date_now() -> Result<String> {
    let format = time::format_description::parse("[year]-[month]-[day]")?;
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    Ok(now.format(&format)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn write_file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, contents).expect("write");
    }

    fn write_json(path: &Path, value: &serde_json::Value) {
        write_file(path, &serde_json::to_string_pretty(value).unwrap());
    }

    #[test]
    fn argv_with_default_command_inserts_call() {
        let argv = argv_with_default_command(&[]);
        assert_eq!(argv, vec!["api-rest".to_string()]);

        let argv = argv_with_default_command(&["--help".to_string()]);
        assert_eq!(argv, vec!["api-rest".to_string(), "--help".to_string()]);

        let argv = argv_with_default_command(&["history".to_string()]);
        assert_eq!(argv, vec!["api-rest".to_string(), "history".to_string()]);

        let argv = argv_with_default_command(&["requests/health.request.json".to_string()]);
        assert_eq!(
            argv,
            vec![
                "api-rest".to_string(),
                "call".to_string(),
                "requests/health.request.json".to_string()
            ]
        );
    }

    #[test]
    fn bool_from_env_parses_and_warns() {
        let mut stderr = Vec::new();
        let got = bool_from_env(Some("true".to_string()), "REST_FOO", false, &mut stderr);
        assert_eq!(got, true);

        let mut stderr = Vec::new();
        let got = bool_from_env(Some("nope".to_string()), "REST_FOO", true, &mut stderr);
        assert_eq!(got, false);
        let msg = String::from_utf8_lossy(&stderr);
        assert!(msg.contains("REST_FOO must be true|false"));
    }

    #[test]
    fn parse_u64_default_enforces_min() {
        assert_eq!(parse_u64_default(Some("".to_string()), 10, 1), 10);
        assert_eq!(parse_u64_default(Some("abc".to_string()), 10, 1), 10);
        assert_eq!(parse_u64_default(Some("0".to_string()), 10, 1), 1);
    }

    #[test]
    fn to_env_key_and_slugify_normalize() {
        assert_eq!(to_env_key("prod-us"), "PROD_US");
        assert_eq!(to_env_key("  foo@@bar  "), "FOO_BAR");
        assert_eq!(slugify("Hello, world!"), "hello-world");
        assert_eq!(slugify("  ___ "), "");
    }

    #[test]
    fn maybe_relpath_and_shell_quote() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        assert_eq!(maybe_relpath(root, root), ".");

        let child = root.join("a/b");
        std::fs::create_dir_all(&child).unwrap();
        assert_eq!(maybe_relpath(&child, root), "a/b");

        assert_eq!(shell_quote(""), "''");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn list_available_suffixes_parses_and_sorts() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("endpoints.env");
        write_file(
            &file,
            "export REST_URL_PROD=http://prod\nREST_URL_DEV=http://dev\nREST_URL_=bad\nREST_URL_FOO-BAR=http://x\nREST_URL_TEST=http://t\nREST_URL_TEST=http://t2\n",
        );

        let suffixes = list_available_suffixes(&file, "REST_URL_");
        assert_eq!(suffixes, vec!["dev", "prod", "test"]);
    }

    #[test]
    fn resolve_endpoint_for_call_honors_url_and_env() {
        let tmp = TempDir::new().unwrap();
        let setup = tmp.path().join("setup/rest");
        std::fs::create_dir_all(&setup).unwrap();
        write_file(
            &setup.join("endpoints.env"),
            "REST_ENV_DEFAULT=prod\nREST_URL_PROD=http://prod\nREST_URL_STAGING=http://staging\n",
        );

        let args = CallArgs {
            env: None,
            url: Some("http://explicit".to_string()),
            token: None,
            config_dir: None,
            no_history: false,
            request: "requests/health.request.json".to_string(),
        };
        let sel = resolve_endpoint_for_call(&args, &setup).unwrap();
        assert_eq!(sel.rest_url, "http://explicit");
        assert_eq!(sel.endpoint_label_used, "url");

        let args = CallArgs {
            env: Some("staging".to_string()),
            url: None,
            token: None,
            config_dir: None,
            no_history: false,
            request: "requests/health.request.json".to_string(),
        };
        let sel = resolve_endpoint_for_call(&args, &setup).unwrap();
        assert_eq!(sel.rest_url, "http://staging");
        assert_eq!(sel.endpoint_label_used, "env");

        let args = CallArgs {
            env: Some("https://example.test".to_string()),
            url: None,
            token: None,
            config_dir: None,
            no_history: false,
            request: "requests/health.request.json".to_string(),
        };
        let sel = resolve_endpoint_for_call(&args, &setup).unwrap();
        assert_eq!(sel.rest_url, "https://example.test");
        assert_eq!(sel.endpoint_label_used, "url");
    }

    #[test]
    fn resolve_endpoint_for_call_unknown_env_lists_available() {
        let tmp = TempDir::new().unwrap();
        let setup = tmp.path().join("setup/rest");
        std::fs::create_dir_all(&setup).unwrap();
        write_file(
            &setup.join("endpoints.env"),
            "REST_URL_PROD=http://prod\nREST_URL_DEV=http://dev\n",
        );

        let args = CallArgs {
            env: Some("missing".to_string()),
            url: None,
            token: None,
            config_dir: None,
            no_history: false,
            request: "requests/health.request.json".to_string(),
        };

        let err = resolve_endpoint_for_call(&args, &setup).unwrap_err();
        assert!(err.to_string().contains("Unknown --env 'missing'"));
        assert!(err.to_string().contains("prod"));
    }

    #[test]
    fn resolve_auth_for_call_prefers_profile_then_env_fallback() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::set("ACCESS_TOKEN", "env-token");

        let tmp = TempDir::new().unwrap();
        let setup = tmp.path().join("setup/rest");
        std::fs::create_dir_all(&setup).unwrap();
        write_file(&setup.join("tokens.env"), "REST_TOKEN_SVC=svc-token\n");

        let args = CallArgs {
            env: None,
            url: None,
            token: Some("svc".to_string()),
            config_dir: None,
            no_history: false,
            request: "requests/health.request.json".to_string(),
        };
        let auth = resolve_auth_for_call(&args, &setup).unwrap();
        assert_eq!(auth.bearer_token.as_deref(), Some("svc-token"));
        assert!(matches!(
            auth.auth_source_used,
            AuthSourceUsed::TokenProfile
        ));

        let args = CallArgs {
            env: None,
            url: None,
            token: None,
            config_dir: None,
            no_history: false,
            request: "requests/health.request.json".to_string(),
        };
        let auth = resolve_auth_for_call(&args, &setup).unwrap();
        assert_eq!(auth.bearer_token.as_deref(), Some("env-token"));
        assert!(matches!(
            auth.auth_source_used,
            AuthSourceUsed::EnvFallback { .. }
        ));
    }

    #[test]
    fn build_report_commands_include_expected_flags() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let req = root.join("requests/health.request.json");
        write_json(
            &req,
            &serde_json::json!({
                "method": "GET",
                "path": "/health",
                "expect": { "status": 200 }
            }),
        );

        let args = ReportArgs {
            case: "Health".to_string(),
            request: req.to_string_lossy().to_string(),
            out: None,
            env: Some("staging".to_string()),
            url: None,
            token: Some("svc".to_string()),
            run: true,
            response: None,
            no_redact: false,
            no_command: false,
            no_command_url: false,
            project_root: None,
            config_dir: Some("setup/rest".to_string()),
        };

        let snippet = build_report_command_snippet(&args, root, true);
        assert!(snippet.contains("--config-dir 'setup/rest'"));
        assert!(snippet.contains("--env 'staging'"));
        assert!(snippet.contains("--token 'svc'"));
        assert!(snippet.contains("api-rest call"));

        let cmd = build_report_from_cmd_dry_run_command(&args);
        assert!(cmd.contains("--case 'Health'"));
        assert!(cmd.contains("--request "));
        assert!(cmd.contains(" --run"));
    }

    #[test]
    fn cmd_history_command_only_and_empty_records() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("setup/rest")).unwrap();

        let history_file = root.join("setup/rest/.rest_history");
        write_file(
            &history_file,
            "# stamp exit=0 setup_dir=.\napi-rest call \\\n  --config-dir 'setup/rest' \\\n  requests/health.request.json \\\n| jq .\n\n",
        );

        let args = HistoryArgs {
            config_dir: Some("setup/rest".to_string()),
            file: None,
            last: false,
            tail: Some(1),
            command_only: true,
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = cmd_history(&args, root, &mut stdout, &mut stderr);
        assert_eq!(code, 0);
        let out = String::from_utf8_lossy(&stdout);
        assert!(out.contains("api-rest call"));

        write_file(&history_file, "");
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = cmd_history(&args, root, &mut stdout, &mut stderr);
        assert_eq!(code, 3);
    }

    #[test]
    fn cmd_report_writes_report_from_response_file() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _guard = EnvGuard::set("REST_REPORT_INCLUDE_COMMAND_ENABLED", "true");
        let _guard_url = EnvGuard::set("REST_REPORT_COMMAND_LOG_URL_ENABLED", "false");

        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let requests = root.join("requests");
        std::fs::create_dir_all(&requests).unwrap();

        let request_file = requests.join("health.request.json");
        write_json(
            &request_file,
            &serde_json::json!({
                "method": "GET",
                "path": "/health",
                "expect": { "status": 200 }
            }),
        );

        let response_file = root.join("response.json");
        write_file(&response_file, r#"{"ok":true}"#);

        let out_path = root.join("report.md");
        let args = ReportArgs {
            case: "Health".to_string(),
            request: request_file.to_string_lossy().to_string(),
            out: Some(out_path.to_string_lossy().to_string()),
            env: Some("staging".to_string()),
            url: None,
            token: None,
            run: false,
            response: Some(response_file.to_string_lossy().to_string()),
            no_redact: true,
            no_command: false,
            no_command_url: true,
            project_root: Some(root.to_string_lossy().to_string()),
            config_dir: Some("setup/rest".to_string()),
        };

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = cmd_report(&args, root, &mut stdout, &mut stderr);
        assert_eq!(code, 0);
        assert!(out_path.is_file());
        let report = std::fs::read_to_string(&out_path).unwrap();
        assert!(report.contains("Result: (response provided; request not executed)"));
        assert!(report.contains("Endpoint: --env staging"));
    }
}
