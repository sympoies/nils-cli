use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "claude-cli",
    version,
    about = "Claude CLI for nils-cli workspace"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Agent command group
    Agent(AgentArgs),
    /// Auth state command group
    AuthState(AuthStateArgs),
    /// Diagnostics command group
    Diag(DiagArgs),
    /// Configuration command group
    Config(ConfigArgs),
    /// Export shell completion script
    Completion(CompletionArgs),
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
}

#[derive(Args)]
pub struct AuthStateArgs {
    #[command(subcommand)]
    pub command: Option<AuthStateCommand>,
}

#[derive(Subcommand)]
pub enum AuthStateCommand {
    /// Show Claude auth state
    Show {
        #[command(flatten)]
        output: OutputModeArgs,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    #[value(name = "text")]
    Text,
    #[value(name = "json")]
    Json,
}

#[derive(Args, Clone, Debug, Default)]
pub struct OutputModeArgs {
    /// Output format (`text` or `json`)
    #[arg(long = "format", value_enum, value_name = "format")]
    pub format: Option<OutputFormat>,
    /// Output machine-readable JSON
    #[arg(long = "json", conflicts_with = "format")]
    pub json: bool,
}

impl OutputModeArgs {
    pub fn is_json(&self) -> bool {
        self.json || matches!(self.format, Some(OutputFormat::Json))
    }
}

#[derive(Args)]
pub struct DiagArgs {
    #[command(subcommand)]
    pub command: Option<DiagCommand>,
}

#[derive(Subcommand)]
pub enum DiagCommand {
    /// Claude readiness diagnostics
    Healthcheck {
        #[command(flatten)]
        output: OutputModeArgs,
        /// Optional healthcheck timeout in milliseconds
        #[arg(long = "timeout-ms")]
        timeout_ms: Option<u64>,
    },
    /// Codex-only rate-limits surface (unsupported)
    RateLimits {
        #[command(flatten)]
        output: OutputModeArgs,
    },
}

#[derive(Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: Option<ConfigCommand>,
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Show current configuration
    Show {
        #[command(flatten)]
        output: OutputModeArgs,
    },
    /// Set configuration value (current shell only)
    Set { key: String, value: String },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
}

#[derive(Args)]
pub struct CompletionArgs {
    /// Shell to generate completion script for
    #[arg(value_enum, value_name = "shell")]
    pub shell: CompletionShell,
}
