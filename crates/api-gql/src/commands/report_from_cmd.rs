use std::io::{Read, Write};

use api_testing_core::cli_util::{shell_quote, trim_non_empty};

use crate::cli::{ReportArgs, ReportFromCmdArgs};
use crate::commands::report::cmd_report;

pub(crate) fn cmd_report_from_cmd(
    args: &ReportFromCmdArgs,
    invocation_dir: &std::path::Path,
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

pub(crate) fn build_api_gql_report_dry_run_command(args: &ReportArgs) -> String {
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
