use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "codex-cli",
    version,
    about = "Codex CLI for nils-cli workspace"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Agent command group
    Agent(AgentArgs),
    /// Authentication command group
    Auth(AuthArgs),
    /// Diagnostics command group
    Diag(DiagArgs),
    /// Configuration command group
    Config(ConfigArgs),
    /// Starship integration command group
    Starship(StarshipArgs),
}

#[derive(Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: Option<AgentCommand>,
}

#[derive(Subcommand)]
pub enum AgentCommand {
    /// Run a raw prompt
    Prompt {
        #[arg(value_name = "prompt", num_args = 0..)]
        prompt: Vec<String>,
    },
    /// Get actionable engineering advice
    Advice {
        #[arg(value_name = "question", num_args = 0..)]
        question: Vec<String>,
    },
    /// Get an explanation for a concept
    Knowledge {
        #[arg(value_name = "concept", num_args = 0..)]
        concept: Vec<String>,
    },
    /// Run the semantic-commit workflow
    Commit {
        /// Push after committing
        #[arg(short = 'p', long = "push")]
        push: bool,
        /// Autostage changes before committing
        #[arg(short = 'a', long = "auto-stage")]
        auto_stage: bool,
        /// Extra prompt text
        #[arg(value_name = "extra", num_args = 0..)]
        extra: Vec<String>,
    },
}

#[derive(Args)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub command: Option<AuthCommand>,
}

#[derive(Subcommand)]
pub enum AuthCommand {
    /// Switch to a secret by name or email
    Use {
        #[arg(value_name = "target", num_args = 0..)]
        args: Vec<String>,
    },
    /// Refresh OAuth tokens
    Refresh {
        #[arg(value_name = "secret", num_args = 0..)]
        args: Vec<String>,
    },
    /// Refresh stale tokens across auth + secrets
    AutoRefresh,
    /// Show which secret matches CODEX_AUTH_FILE
    Current,
    /// Sync CODEX_AUTH_FILE back into matching secrets
    Sync,
}

#[derive(Args)]
pub struct DiagArgs {
    #[command(subcommand)]
    pub command: Option<DiagCommand>,
}

#[derive(Subcommand)]
pub enum DiagCommand {
    /// Rate-limits diagnostics
    RateLimits(RateLimitsArgs),
}

#[derive(Args)]
pub struct RateLimitsArgs {
    /// Clear starship cache before querying
    #[arg(short = 'c')]
    pub clear_cache: bool,
    /// Debug output
    #[arg(short = 'd', long = "debug")]
    pub debug: bool,
    /// Cached mode (no network)
    #[arg(long = "cached")]
    pub cached: bool,
    /// Disable refresh-on-401 behavior
    #[arg(long = "no-refresh-auth")]
    pub no_refresh_auth: bool,
    /// Output raw JSON
    #[arg(long = "json")]
    pub json: bool,
    /// Output a one-line summary
    #[arg(long = "one-line")]
    pub one_line: bool,
    /// Query all secrets under CODEX_SECRET_DIR
    #[arg(long = "all")]
    pub all: bool,
    /// Run concurrent async mode
    #[arg(long = "async")]
    pub async_mode: bool,
    /// Max concurrent jobs (async mode)
    #[arg(long = "jobs")]
    pub jobs: Option<String>,
    /// Optional secret.json
    pub secret: Option<String>,
}

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: Option<ConfigCommand>,
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Show current configuration
    Show,
    /// Set configuration value (current shell only)
    Set { key: String, value: String },
}

#[derive(Args)]
pub struct StarshipArgs {
    /// Hide the 5h window output
    #[arg(long = "no-5h")]
    pub no_5h: bool,
    /// Cache TTL
    #[arg(long = "ttl")]
    pub ttl: Option<String>,
    /// Reset time format (UTC)
    #[arg(long = "time-format")]
    pub time_format: Option<String>,
    /// Force a blocking refresh
    #[arg(long = "refresh")]
    pub refresh: bool,
    /// Exit 0 if starship is enabled
    #[arg(long = "is-enabled")]
    pub is_enabled: bool,
}
