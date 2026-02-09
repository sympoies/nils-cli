use std::path::Path;

use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdSnippetKind {
    Graphql,
    Rest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmdSnippet {
    Graphql(GraphqlCallSnippet),
    Rest(RestCallSnippet),
}

impl CmdSnippet {
    pub fn kind(&self) -> CmdSnippetKind {
        match self {
            CmdSnippet::Graphql(_) => CmdSnippetKind::Graphql,
            CmdSnippet::Rest(_) => CmdSnippetKind::Rest,
        }
    }

    pub fn command_basename(&self) -> &str {
        match self {
            CmdSnippet::Graphql(s) => &s.command_basename,
            CmdSnippet::Rest(s) => &s.command_basename,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlCallSnippet {
    pub command_basename: String,
    pub config_dir: Option<String>,
    pub env: Option<String>,
    pub url: Option<String>,
    pub jwt: Option<String>,
    pub operation: String,
    pub variables: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestCallSnippet {
    pub command_basename: String,
    pub config_dir: Option<String>,
    pub env: Option<String>,
    pub url: Option<String>,
    pub token: Option<String>,
    pub request: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportFromCmd {
    Graphql(GraphqlReportFromCmd),
    Rest(RestReportFromCmd),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlReportFromCmd {
    pub case: String,
    pub config_dir: Option<String>,
    pub env: Option<String>,
    pub url: Option<String>,
    pub jwt: Option<String>,
    pub op: String,
    pub vars: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestReportFromCmd {
    pub case: String,
    pub config_dir: Option<String>,
    pub env: Option<String>,
    pub url: Option<String>,
    pub token: Option<String>,
    pub request: String,
}

#[derive(Debug, Error)]
pub enum CmdSnippetError {
    #[error("command snippet is empty")]
    EmptySnippet,

    #[error("failed to tokenize snippet: {message}")]
    TokenizeFailed { message: String },

    #[error("unsupported command: {command}")]
    UnsupportedCommand { command: String },

    #[error("expected a `call` snippet; found subcommand: {subcommand}")]
    UnsupportedSubcommand { subcommand: String },

    #[error("flag {flag} requires a value")]
    MissingFlagValue { flag: String },

    #[error("unknown flag: {flag}")]
    UnknownFlag { flag: String },

    #[error("missing GraphQL operation file path (*.graphql)")]
    MissingGraphqlOperation,

    #[error("missing REST request file path (*.request.json)")]
    MissingRestRequest,

    #[error("unexpected extra argument: {arg}")]
    UnexpectedArg { arg: String },
}

pub fn parse_call_snippet(snippet: &str) -> Result<CmdSnippet, CmdSnippetError> {
    let tokens = tokenize_call_snippet(snippet)?;
    let (cmd, rest) = match tokens.split_first() {
        Some(v) => v,
        None => return Err(CmdSnippetError::EmptySnippet),
    };

    let cmd_base = basename(cmd);
    match cmd_base.as_str() {
        "api-gql" | "gql.sh" => Ok(CmdSnippet::Graphql(parse_graphql_call_args(
            cmd_base, rest,
        )?)),
        "api-rest" | "rest.sh" => Ok(CmdSnippet::Rest(parse_rest_call_args(cmd_base, rest)?)),
        _ => Err(CmdSnippetError::UnsupportedCommand { command: cmd_base }),
    }
}

pub fn parse_report_from_cmd_snippet(snippet: &str) -> Result<ReportFromCmd, CmdSnippetError> {
    let parsed = parse_call_snippet(snippet)?;
    Ok(match parsed {
        CmdSnippet::Graphql(s) => ReportFromCmd::Graphql(graphql_to_report_from_cmd(&s)),
        CmdSnippet::Rest(s) => ReportFromCmd::Rest(rest_to_report_from_cmd(&s)),
    })
}

fn graphql_to_report_from_cmd(s: &GraphqlCallSnippet) -> GraphqlReportFromCmd {
    GraphqlReportFromCmd {
        case: derive_graphql_case_name(s),
        config_dir: s.config_dir.clone(),
        env: s.env.clone(),
        url: s.url.clone(),
        jwt: s.jwt.clone(),
        op: s.operation.clone(),
        vars: s.variables.clone(),
    }
}

fn rest_to_report_from_cmd(s: &RestCallSnippet) -> RestReportFromCmd {
    RestReportFromCmd {
        case: derive_rest_case_name(s),
        config_dir: s.config_dir.clone(),
        env: s.env.clone(),
        url: s.url.clone(),
        token: s.token.clone(),
        request: s.request.clone(),
    }
}

fn parse_graphql_call_args(
    command_basename: String,
    raw_args: &[String],
) -> Result<GraphqlCallSnippet, CmdSnippetError> {
    let mut config_dir: Option<String> = None;
    let mut env: Option<String> = None;
    let mut url: Option<String> = None;
    let mut jwt: Option<String> = None;

    let mut args: Vec<String> = raw_args.to_vec();
    if let Some(first) = args.first().cloned()
        && !first.starts_with('-')
        && first != "--"
    {
        if first == "call" {
            args.remove(0);
        } else if matches!(first.as_str(), "history" | "report" | "schema") {
            return Err(CmdSnippetError::UnsupportedSubcommand { subcommand: first });
        }
    }

    let mut positional: Vec<String> = Vec::new();
    let mut i: usize = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            positional.extend(args[i + 1..].iter().cloned());
            break;
        }

        if arg == "--no-history" || arg == "--list-envs" || arg == "--list-jwts" {
            i += 1;
            continue;
        }

        if let Some(v) = flag_value_eq(arg, "--config-dir") {
            config_dir = Some(v?);
            i += 1;
            continue;
        }
        if arg == "--config-dir" {
            config_dir = Some(take_value(&args, i, "--config-dir")?);
            i += 2;
            continue;
        }

        if let Some(v) = flag_value_eq(arg, "--env") {
            env = Some(v?);
            i += 1;
            continue;
        }
        if arg == "--env" || arg == "-e" {
            env = Some(take_value(&args, i, arg)?);
            i += 2;
            continue;
        }

        if let Some(v) = flag_value_eq(arg, "--url") {
            url = Some(v?);
            i += 1;
            continue;
        }
        if arg == "--url" || arg == "-u" {
            url = Some(take_value(&args, i, arg)?);
            i += 2;
            continue;
        }

        if let Some(v) = flag_value_eq(arg, "--jwt") {
            jwt = Some(v?);
            i += 1;
            continue;
        }
        if arg == "--jwt" {
            jwt = Some(take_value(&args, i, "--jwt")?);
            i += 2;
            continue;
        }

        if arg.starts_with('-') {
            return Err(CmdSnippetError::UnknownFlag {
                flag: arg.to_string(),
            });
        }

        positional.push(arg.to_string());
        i += 1;
    }

    let operation = positional
        .first()
        .cloned()
        .ok_or(CmdSnippetError::MissingGraphqlOperation)?;
    let variables = positional.get(1).cloned();
    if let Some(extra) = positional.get(2) {
        return Err(CmdSnippetError::UnexpectedArg { arg: extra.clone() });
    }

    Ok(GraphqlCallSnippet {
        command_basename,
        config_dir,
        env,
        url,
        jwt,
        operation,
        variables,
    })
}

fn parse_rest_call_args(
    command_basename: String,
    raw_args: &[String],
) -> Result<RestCallSnippet, CmdSnippetError> {
    let mut config_dir: Option<String> = None;
    let mut env: Option<String> = None;
    let mut url: Option<String> = None;
    let mut token: Option<String> = None;

    let mut args: Vec<String> = raw_args.to_vec();
    if let Some(first) = args.first().cloned()
        && !first.starts_with('-')
        && first != "--"
    {
        if first == "call" {
            args.remove(0);
        } else if matches!(first.as_str(), "history" | "report") {
            return Err(CmdSnippetError::UnsupportedSubcommand { subcommand: first });
        }
    }

    let mut positional: Vec<String> = Vec::new();
    let mut i: usize = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            positional.extend(args[i + 1..].iter().cloned());
            break;
        }

        if arg == "--no-history" {
            i += 1;
            continue;
        }

        if let Some(v) = flag_value_eq(arg, "--config-dir") {
            config_dir = Some(v?);
            i += 1;
            continue;
        }
        if arg == "--config-dir" {
            config_dir = Some(take_value(&args, i, "--config-dir")?);
            i += 2;
            continue;
        }

        if let Some(v) = flag_value_eq(arg, "--env") {
            env = Some(v?);
            i += 1;
            continue;
        }
        if arg == "--env" || arg == "-e" {
            env = Some(take_value(&args, i, arg)?);
            i += 2;
            continue;
        }

        if let Some(v) = flag_value_eq(arg, "--url") {
            url = Some(v?);
            i += 1;
            continue;
        }
        if arg == "--url" || arg == "-u" {
            url = Some(take_value(&args, i, arg)?);
            i += 2;
            continue;
        }

        if let Some(v) = flag_value_eq(arg, "--token") {
            token = Some(v?);
            i += 1;
            continue;
        }
        if arg == "--token" {
            token = Some(take_value(&args, i, "--token")?);
            i += 2;
            continue;
        }

        if arg.starts_with('-') {
            return Err(CmdSnippetError::UnknownFlag {
                flag: arg.to_string(),
            });
        }

        positional.push(arg.to_string());
        i += 1;
    }

    let request = positional
        .first()
        .cloned()
        .ok_or(CmdSnippetError::MissingRestRequest)?;
    if let Some(extra) = positional.get(1) {
        return Err(CmdSnippetError::UnexpectedArg { arg: extra.clone() });
    }

    Ok(RestCallSnippet {
        command_basename,
        config_dir,
        env,
        url,
        token,
        request,
    })
}

fn tokenize_call_snippet(snippet: &str) -> Result<Vec<String>, CmdSnippetError> {
    let raw = snippet.trim();
    if raw.is_empty() {
        return Err(CmdSnippetError::EmptySnippet);
    }

    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let continued = remove_line_continuations(&normalized);
    let expanded = expand_env_vars_best_effort(&continued);
    let expanded = expanded.replace('\n', " ");

    let mut tokens =
        shell_words::split(&expanded).map_err(|err| CmdSnippetError::TokenizeFailed {
            message: err.to_string(),
        })?;

    if let Some(pipe_idx) = tokens.iter().position(|t| t == "|") {
        tokens.truncate(pipe_idx);
    }

    Ok(tokens)
}

fn remove_line_continuations(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && matches!(chars.peek(), Some('\n')) {
            let _ = chars.next();
            continue;
        }
        out.push(ch);
    }
    out
}

fn expand_env_vars_best_effort(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
                out.push(ch);
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
                out.push(ch);
            }
            '\\' => {
                if matches!(chars.peek(), Some('$')) && !in_single_quote {
                    let _ = chars.next();
                    out.push('$');
                    continue;
                }
                out.push(ch);
            }
            '$' if !in_single_quote => {
                if matches!(chars.peek(), Some('{')) {
                    let _ = chars.next();
                    let mut name = String::new();
                    while let Some(&c) = chars.peek() {
                        chars.next();
                        if c == '}' {
                            break;
                        }
                        name.push(c);
                    }
                    if name.is_empty() {
                        out.push('$');
                        out.push_str("{}");
                        continue;
                    }
                    match std::env::var(&name) {
                        Ok(v) => out.push_str(&v),
                        Err(_) => {
                            out.push_str("${");
                            out.push_str(&name);
                            out.push('}');
                        }
                    }
                    continue;
                }

                let mut name = String::new();
                while let Some(&c) = chars.peek() {
                    if name.is_empty() {
                        if c.is_ascii_alphabetic() || c == '_' {
                            name.push(c);
                            chars.next();
                            continue;
                        }
                        break;
                    }
                    if c.is_ascii_alphanumeric() || c == '_' {
                        name.push(c);
                        chars.next();
                        continue;
                    }
                    break;
                }

                if name.is_empty() {
                    out.push('$');
                    continue;
                }

                match std::env::var(&name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('$');
                        out.push_str(&name);
                    }
                }
            }
            _ => out.push(ch),
        }
    }

    out
}

fn flag_value_eq(arg: &str, flag: &str) -> Option<Result<String, CmdSnippetError>> {
    arg.strip_prefix(&format!("{flag}=")).map(|v| {
        if v.is_empty() {
            Err(CmdSnippetError::MissingFlagValue {
                flag: flag.to_string(),
            })
        } else {
            Ok(v.to_string())
        }
    })
}

fn take_value(args: &[String], idx: usize, flag: &str) -> Result<String, CmdSnippetError> {
    args.get(idx + 1)
        .cloned()
        .ok_or_else(|| CmdSnippetError::MissingFlagValue {
            flag: flag.to_string(),
        })
}

fn basename(path: &str) -> String {
    let p = Path::new(path);
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

fn stem_for_operation(path: &str) -> String {
    let name = basename(path);
    if let Some(stem) = name.strip_suffix(".graphql") {
        return stem.to_string();
    }
    Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or(name)
}

fn stem_for_request(path: &str) -> String {
    let name = basename(path);
    if let Some(stem) = name.strip_suffix(".request.json") {
        return stem.to_string();
    }
    Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or(name)
}

fn derive_graphql_case_name(s: &GraphqlCallSnippet) -> String {
    let stem = stem_for_operation(&s.operation);
    let stem = if stem.trim().is_empty() {
        "case".to_string()
    } else {
        stem
    };

    let env_or_url = s.url.as_deref().or(s.env.as_deref()).unwrap_or("implicit");
    let mut meta: Vec<String> = vec![env_or_url.to_string()];
    if let Some(jwt) = s.jwt.as_deref() {
        meta.push(format!("jwt:{jwt}"));
    }

    format!("{stem} ({})", meta.join(", "))
}

fn derive_rest_case_name(s: &RestCallSnippet) -> String {
    let stem = stem_for_request(&s.request);
    let stem = if stem.trim().is_empty() {
        "case".to_string()
    } else {
        stem
    };

    let env_or_url = s.url.as_deref().or(s.env.as_deref()).unwrap_or("implicit");
    let mut meta: Vec<String> = vec![env_or_url.to_string()];
    if let Some(token) = s.token.as_deref() {
        meta.push(format!("token:{token}"));
    }

    format!("{stem} ({})", meta.join(", "))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use pretty_assertions::assert_eq;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn tokenization_truncates_at_first_pipe() {
        let s = "api-gql call --env staging op.graphql | jq .";
        let tokens = tokenize_call_snippet(s).expect("tokens");
        assert_eq!(
            tokens,
            vec!["api-gql", "call", "--env", "staging", "op.graphql"]
        );
    }

    #[test]
    fn tokenization_removes_backslash_newline() {
        let s = "api-gql call --env staging \\\n op.graphql";
        let tokens = tokenize_call_snippet(s).expect("tokens");
        assert_eq!(
            tokens,
            vec!["api-gql", "call", "--env", "staging", "op.graphql"]
        );
    }

    #[test]
    fn tokenization_expands_env_vars_best_effort() {
        let _g = ENV_LOCK.lock().expect("lock");
        let key = "NILS_TEST_HOME";
        let prev = std::env::var(key).ok();
        // SAFETY: tests mutate process env while guarded by ENV_LOCK.
        unsafe { std::env::set_var(key, "/tmp/nils-test-home") };

        let s = "$NILS_TEST_HOME/bin/api-gql call --env staging op.graphql";
        let tokens = tokenize_call_snippet(s).expect("tokens");
        assert_eq!(
            tokens,
            vec![
                "/tmp/nils-test-home/bin/api-gql",
                "call",
                "--env",
                "staging",
                "op.graphql"
            ]
        );

        if let Some(v) = prev {
            // SAFETY: tests restore process env while guarded by ENV_LOCK.
            unsafe { std::env::set_var(key, v) };
        } else {
            // SAFETY: tests restore process env while guarded by ENV_LOCK.
            unsafe { std::env::remove_var(key) };
        }
    }

    #[test]
    fn parses_graphql_call_and_ignores_command_path_prefix() {
        let s = "/usr/local/bin/api-gql call --env staging --jwt service setup/graphql/operations/health.graphql";
        let parsed = parse_call_snippet(s).expect("parse");
        let CmdSnippet::Graphql(gql) = parsed else {
            panic!("expected graphql");
        };
        assert_eq!(gql.command_basename, "api-gql");
        assert_eq!(gql.env.as_deref(), Some("staging"));
        assert_eq!(gql.jwt.as_deref(), Some("service"));
        assert_eq!(
            gql.operation,
            "setup/graphql/operations/health.graphql".to_string()
        );
    }

    #[test]
    fn graphql_missing_operation_is_error() {
        let s = "api-gql call --env staging";
        let err = parse_call_snippet(s).expect_err("expected err");
        assert!(matches!(err, CmdSnippetError::MissingGraphqlOperation));
    }

    #[test]
    fn graphql_case_is_derived_from_op_and_meta() {
        let s = "api-gql call --env staging --jwt service setup/graphql/operations/health.graphql";
        let ReportFromCmd::Graphql(report) = parse_report_from_cmd_snippet(s).expect("parse")
        else {
            panic!("expected graphql");
        };
        assert_eq!(report.case, "health (staging, jwt:service)");
    }

    fn assert_missing_flag_value(snippet: &str, expected_flag: &str) {
        let err = parse_call_snippet(snippet).expect_err("expected err");
        match err {
            CmdSnippetError::MissingFlagValue { flag } => assert_eq!(flag, expected_flag),
            _ => panic!("expected missing flag value error"),
        }
    }

    #[test]
    fn graphql_empty_flag_values_are_errors() {
        let cases = [
            ("--env=", "--env"),
            ("--url=", "--url"),
            ("--jwt=", "--jwt"),
            ("--config-dir=", "--config-dir"),
        ];
        for (flag, expected) in cases {
            let s = format!("api-gql call {flag} setup/graphql/operations/health.graphql");
            assert_missing_flag_value(&s, expected);
        }
    }

    #[test]
    fn rest_missing_request_is_error() {
        let s = "api-rest call --env staging";
        let err = parse_call_snippet(s).expect_err("expected err");
        assert!(matches!(err, CmdSnippetError::MissingRestRequest));
    }

    #[test]
    fn rest_case_is_derived_from_request_and_meta() {
        let s =
            "api-rest call --env staging --token service setup/rest/requests/health.request.json";
        let ReportFromCmd::Rest(report) = parse_report_from_cmd_snippet(s).expect("parse") else {
            panic!("expected rest");
        };
        assert_eq!(report.case, "health (staging, token:service)");
    }

    #[test]
    fn rest_empty_flag_values_are_errors() {
        let cases = [
            ("--env=", "--env"),
            ("--url=", "--url"),
            ("--token=", "--token"),
        ];
        for (flag, expected) in cases {
            let s = format!("api-rest call {flag} setup/rest/requests/health.request.json");
            assert_missing_flag_value(&s, expected);
        }
    }
}
