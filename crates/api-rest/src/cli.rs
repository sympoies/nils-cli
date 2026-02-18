use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "api-rest",
    version,
    about = "REST API runner (call/history/report)",
    disable_help_subcommand = true
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Execute a request file and print the response body to stdout (default)
    Call(CallArgs),
    /// Print the last (or last N) history entries
    History(HistoryArgs),
    /// Generate a Markdown API test report
    Report(ReportArgs),
    /// Generate a Markdown API test report from a saved `call` command snippet
    ReportFromCmd(ReportFromCmdArgs),
    /// Print shell completion script
    Completion(CompletionArgs),
}

#[derive(Args)]
pub(crate) struct CompletionArgs {
    /// Shell to generate completion for
    #[arg(value_enum)]
    pub(crate) shell: crate::completion::CompletionShell,
}

#[derive(Args)]
pub(crate) struct CallArgs {
    /// Endpoint preset name (or literal URL if it starts with http(s)://)
    #[arg(short = 'e', long = "env")]
    pub(crate) env: Option<String>,

    /// Explicit REST base URL
    #[arg(short = 'u', long = "url")]
    pub(crate) url: Option<String>,

    /// Token profile name
    #[arg(long = "token")]
    pub(crate) token: Option<String>,

    /// REST setup dir (discovery seed)
    #[arg(long = "config-dir")]
    pub(crate) config_dir: Option<String>,

    /// Disable writing to .rest_history for this run
    #[arg(long = "no-history")]
    pub(crate) no_history: bool,

    /// Request file path (*.request.json)
    #[arg(value_name = "request.request.json")]
    pub(crate) request: String,
}

#[derive(Args)]
pub(crate) struct HistoryArgs {
    /// REST setup dir (discovery seed)
    #[arg(long = "config-dir")]
    pub(crate) config_dir: Option<String>,

    /// Explicit history file path (relative paths resolve under setup dir)
    #[arg(long = "file")]
    pub(crate) file: Option<String>,

    /// Print the last entry (default)
    #[arg(long = "last", conflicts_with = "tail")]
    pub(crate) last: bool,

    /// Print the last N entries (blank-line separated)
    #[arg(long = "tail")]
    pub(crate) tail: Option<u32>,

    /// Omit metadata lines (starting with "#") from each entry
    #[arg(long = "command-only")]
    pub(crate) command_only: bool,
}

#[derive(Args)]
pub(crate) struct ReportArgs {
    /// Report case name
    #[arg(long = "case")]
    pub(crate) case: String,

    /// Request file path (*.request.json)
    #[arg(long = "request")]
    pub(crate) request: String,

    /// Output report path (default: <project_root>/docs/<stamp>-<case>-api-test-report.md)
    #[arg(long = "out")]
    pub(crate) out: Option<String>,

    /// Endpoint preset name (passed through)
    #[arg(short = 'e', long = "env")]
    pub(crate) env: Option<String>,

    /// Explicit REST base URL (passed through)
    #[arg(short = 'u', long = "url")]
    pub(crate) url: Option<String>,

    /// Token profile name (passed through)
    #[arg(long = "token")]
    pub(crate) token: Option<String>,

    /// Execute the request and embed the response
    #[arg(
        long = "run",
        conflicts_with = "response",
        required_unless_present = "response"
    )]
    pub(crate) run: bool,

    /// Use an existing response file (or "-" for stdin)
    #[arg(
        long = "response",
        conflicts_with = "run",
        required_unless_present = "run"
    )]
    pub(crate) response: Option<String>,

    /// Do not redact secrets in request/response JSON blocks
    #[arg(long = "no-redact")]
    pub(crate) no_redact: bool,

    /// Omit the command snippet section
    #[arg(long = "no-command")]
    pub(crate) no_command: bool,

    /// When using --url, omit the URL value in the command snippet
    #[arg(long = "no-command-url")]
    pub(crate) no_command_url: bool,

    /// Override project root (default: git root or CWD)
    #[arg(long = "project-root")]
    pub(crate) project_root: Option<String>,

    /// REST setup dir (passed through)
    #[arg(long = "config-dir")]
    pub(crate) config_dir: Option<String>,
}

#[derive(Args)]
pub(crate) struct ReportFromCmdArgs {
    /// Report case name (default: derived from the snippet)
    #[arg(long = "case")]
    pub(crate) case: Option<String>,

    /// Output report path (default: <project_root>/docs/<stamp>-<case>-api-test-report.md)
    #[arg(long = "out")]
    pub(crate) out: Option<String>,

    /// Use an existing response file (or "-" for stdin)
    ///
    /// Note: when using "--response -", stdin is reserved for the response body; provide the snippet as a positional argument.
    #[arg(long = "response")]
    pub(crate) response: Option<String>,

    /// Allow generating a report with an empty/no-data response (no-op for api-rest; kept for parity)
    #[arg(long = "allow-empty", alias = "expect-empty")]
    pub(crate) allow_empty: bool,

    /// Print the equivalent `api-rest report ...` command and exit 0
    #[arg(long = "dry-run")]
    pub(crate) dry_run: bool,

    /// Read the command snippet from stdin
    #[arg(long = "stdin", conflicts_with = "snippet")]
    pub(crate) stdin: bool,

    /// Command snippet (e.g. from `api-rest history --command-only`)
    #[arg(value_name = "snippet", required_unless_present = "stdin")]
    pub(crate) snippet: Option<String>,
}
