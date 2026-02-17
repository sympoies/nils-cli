use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::model::{
    BaselineTarget, Context, DocumentWhen, FallbackMode, OutputFormat, ResolveFormat, Scope,
};

#[derive(Debug, Parser)]
#[command(
    name = "agent-docs",
    version,
    about = "Deterministic required-doc discovery for agent workflows",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    pub agent_home: Option<PathBuf>,

    #[arg(long, global = true, value_name = "PATH")]
    pub project_path: Option<PathBuf>,

    #[arg(
        long = "worktree-fallback",
        global = true,
        value_enum,
        default_value_t = FallbackMode::Auto,
        value_name = "MODE",
        help = "Project worktree fallback mode",
        long_help = "Project worktree fallback mode. auto enables linked-worktree fallback to the primary worktree; local-only disables fallback and enforces local project files only."
    )]
    pub worktree_fallback: FallbackMode,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Resolve(ResolveArgs),
    Contexts(ContextsArgs),
    Add(AddArgs),
    ScaffoldAgents(ScaffoldAgentsArgs),
    Baseline(BaselineArgs),
    ScaffoldBaseline(ScaffoldBaselineArgs),
}

#[derive(Debug, Args)]
pub struct ResolveArgs {
    #[arg(long, value_enum)]
    pub context: Context,

    #[arg(long, value_enum, default_value_t = ResolveFormat::Text)]
    pub format: ResolveFormat,

    #[arg(long)]
    pub strict: bool,
}

#[derive(Debug, Args)]
pub struct ContextsArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    #[arg(long, value_enum)]
    pub target: Scope,

    #[arg(long, value_enum)]
    pub context: Context,

    #[arg(long, value_enum)]
    pub scope: Scope,

    #[arg(long, value_name = "PATH")]
    pub path: PathBuf,

    #[arg(long)]
    pub required: bool,

    #[arg(long, value_enum, default_value_t = DocumentWhen::Always)]
    pub when: DocumentWhen,

    #[arg(long)]
    pub notes: Option<String>,
}

#[derive(Debug, Args)]
pub struct ScaffoldAgentsArgs {
    #[arg(long, value_enum)]
    pub target: Scope,

    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,

    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct BaselineArgs {
    #[arg(long)]
    pub check: bool,

    #[arg(long, value_enum, default_value_t = BaselineTarget::All)]
    pub target: BaselineTarget,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    #[arg(long)]
    pub strict: bool,
}

#[derive(Debug, Args)]
pub struct ScaffoldBaselineArgs {
    #[arg(long, value_enum, default_value_t = BaselineTarget::All)]
    pub target: BaselineTarget,

    #[arg(long)]
    pub missing_only: bool,

    #[arg(long)]
    pub force: bool,

    #[arg(long)]
    pub dry_run: bool,

    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}
