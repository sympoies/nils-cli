use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use clap::error::ErrorKind;
use clap::{Args, Parser, Subcommand};

use api_testing_core::{config, env_file, history, Result};
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};

#[derive(Parser)]
#[command(
    name = "api-gql",
    version,
    about = "GraphQL runner (call/history/report/schema)",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Execute an operation (and optional variables) and print the response body JSON to stdout (default)
    Call(CallArgs),
    /// Print the last (or last N) history entries
    History(HistoryArgs),
    /// Generate a Markdown API test report
    Report(ReportArgs),
    /// Generate a report from a command snippet (arg or stdin)
    ReportFromCmd(ReportFromCmdArgs),
    /// Resolve a schema file path (or print schema contents)
    Schema(SchemaArgs),
}

#[derive(Args)]
struct CallArgs {
    /// Endpoint preset name (or literal URL if it starts with http(s)://)
    #[arg(short = 'e', long = "env")]
    env: Option<String>,

    /// Explicit GraphQL endpoint URL
    #[arg(short = 'u', long = "url")]
    url: Option<String>,

    /// JWT profile name
    #[arg(long = "jwt")]
    jwt: Option<String>,

    /// GraphQL setup dir (discovery seed)
    #[arg(long = "config-dir")]
    config_dir: Option<String>,

    /// Print available env names from endpoints.env, then exit
    #[arg(long = "list-envs")]
    list_envs: bool,

    /// Print available JWT profile names from jwts(.local).env, then exit
    #[arg(long = "list-jwts")]
    list_jwts: bool,

    /// Disable writing to .gql_history for this run
    #[arg(long = "no-history")]
    no_history: bool,

    /// Operation file path (*.graphql)
    #[arg(value_name = "operation.graphql")]
    operation: Option<String>,

    /// Variables JSON file path
    #[arg(value_name = "variables.json")]
    variables: Option<String>,
}

#[derive(Args)]
struct HistoryArgs {
    /// GraphQL setup dir (discovery seed)
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

    /// GraphQL operation file path (*.graphql)
    #[arg(long = "op", alias = "operation")]
    op: String,

    /// Variables JSON file path
    #[arg(long = "vars", alias = "variables")]
    vars: Option<String>,

    /// Output report path (default: <project_root>/docs/<stamp>-<case>-api-test-report.md)
    #[arg(long = "out")]
    out: Option<String>,

    /// Endpoint preset name (passed through)
    #[arg(short = 'e', long = "env")]
    env: Option<String>,

    /// Explicit GraphQL endpoint URL (passed through)
    #[arg(short = 'u', long = "url")]
    url: Option<String>,

    /// JWT profile name (passed through)
    #[arg(long = "jwt")]
    jwt: Option<String>,

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

    /// Allow generating a report with an empty/no-data response (or as a draft without --run/--response)
    #[arg(long = "allow-empty", alias = "expect-empty")]
    allow_empty: bool,

    /// Do not redact secrets in variables/response JSON blocks
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

    /// GraphQL setup dir (passed through)
    #[arg(long = "config-dir")]
    config_dir: Option<String>,
}

#[derive(Args)]
struct ReportFromCmdArgs {
    /// Override report case name (default: derived from snippet)
    #[arg(long = "case")]
    case: Option<String>,

    /// Output report path (default: <project_root>/docs/<stamp>-<case>-api-test-report.md)
    #[arg(long = "out")]
    out: Option<String>,

    /// Use an existing response file (or "-" for stdin)
    #[arg(long = "response")]
    response: Option<String>,

    /// Allow generating a report with an empty/no-data response
    #[arg(long = "allow-empty", alias = "expect-empty")]
    allow_empty: bool,

    /// Print equivalent `api-gql report ...` command and exit 0 (no network)
    #[arg(long = "dry-run")]
    dry_run: bool,

    /// Read the command snippet from stdin
    #[arg(long = "stdin", conflicts_with = "snippet")]
    stdin: bool,

    /// Command snippet (e.g. from `api-gql history --command-only`)
    #[arg(value_name = "snippet", required_unless_present = "stdin")]
    snippet: Option<String>,
}

#[derive(Args)]
struct SchemaArgs {
    /// GraphQL setup dir (same discovery semantics as call)
    #[arg(long = "config-dir")]
    config_dir: Option<String>,

    /// Explicit schema file path (overrides env + schema.env)
    #[arg(long = "file")]
    file: Option<String>,

    /// Print schema file contents (default: print resolved path)
    #[arg(long = "cat")]
    cat: bool,
}

fn argv_with_default_command(raw_args: &[String]) -> Vec<String> {
    let mut argv = vec!["api-gql".to_string()];
    if raw_args.is_empty() {
        return argv;
    }

    let first = raw_args[0].as_str();
    let is_root_help = first == "-h" || first == "--help";
    let is_root_version = first == "-V" || first == "--version";

    let is_explicit_command = matches!(
        first,
        "call" | "history" | "report" | "report-from-cmd" | "schema"
    );
    if !is_explicit_command && !is_root_help && !is_root_version {
        argv.push("call".to_string());
    }

    argv.extend_from_slice(raw_args);
    argv
}

fn print_root_help() {
    println!("Usage: api-gql <command> [args]");
    println!();
    println!("Commands:");
    println!("  call     Execute an operation (and optional variables) and print response JSON (default)");
    println!("  history  Print the last (or last N) history entries");
    println!("  report   Generate a Markdown API test report");
    println!("  report-from-cmd  Generate a report from a command snippet (arg or stdin)");
    println!("  schema   Resolve a schema file path (or print schema contents)");
    println!();
    println!("Common options (see subcommand help for full details):");
    println!("  --config-dir <dir>   Seed setup/graphql discovery (call/history/report/schema)");
    println!("  --list-envs          List available endpoint presets and exit 0 (call)");
    println!("  --list-jwts          List available JWT profiles and exit 0 (call)");
    println!("  -h, --help           Print help");
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

    let stdout_is_tty = std::io::stdout().is_terminal();
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();

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
        Some(Command::Schema(args)) => cmd_schema(&args, &invocation_dir, &mut stdout, &mut stderr),
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
                "api-gql: warning: {name} must be true|false (got: {raw}); treating as false"
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

fn build_api_gql_report_dry_run_command(args: &ReportArgs) -> String {
    let mut cmd: Vec<String> = vec!["api-gql".to_string(), "report".to_string()];

    cmd.push("--case".to_string());
    cmd.push(shell_quote(&args.case));

    cmd.push("--op".to_string());
    cmd.push(shell_quote(&args.op));

    if let Some(v) = args.vars.as_deref().and_then(trim_non_empty) {
        cmd.push("--vars".to_string());
        cmd.push(shell_quote(&v));
    }

    if let Some(out) = args.out.as_deref().and_then(trim_non_empty) {
        cmd.push("--out".to_string());
        cmd.push(shell_quote(&out));
    }

    if let Some(cd) = args.config_dir.as_deref().and_then(trim_non_empty) {
        cmd.push("--config-dir".to_string());
        cmd.push(shell_quote(&cd));
    }

    if let Some(env) = args.env.as_deref().and_then(trim_non_empty) {
        cmd.push("--env".to_string());
        cmd.push(shell_quote(&env));
    }

    if let Some(url) = args.url.as_deref().and_then(trim_non_empty) {
        cmd.push("--url".to_string());
        cmd.push(shell_quote(&url));
    }

    if let Some(jwt) = args.jwt.as_deref().and_then(trim_non_empty) {
        cmd.push("--jwt".to_string());
        cmd.push(shell_quote(&jwt));
    }

    if args.run {
        cmd.push("--run".to_string());
    } else if let Some(resp) = args.response.as_deref().and_then(trim_non_empty) {
        cmd.push("--response".to_string());
        cmd.push(shell_quote(&resp));
    }

    if args.allow_empty {
        cmd.push("--allow-empty".to_string());
    }

    cmd.join(" ")
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

fn history_timestamp_now() -> Result<String> {
    let format = time::format_description::parse(
        "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory][offset_minute]",
    )?;
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    Ok(now.format(&format)?)
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

#[derive(Debug, Clone)]
struct EndpointSelection {
    gql_url: String,
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
                let mut available = list_available_suffixes(&endpoints_env, "GQL_URL_");
                if endpoints_local.is_file() {
                    available.extend(list_available_suffixes(&endpoints_local, "GQL_URL_"));
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

    if args.list_envs {
        let endpoints_env = setup_dir.join("endpoints.env");
        let endpoints_local = setup_dir.join("endpoints.local.env");
        if !endpoints_env.is_file() {
            let _ = writeln!(
                stderr,
                "endpoints.env not found (expected under setup/graphql/)"
            );
            return 1;
        }
        let mut out = list_available_suffixes(&endpoints_env, "GQL_URL_");
        if endpoints_local.is_file() {
            out.extend(list_available_suffixes(&endpoints_local, "GQL_URL_"));
            out.sort();
            out.dedup();
        }
        for v in out {
            let _ = writeln!(stdout, "{v}");
        }
        return 0;
    }

    if args.list_jwts {
        let jwts_env = setup_dir.join("jwts.env");
        let jwts_local = setup_dir.join("jwts.local.env");
        if !jwts_env.is_file() && !jwts_local.is_file() {
            let _ = writeln!(
                stderr,
                "jwts(.local).env not found (expected under setup/graphql/)"
            );
            return 1;
        }
        let mut out = list_available_suffixes(&jwts_env, "GQL_JWT_");
        if jwts_local.is_file() {
            out.extend(list_available_suffixes(&jwts_local, "GQL_JWT_"));
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

    let history_file_override = std::env::var("GQL_HISTORY_FILE")
        .ok()
        .and_then(|s| trim_non_empty(&s))
        .map(PathBuf::from);

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

    let mut history_ctx = CallHistoryContext {
        enabled: history_enabled,
        setup_dir: setup_dir.clone(),
        history_file_override,
        rotation,
        invocation_dir: invocation_dir.to_path_buf(),
        op_arg: args.operation.clone().unwrap_or_default(),
        vars_arg: args.variables.clone(),
        endpoint_label_used: String::new(),
        endpoint_value_used: String::new(),
        log_url,
        auth_source_used: api_testing_core::graphql::auth::GraphqlAuthSourceUsed::None,
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

    let auth = match api_testing_core::graphql::auth::resolve_bearer_token(
        &setup_dir,
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
    history_file_override: Option<PathBuf>,
    rotation: history::RotationPolicy,
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

    let history_file = history::resolve_history_file(
        &ctx.setup_dir,
        ctx.history_file_override.as_deref(),
        ".gql_history",
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
        api_testing_core::graphql::auth::GraphqlAuthSourceUsed::JwtProfile { name } => {
            if !name.is_empty() {
                record.push_str(&format!(" jwt={name}"));
            }
        }
        api_testing_core::graphql::auth::GraphqlAuthSourceUsed::EnvAccessToken => {
            record.push_str(" auth=ACCESS_TOKEN");
        }
        api_testing_core::graphql::auth::GraphqlAuthSourceUsed::None => {}
    }

    record.push('\n');

    let config_rel = maybe_relpath(&ctx.setup_dir, &ctx.invocation_dir);
    let op_arg_path = Path::new(&ctx.op_arg);
    let op_rel = if op_arg_path.is_absolute() {
        maybe_relpath(op_arg_path, &ctx.invocation_dir)
    } else {
        ctx.op_arg.clone()
    };

    record.push_str("api-gql call \\\n");
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

    let _ = history::append_record(&history_file, &record, ctx.rotation);
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

    let setup_dir = match config::resolve_gql_setup_dir_for_history(
        invocation_dir,
        invocation_dir,
        config_dir.as_deref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };

    let file_override = args.file.as_deref().and_then(trim_non_empty).or_else(|| {
        std::env::var("GQL_HISTORY_FILE")
            .ok()
            .and_then(|s| trim_non_empty(&s))
    });
    let file_override = file_override.as_deref().map(Path::new);
    let history_file = history::resolve_history_file(&setup_dir, file_override, ".gql_history");

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

fn response_has_meaningful_data_records(response: &serde_json::Value) -> bool {
    let data = response.get("data");
    let Some(data) = data else {
        return false;
    };
    if data.is_null() {
        return false;
    }

    const META_KEYS: &[&str] = &[
        "__typename",
        "pageinfo",
        "totalcount",
        "count",
        "cursor",
        "edges",
        "nodes",
        "hasnextpage",
        "haspreviouspage",
        "startcursor",
        "endcursor",
    ];

    #[derive(Debug, Clone)]
    enum PathElem {
        Key(String),
        Index,
    }

    fn is_meta_key(key: &str) -> bool {
        let k = key.trim().to_ascii_lowercase();
        META_KEYS.iter().any(|m| *m == k)
    }

    fn key_for_path(path: &[PathElem]) -> Option<String> {
        if path.is_empty() {
            return None;
        }
        match path.last().expect("non-empty") {
            PathElem::Key(k) => Some(k.clone()),
            PathElem::Index => match path.iter().rev().nth(1) {
                Some(PathElem::Key(k)) => Some(k.clone()),
                _ => None,
            },
        }
    }

    fn walk(value: &serde_json::Value, path: &mut Vec<PathElem>) -> bool {
        match value {
            serde_json::Value::Null => false,
            serde_json::Value::Bool(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::String(_) => {
                let Some(k) = key_for_path(path) else {
                    return false;
                };
                !is_meta_key(&k)
            }
            serde_json::Value::Array(values) => {
                for v in values.iter() {
                    path.push(PathElem::Index);
                    if walk(v, path) {
                        return true;
                    }
                    path.pop();
                }
                false
            }
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    path.push(PathElem::Key(k.clone()));
                    if walk(v, path) {
                        return true;
                    }
                    path.pop();
                }
                false
            }
        }
    }

    walk(data, &mut Vec::new())
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

    let op_path = PathBuf::from(&args.op);
    if !op_path.is_file() {
        let _ = writeln!(stderr, "Operation file not found: {}", op_path.display());
        return 1;
    }

    let vars_path = args
        .vars
        .as_deref()
        .and_then(trim_non_empty)
        .map(PathBuf::from);
    if let Some(vp) = vars_path.as_deref() {
        if !vp.is_file() {
            let _ = writeln!(stderr, "Variables file not found: {}", vp.display());
            return 1;
        }
    }

    let project_root = if let Some(p) = args.project_root.as_deref().and_then(trim_non_empty) {
        PathBuf::from(p)
    } else {
        find_git_root(invocation_dir).unwrap_or_else(|| invocation_dir.to_path_buf())
    };

    let include_command = !args.no_command
        && bool_from_env(
            std::env::var("GQL_REPORT_INCLUDE_COMMAND_ENABLED").ok(),
            "GQL_REPORT_INCLUDE_COMMAND_ENABLED",
            true,
            stderr,
        );
    let include_command_url = !args.no_command_url
        && bool_from_env(
            std::env::var("GQL_REPORT_COMMAND_LOG_URL_ENABLED").ok(),
            "GQL_REPORT_COMMAND_LOG_URL_ENABLED",
            true,
            stderr,
        );

    let allow_empty = args.allow_empty
        || bool_from_env(
            std::env::var("GQL_ALLOW_EMPTY_ENABLED").ok(),
            "GQL_ALLOW_EMPTY_ENABLED",
            false,
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

            let report_dir = std::env::var("GQL_REPORT_DIR")
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
        "Endpoint: (implicit; see GQL_URL / GQL_ENV_DEFAULT)".to_string()
    };

    let op_content = match std::fs::read_to_string(&op_path) {
        Ok(v) => v,
        Err(_) => {
            let _ = writeln!(
                stderr,
                "error: failed to read operation file: {}",
                op_path.display()
            );
            return 1;
        }
    };

    let vars_min_limit = parse_u64_default(std::env::var("GQL_VARS_MIN_LIMIT").ok(), 5, 0);
    let (variables_note, variables_json_value) = match vars_path.as_deref() {
        None => (None, serde_json::json!({})),
        Some(p) => {
            match api_testing_core::graphql::vars::GraphqlVariablesFile::load(p, vars_min_limit) {
                Ok(v) => {
                    let note = if vars_min_limit > 0 && v.bumped_limit_fields > 0 {
                        Some(format!(
                        "> NOTE: variables normalized: bumped {} limit field(s) to at least {} (GQL_VARS_MIN_LIMIT).",
                        v.bumped_limit_fields, vars_min_limit
                    ))
                    } else {
                        None
                    };
                    (note, v.variables)
                }
                Err(err) => {
                    let _ = writeln!(stderr, "{err}");
                    return 1;
                }
            }
        }
    };

    let mut variables_json_value = variables_json_value;
    if !args.no_redact {
        let _ = api_testing_core::redact::redact_json(&mut variables_json_value);
    }
    let variables_json =
        api_testing_core::markdown::format_json_pretty_sorted(&variables_json_value)
            .unwrap_or_else(|_| variables_json_value.to_string());

    let mut response_note: Option<String> = None;
    let response_raw: Vec<u8> = if args.run {
        let mut run_stdout = Vec::new();
        let mut run_stderr = Vec::new();
        let call_args = CallArgs {
            env: args.env.clone(),
            url: args.url.clone(),
            jwt: args.jwt.clone(),
            config_dir: args.config_dir.clone(),
            list_envs: false,
            list_jwts: false,
            no_history: true,
            operation: Some(args.op.clone()),
            variables: args.vars.clone(),
        };
        let run_exit_code = cmd_call_internal(
            &call_args,
            invocation_dir,
            false,
            false,
            &mut run_stdout,
            &mut run_stderr,
        );
        if run_exit_code != 0 {
            let _ = stderr.write_all(&run_stderr);
            return 1;
        }
        run_stdout
    } else {
        match args.response.as_deref().and_then(trim_non_empty) {
            Some(resp) if resp == "-" => {
                let mut buf = Vec::new();
                if std::io::stdin().read_to_end(&mut buf).is_err() {
                    let _ = writeln!(stderr, "error: failed to read response from stdin");
                    return 1;
                }
                buf
            }
            Some(resp) => {
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
            None if allow_empty => {
                response_note = Some("> NOTE: run the operation and replace this section with the real response (formatted JSON).".to_string());
                serde_json::to_vec(&serde_json::json!({})).unwrap_or_default()
            }
            None => {
                let _ = writeln!(stderr, "error: Use either --run or --response.");
                return 1;
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

    if !allow_empty {
        if !args.run && args.response.as_deref().and_then(trim_non_empty).is_none() {
            let _ = writeln!(stderr, "Refusing to write a report without a real response. Use --run or --response (or pass --allow-empty for an intentionally empty/draft report).");
            return 1;
        }

        if response_json_for_eval.is_none() {
            let _ = writeln!(
                stderr,
                "Response is not JSON; refusing to write a no-data report. Re-run with --allow-empty if this is expected."
            );
            return 1;
        }

        if !response_has_meaningful_data_records(response_json_for_eval.as_ref().expect("json")) {
            let _ = writeln!(stderr, "Response appears to contain no data records; refusing to write report. Adjust query/variables to return at least one record, or pass --allow-empty if an empty result is expected.");
            return 1;
        }
    }

    let result_note = if args.run {
        "Result: PASS".to_string()
    } else if args.response.as_deref().and_then(trim_non_empty).is_some() {
        "Result: (response provided; request not executed)".to_string()
    } else {
        "Result: (not executed)".to_string()
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

    let report = api_testing_core::graphql::report::GraphqlReport {
        report_date,
        case_name: case_name.to_string(),
        generated_at,
        endpoint_note,
        result_note,
        command_snippet,
        operation: op_content,
        variables_note,
        variables_json,
        response_note,
        response_lang,
        response_body,
    };

    let markdown = api_testing_core::graphql::report::render_graphql_report_markdown(&report);
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
    let response_is_stdin = matches!(args.response.as_deref(), Some(resp) if resp.trim() == "-");
    if response_is_stdin && args.stdin {
        let _ = writeln!(
            stderr,
            "error: --stdin cannot be used with --response - (stdin is reserved for response body)"
        );
        return 1;
    }

    let snippet = if args.stdin {
        let mut buf = Vec::new();
        if std::io::stdin().read_to_end(&mut buf).is_err() {
            let _ = writeln!(stderr, "error: failed to read snippet from stdin");
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

    let api_testing_core::cmd_snippet::ReportFromCmd::Graphql(parsed) = parsed else {
        let _ = writeln!(
            stderr,
            "error: expected an api-gql/gql.sh snippet; got a non-GraphQL snippet"
        );
        return 1;
    };

    let api_testing_core::cmd_snippet::GraphqlReportFromCmd {
        case: derived_case,
        config_dir,
        env,
        url,
        jwt,
        op,
        vars,
    } = parsed;

    let case = args
        .case
        .as_deref()
        .and_then(trim_non_empty)
        .unwrap_or(derived_case);

    let report_args = ReportArgs {
        case,
        op,
        vars,
        out: args.out.clone(),
        env,
        url,
        jwt,
        run: args
            .response
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .is_none(),
        response: args.response.clone(),
        allow_empty: args.allow_empty,
        no_redact: false,
        no_command: false,
        no_command_url: false,
        project_root: None,
        config_dir,
    };

    if args.dry_run {
        let cmd = build_api_gql_report_dry_run_command(&report_args);
        let _ = writeln!(stdout, "{cmd}");
        return 0;
    }

    cmd_report(&report_args, invocation_dir, stdout, stderr)
}

fn build_report_command_snippet(
    args: &ReportArgs,
    project_root: &Path,
    include_command_url: bool,
) -> String {
    let op_arg = PathBuf::from(&args.op);
    let op_arg = if op_arg.is_absolute() {
        maybe_relpath(&op_arg, project_root)
    } else {
        args.op.clone()
    };

    let vars_arg = args
        .vars
        .as_deref()
        .and_then(trim_non_empty)
        .map(PathBuf::from);
    let vars_arg = vars_arg.map(|p| {
        if p.is_absolute() {
            maybe_relpath(&p, project_root)
        } else {
            p.to_string_lossy().to_string()
        }
    });

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
    out.push_str("api-gql call \\\n");
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
    if let Some(jwt) = args.jwt.as_deref().and_then(trim_non_empty) {
        out.push_str(&format!("  --jwt {} \\\n", shell_quote(&jwt)));
    }

    out.push_str(&format!("  {} \\\n", shell_quote(&op_arg)));
    if let Some(vars) = vars_arg {
        out.push_str(&format!("  {} \\\n", shell_quote(&vars)));
    }
    out.push_str("| jq .\n");
    out
}

fn cmd_schema(
    args: &SchemaArgs,
    invocation_dir: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let config_dir = args
        .config_dir
        .as_deref()
        .and_then(trim_non_empty)
        .map(PathBuf::from);

    let setup_dir = match config::resolve_gql_setup_dir_for_schema(
        invocation_dir,
        invocation_dir,
        config_dir.as_deref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };

    let schema_path = match api_testing_core::graphql::schema_file::resolve_schema_path(
        &setup_dir,
        args.file.as_deref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };

    if args.cat {
        match std::fs::read_to_string(&schema_path) {
            Ok(v) => {
                let _ = write!(stdout, "{v}");
                0
            }
            Err(_) => {
                let _ = writeln!(
                    stderr,
                    "error: failed to read schema file: {}",
                    schema_path.display()
                );
                1
            }
        }
    } else {
        let _ = writeln!(stdout, "{}", schema_path.display());
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    fn write_file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, contents).expect("write");
    }

    #[test]
    fn argv_with_default_command_inserts_call() {
        let argv = argv_with_default_command(&[]);
        assert_eq!(argv, vec!["api-gql".to_string()]);

        let argv = argv_with_default_command(&["--help".to_string()]);
        assert_eq!(argv, vec!["api-gql".to_string(), "--help".to_string()]);

        let argv = argv_with_default_command(&["history".to_string()]);
        assert_eq!(argv, vec!["api-gql".to_string(), "history".to_string()]);

        let argv = argv_with_default_command(&["ops/health.graphql".to_string()]);
        assert_eq!(
            argv,
            vec![
                "api-gql".to_string(),
                "call".to_string(),
                "ops/health.graphql".to_string()
            ]
        );
    }

    #[test]
    fn bool_from_env_parses_and_warns() {
        let mut stderr = Vec::new();
        let got = bool_from_env(Some("true".to_string()), "GQL_FOO", false, &mut stderr);
        assert_eq!(got, true);

        let mut stderr = Vec::new();
        let got = bool_from_env(Some("nope".to_string()), "GQL_FOO", true, &mut stderr);
        assert_eq!(got, false);
        let msg = String::from_utf8_lossy(&stderr);
        assert!(msg.contains("GQL_FOO must be true|false"));
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
    fn build_report_command_includes_expected_flags() {
        let args = ReportArgs {
            case: "Health".to_string(),
            op: "ops/health.graphql".to_string(),
            vars: Some("vars.json".to_string()),
            out: Some("docs/report.md".to_string()),
            env: Some("staging".to_string()),
            url: None,
            jwt: Some("svc".to_string()),
            run: true,
            response: None,
            allow_empty: true,
            no_redact: false,
            no_command: false,
            no_command_url: false,
            project_root: None,
            config_dir: Some("setup/graphql".to_string()),
        };

        let cmd = build_api_gql_report_dry_run_command(&args);
        assert!(cmd.contains("--case 'Health'"));
        assert!(cmd.contains("--op 'ops/health.graphql'"));
        assert!(cmd.contains("--vars 'vars.json'"));
        assert!(cmd.contains("--out 'docs/report.md'"));
        assert!(cmd.contains("--config-dir 'setup/graphql'"));
        assert!(cmd.contains("--env 'staging'"));
        assert!(cmd.contains("--jwt 'svc'"));
        assert!(cmd.contains("--run"));
        assert!(cmd.contains("--allow-empty"));
    }

    #[test]
    fn list_available_suffixes_parses_and_sorts() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("endpoints.env");
        write_file(
            &file,
            "export GQL_URL_PROD=http://prod\nGQL_URL_DEV=http://dev\nGQL_URL_=bad\nGQL_URL_FOO-BAR=http://x\nGQL_URL_TEST=http://t\nGQL_URL_TEST=http://t2\n",
        );

        let suffixes = list_available_suffixes(&file, "GQL_URL_");
        assert_eq!(suffixes, vec!["dev", "prod", "test"]);
    }

    #[test]
    fn resolve_endpoint_for_call_honors_url_and_env() {
        let tmp = TempDir::new().unwrap();
        let setup = tmp.path().join("setup/graphql");
        std::fs::create_dir_all(&setup).unwrap();
        write_file(
            &setup.join("endpoints.env"),
            "GQL_ENV_DEFAULT=prod\nGQL_URL_PROD=http://prod\nGQL_URL_STAGING=http://staging\n",
        );

        let args = CallArgs {
            env: None,
            url: Some("http://explicit".to_string()),
            jwt: None,
            config_dir: None,
            list_envs: false,
            list_jwts: false,
            no_history: false,
            operation: Some("ops/health.graphql".to_string()),
            variables: None,
        };
        let sel = resolve_endpoint_for_call(&args, &setup).unwrap();
        assert_eq!(sel.gql_url, "http://explicit");
        assert_eq!(sel.endpoint_label_used, "url");

        let args = CallArgs {
            env: Some("staging".to_string()),
            url: None,
            jwt: None,
            config_dir: None,
            list_envs: false,
            list_jwts: false,
            no_history: false,
            operation: Some("ops/health.graphql".to_string()),
            variables: None,
        };
        let sel = resolve_endpoint_for_call(&args, &setup).unwrap();
        assert_eq!(sel.gql_url, "http://staging");
        assert_eq!(sel.endpoint_label_used, "env");

        let args = CallArgs {
            env: Some("https://example.test/graphql".to_string()),
            url: None,
            jwt: None,
            config_dir: None,
            list_envs: false,
            list_jwts: false,
            no_history: false,
            operation: Some("ops/health.graphql".to_string()),
            variables: None,
        };
        let sel = resolve_endpoint_for_call(&args, &setup).unwrap();
        assert_eq!(sel.gql_url, "https://example.test/graphql");
        assert_eq!(sel.endpoint_label_used, "url");
    }

    #[test]
    fn resolve_endpoint_for_call_unknown_env_lists_available() {
        let tmp = TempDir::new().unwrap();
        let setup = tmp.path().join("setup/graphql");
        std::fs::create_dir_all(&setup).unwrap();
        write_file(
            &setup.join("endpoints.env"),
            "GQL_URL_PROD=http://prod\nGQL_URL_DEV=http://dev\n",
        );

        let args = CallArgs {
            env: Some("missing".to_string()),
            url: None,
            jwt: None,
            config_dir: None,
            list_envs: false,
            list_jwts: false,
            no_history: false,
            operation: Some("ops/health.graphql".to_string()),
            variables: None,
        };

        let err = resolve_endpoint_for_call(&args, &setup).unwrap_err();
        assert!(err.to_string().contains("Unknown --env 'missing'"));
        assert!(err.to_string().contains("prod"));
    }
}
