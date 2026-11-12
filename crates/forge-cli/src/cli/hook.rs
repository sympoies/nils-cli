//! `repo hook` and `org hook` arguments (GitHub webhooks only).
use std::path::PathBuf;

use clap::{ArgGroup, Args, Subcommand, ValueEnum};

/// Webhook id (see `hook list`).
fn hook_id() -> clap::builder::RangedU64ValueParser<u64> {
    clap::value_parser!(u64).range(1..)
}

/// `hook set-url` arguments.
#[derive(Args, Debug, Clone)]
pub struct HookSetUrlArgs {
    /// Webhook id (see `hook list`).
    #[arg(long, value_name = "ID", value_parser = hook_id())]
    pub hook: u64,
    /// URL the hook must currently have; any other current URL is refused.
    #[arg(long, value_name = "URL")]
    pub from: String,
    /// URL to switch the hook to.
    #[arg(long, value_name = "URL")]
    pub to: String,
}

/// `hook set-active` arguments: the state to move to; the opposite state is
/// the one the hook must currently be in.
#[derive(Args, Debug, Clone)]
#[command(group(ArgGroup::new("state").required(true).args(["active", "inactive"])))]
pub struct HookSetActiveArgs {
    /// Webhook id (see `hook list`).
    #[arg(long, value_name = "ID", value_parser = hook_id())]
    pub hook: u64,
    /// Activate the hook (it must currently be inactive).
    #[arg(long)]
    pub active: bool,
    /// Deactivate the hook (it must currently be active).
    #[arg(long)]
    pub inactive: bool,
}

/// `hook batch` arguments.
#[derive(Args, Debug, Clone)]
pub struct HookBatchArgs {
    /// JSON array of `{"target","hook","action","from","to"}` rows. `action` is
    /// `set-url` (string `from`/`to`) or `set-active` (boolean `from`/`to`);
    /// `target` is `OWNER/NAME` for `repo hook` and the organization for `org hook`.
    #[arg(long, value_name = "FILE")]
    pub file: PathBuf,
}

/// `repo hook` arguments.
#[derive(Args, Debug, Clone)]
pub struct RepoHookArgs {
    #[command(subcommand)]
    pub command: RepoHookCommand,
}

/// `repo hook` subcommands.
#[derive(Subcommand, Debug, Clone)]
pub enum RepoHookCommand {
    /// List the repository's webhooks. The secret is never printed.
    List,
    /// Change only a webhook's delivery URL (compare-and-swap on the current URL).
    SetUrl(HookSetUrlArgs),
    /// Change only a webhook's active flag (compare-and-swap on the current flag).
    SetActive(HookSetActiveArgs),
    /// Apply many `set-url` / `set-active` rows from one file.
    Batch(HookBatchArgs),
}

/// `org` arguments.
#[derive(Args, Debug)]
pub struct OrgArgs {
    /// Organization login the command acts on.
    #[arg(long, global = true, value_name = "ORG")]
    pub org: Option<String>,
    #[command(subcommand)]
    pub command: OrgCommand,
}

/// `org` subtree.
#[derive(Subcommand, Debug)]
pub enum OrgCommand {
    /// Organization webhooks (needs an organization owner token with `admin:org_hook`).
    Hook(OrgHookArgs),
}

/// `org hook` arguments.
#[derive(Args, Debug, Clone)]
pub struct OrgHookArgs {
    #[command(subcommand)]
    pub command: OrgHookCommand,
}

/// `org hook` subcommands.
#[derive(Subcommand, Debug, Clone)]
pub enum OrgHookCommand {
    /// List the organization's webhooks. The secret is never printed.
    List,
    /// Create a webhook; the secret is read only from `--secret-env`.
    Create(OrgHookCreateArgs),
    /// Change only a webhook's delivery URL (compare-and-swap on the current URL).
    SetUrl(HookSetUrlArgs),
    /// Change only a webhook's active flag (compare-and-swap on the current flag).
    SetActive(HookSetActiveArgs),
    /// Apply many `set-url` / `set-active` rows from one file.
    Batch(HookBatchArgs),
}

/// Payload content type of a new webhook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HookContentType {
    Json,
    Form,
}

impl HookContentType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Form => "form",
        }
    }
}

/// `org hook create` arguments.
#[derive(Args, Debug, Clone)]
#[command(group(ArgGroup::new("state").args(["active", "inactive"])))]
pub struct OrgHookCreateArgs {
    /// URL the payloads are delivered to.
    #[arg(long, value_name = "URL")]
    pub url: String,
    /// Comma-separated events to subscribe to (`*` for all).
    #[arg(long, value_delimiter = ',', required = true, value_name = "EVENTS")]
    pub events: Vec<String>,
    /// Payload content type.
    #[arg(long, value_enum)]
    pub content_type: HookContentType,
    /// Environment variable holding the webhook secret. The secret is never
    /// accepted on the command line and never printed.
    #[arg(long, value_name = "VAR")]
    pub secret_env: String,
    /// Create the hook active (default).
    #[arg(long)]
    pub active: bool,
    /// Create the hook inactive.
    #[arg(long)]
    pub inactive: bool,
}
