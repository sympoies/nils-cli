use std::env;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::errors::AppError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    Text,
    Json,
}

impl OutputMode {
    pub fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ItemState {
    All,
    Pending,
    Enriched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReportPeriod {
    Week,
    Month,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FetchState {
    Pending,
}

#[derive(Debug, Parser)]
#[command(
    name = "memo-cli",
    version,
    about = "Capture-first memo CLI with agent enrichment"
)]
pub struct Cli {
    /// SQLite file path
    #[arg(long, global = true, value_name = "path", default_value_os_t = default_db_path())]
    pub db: PathBuf,

    /// Output JSON (shorthand for --format json)
    #[arg(long, global = true)]
    pub json: bool,

    /// Output format
    #[arg(long, global = true, value_enum)]
    pub format: Option<OutputFormat>,

    #[command(subcommand)]
    pub command: MemoCommand,
}

#[derive(Debug, Subcommand)]
pub enum MemoCommand {
    /// Capture one raw memo entry
    Add(AddArgs),
    /// List memo entries in deterministic order
    List(ListArgs),
    /// Search memo entries with FTS-backed query
    Search(SearchArgs),
    /// Show weekly or monthly summary report
    Report(ReportArgs),
    /// Fetch pending items for agent enrichment
    Fetch(FetchArgs),
    /// Apply enrichment payloads
    Apply(ApplyArgs),
}

#[derive(Debug, clap::Args)]
pub struct AddArgs {
    /// Memo text
    pub text: String,

    /// Capture source label
    #[arg(long, default_value = "cli")]
    pub source: String,
}

#[derive(Debug, clap::Args)]
pub struct ListArgs {
    /// Max rows to return
    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    /// Row offset for paging
    #[arg(long, default_value_t = 0)]
    pub offset: usize,

    /// Row selection mode
    #[arg(long, value_enum, default_value_t = ItemState::All)]
    pub state: ItemState,
}

#[derive(Debug, clap::Args)]
pub struct SearchArgs {
    /// Search query text (FTS syntax)
    pub query: String,

    /// Max rows to return
    #[arg(long, default_value_t = 20)]
    pub limit: usize,

    /// Row selection mode
    #[arg(long, value_enum, default_value_t = ItemState::All)]
    pub state: ItemState,
}

#[derive(Debug, clap::Args)]
pub struct ReportArgs {
    /// Report period: week or month
    pub period: ReportPeriod,
}

#[derive(Debug, clap::Args)]
pub struct FetchArgs {
    /// Max rows to return
    #[arg(long, default_value_t = 50)]
    pub limit: usize,

    /// Optional cursor (reserved for future pagination)
    #[arg(long)]
    pub cursor: Option<String>,

    /// Fetch selection mode
    #[arg(long, value_enum, default_value_t = FetchState::Pending)]
    pub state: FetchState,
}

#[derive(Debug, clap::Args)]
pub struct ApplyArgs {
    /// JSON file containing apply payload
    #[arg(long)]
    pub input: Option<PathBuf>,

    /// Read payload JSON from stdin
    #[arg(long)]
    pub stdin: bool,

    /// Validate payload without write-back
    #[arg(long)]
    pub dry_run: bool,
}

impl Cli {
    pub fn resolve_output_mode(&self) -> Result<OutputMode, AppError> {
        if self.json && matches!(self.format, Some(OutputFormat::Text)) {
            return Err(AppError::usage(
                "invalid output mode: --json cannot be combined with --format text",
            ));
        }

        if self.json || matches!(self.format, Some(OutputFormat::Json)) {
            return Ok(OutputMode::Json);
        }

        Ok(OutputMode::Text)
    }

    pub fn command_id(&self) -> &'static str {
        match self.command {
            MemoCommand::Add(_) => "memo-cli add",
            MemoCommand::List(_) => "memo-cli list",
            MemoCommand::Search(_) => "memo-cli search",
            MemoCommand::Report(_) => "memo-cli report",
            MemoCommand::Fetch(_) => "memo-cli fetch",
            MemoCommand::Apply(_) => "memo-cli apply",
        }
    }

    pub fn schema_version(&self) -> &'static str {
        match self.command {
            MemoCommand::Add(_) => "memo-cli.add.v1",
            MemoCommand::List(_) => "memo-cli.list.v1",
            MemoCommand::Search(_) => "memo-cli.search.v1",
            MemoCommand::Report(_) => "memo-cli.report.v1",
            MemoCommand::Fetch(_) => "memo-cli.fetch.v1",
            MemoCommand::Apply(_) => "memo-cli.apply.v1",
        }
    }
}

fn default_db_path() -> PathBuf {
    if let Some(data_home) = env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(data_home).join("nils-cli").join("memo.db");
    }

    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("nils-cli")
            .join("memo.db");
    }

    PathBuf::from("memo.db")
}

#[cfg(test)]
pub(crate) mod tests {
    use clap::{CommandFactory, Parser};

    use super::{Cli, OutputMode};

    #[test]
    fn output_mode_defaults_to_text() {
        let cli = Cli::parse_from(["memo-cli", "list"]);
        let mode = cli.resolve_output_mode().expect("mode should resolve");
        assert_eq!(mode, OutputMode::Text);
    }

    #[test]
    fn output_mode_json_flag_wins() {
        let cli = Cli::parse_from(["memo-cli", "--json", "list"]);
        let mode = cli.resolve_output_mode().expect("mode should resolve");
        assert_eq!(mode, OutputMode::Json);
    }

    #[test]
    fn output_mode_format_json_is_supported() {
        let cli = Cli::parse_from(["memo-cli", "--format", "json", "list"]);
        let mode = cli.resolve_output_mode().expect("mode should resolve");
        assert_eq!(mode, OutputMode::Json);
    }

    #[test]
    fn output_mode_rejects_conflict() {
        let cli = Cli::parse_from(["memo-cli", "--json", "--format", "text", "list"]);
        let err = cli.resolve_output_mode().expect_err("conflict should fail");
        assert_eq!(err.exit_code(), 64);
    }

    #[test]
    fn parser_exposes_expected_subcommands() {
        let mut cmd = Cli::command();
        let subcommands = cmd
            .get_subcommands_mut()
            .map(|sub| sub.get_name().to_string())
            .collect::<Vec<_>>();
        assert!(subcommands.contains(&"add".to_string()));
        assert!(subcommands.contains(&"list".to_string()));
        assert!(subcommands.contains(&"search".to_string()));
        assert!(subcommands.contains(&"report".to_string()));
        assert!(subcommands.contains(&"fetch".to_string()));
        assert!(subcommands.contains(&"apply".to_string()));
    }
}
