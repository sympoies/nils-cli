use crate::{Limits, Operation, Provider, Request};
use clap::{Args, CommandFactory, Parser, Subcommand};
use nils_common::cli_contract::OutputFormat;
use std::net::SocketAddr;

#[derive(Debug, Parser)]
#[command(name="research-cli", version, long_version=nils_build_info::long_version(env!("CARGO_PKG_VERSION")),
    about="Search, read public URLs and prepare cited research with a reusable engine.",
    after_help="ENVIRONMENT:\n  OPENAI_API_KEY       OpenAI provider key\n  ANTHROPIC_API_KEY    Anthropic provider key\n\nEXIT CODES:\n  0 success; 1 runtime/upstream error; 64 syntax; 65 invalid request; 69 unavailable provider")]
pub struct Cli {
    /// Primary output format.
    #[arg(long, global = true, value_enum, default_value = "text")]
    pub format: OutputFormat,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Search with a selected provider.
    Search {
        /// Search query or research question.
        query: String,
        #[command(flatten)]
        options: Options,
    },
    /// Read a public URL using plain HTTP, without a model call.
    ReadUrl {
        /// Public HTTP(S) URL to read.
        #[arg(value_hint=clap::ValueHint::Url)]
        url: String,
        #[command(flatten)]
        limits: LimitArgs,
    },
    /// Research a question, optionally reading URLs and requesting topics.
    Brief {
        /// Search query or research question.
        query: String,
        /// Public evidence URL to read before synthesis; repeat as needed.
        #[arg(long = "url", value_hint=clap::ValueHint::Url)]
        urls: Vec<String>,
        /// Requested synthesis topic; repeat as needed.
        #[arg(long = "field")]
        fields: Vec<String>,
        #[command(flatten)]
        options: Options,
    },
    /// Serve the same engine over a local HTTP endpoint.
    Serve {
        /// HTTP listen address (loopback by default).
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: SocketAddr,
    },
    /// Export clap-generated shell completion.
    Completion {
        /// Shell to generate completion for.
        #[arg(value_enum)]
        shell: Shell,
    },
}
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
}
#[derive(Debug, Args)]
pub struct Options {
    /// Provider to use, without changing the caller schema.
    #[arg(long, value_enum, default_value = "openai")]
    pub provider: Provider,
    /// Requested answer language (best effort).
    #[arg(long)]
    pub language: Option<String>,
    /// Restrict returned source domains; repeat for multiple domains.
    #[arg(long = "domain")]
    pub domains: Vec<String>,
    /// Requested recency in days (best effort).
    #[arg(long)]
    pub recency_days: Option<u32>,
    /// Maximum number of returned sources.
    #[arg(long, default_value_t = 5)]
    pub limit: usize,
    #[command(flatten)]
    pub limits: LimitArgs,
}
#[derive(Debug, Args)]
pub struct LimitArgs {
    /// Total research deadline in seconds.
    #[arg(long, default_value_t = 30)]
    pub timeout_seconds: u64,
    /// Maximum bytes read per URL or provider response.
    #[arg(long, default_value_t = 262_144)]
    pub max_bytes: usize,
    /// Maximum aggregate prose characters returned.
    #[arg(long, default_value_t = 16_384)]
    pub max_chars: usize,
    /// Maximum redirects per public URL.
    #[arg(long, default_value_t = 5)]
    pub max_redirects: usize,
    /// Requested model output token limit; aggregate usage may exceed it.
    #[arg(long, default_value_t = 1024)]
    pub max_output_tokens: u32,
    /// Maximum uses of each configured server tool.
    #[arg(long, default_value_t = 1)]
    pub max_tool_uses: u32,
}
impl From<LimitArgs> for Limits {
    fn from(args: LimitArgs) -> Self {
        Self {
            timeout_seconds: args.timeout_seconds,
            max_bytes: args.max_bytes,
            max_chars: args.max_chars,
            max_redirects: args.max_redirects,
            max_output_tokens: args.max_output_tokens,
            max_tool_uses: args.max_tool_uses,
        }
    }
}
impl Command {
    pub fn into_request(self) -> Option<Request> {
        match self {
            Self::Search { query, options } => Some(options.request(Operation::Search, query)),
            Self::ReadUrl { url, limits } => {
                let mut request = Request::new(Operation::ReadUrl, "", Provider::Openai);
                request.urls = vec![url];
                request.limits = limits.into();
                Some(request)
            }
            Self::Brief {
                query,
                urls,
                fields,
                options,
            } => {
                let mut request = options.request(Operation::Brief, query);
                request.urls = urls;
                request.fields = fields;
                Some(request)
            }
            _ => None,
        }
    }
}
impl Options {
    fn request(self, operation: Operation, query: String) -> Request {
        let mut request = Request::new(operation, query, self.provider);
        request.language = self.language;
        request.domains = self.domains;
        request.recency_days = self.recency_days;
        request.limit = self.limit;
        request.limits = self.limits.into();
        request
    }
}
pub fn completion(shell: Shell) -> String {
    let mut bytes = Vec::new();
    let generator = match shell {
        Shell::Bash => clap_complete::Shell::Bash,
        Shell::Zsh => clap_complete::Shell::Zsh,
    };
    clap_complete::generate(generator, &mut Cli::command(), "research-cli", &mut bytes);
    String::from_utf8(bytes)
        .expect("UTF-8 completion")
        .replace("__subcmd__", "__")
}
