use clap::{Args, Subcommand, ValueEnum};

/// `search` subtree. Free-text / reverse-reference query over forge issues and
/// PRs, scoped by explicit query qualifiers or a repository. GitHub-only in v1; GitLab / Local hit a
/// structured `provider_unsupported` seam.
#[derive(Subcommand, Debug)]
pub enum SearchCommand {
    /// Full-text search over issues (`gh search issues`).
    Issues(SearchQueryArgs),
    /// Full-text search over pull requests (`gh search prs`).
    Prs(SearchQueryArgs),
    /// Read paginated closed issue or merged PR facts in a completion window.
    Completions(CompletionQueryArgs),
    /// List issues / PRs that reference a ref via cross-reference events.
    #[command(name = "refs-to")]
    RefsTo(SearchRefsToArgs),
}

#[derive(Args, Debug, Clone)]
pub struct CompletionQueryArgs {
    /// Structured provider query; repository qualifiers own scope.
    #[arg(value_name = "QUERY")]
    pub query: String,
    /// Read closed issues or merged pull requests.
    #[arg(long, value_enum, default_value_t = CompletionKind::Issue)]
    pub kind: CompletionKind,
    /// Inclusive completion-window start, as an RFC 3339 timestamp.
    #[arg(long, value_name = "TIMESTAMP")]
    pub since: String,
    /// Exclusive completion-window end, as an RFC 3339 timestamp.
    #[arg(long, value_name = "TIMESTAMP")]
    pub until: String,
    /// Size of top-level and nested provider pages (1-100).
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub page_size: u32,
    /// Maximum search pages read in one call (1-10).
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=10))]
    pub max_pages: u32,
    /// Maximum pages read from each label or closing-reference connection (1-100).
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u32).range(1..=100))]
    pub max_connection_pages: u32,
    /// Opaque continuation returned by a prior completion read.
    #[arg(long)]
    pub cursor: Option<String>,
    /// Exclude results whose canonical repository is a fork.
    #[arg(long)]
    pub exclude_forks: bool,
    /// Exclude results whose canonical repository is archived.
    #[arg(long)]
    pub exclude_archived: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize)]
#[clap(rename_all = "lower")]
#[serde(rename_all = "lowercase")]
pub enum CompletionKind {
    Issue,
    Pr,
}

/// Arguments for `search refs-to <ref>`.
#[derive(Args, Debug, Clone)]
pub struct SearchRefsToArgs {
    /// Target issue / PR to find references to. Accepts a GitHub URL,
    /// `owner/name#number`, or `#number` / `number` (repo from context).
    #[arg(value_name = "REF")]
    pub reference: String,
    /// Cap the number of cross-reference events scanned (default: 30).
    #[arg(long, default_value_t = 30)]
    pub limit: u32,
}

/// Shared arguments for `search issues` / `search prs`.
#[derive(Args, Debug, Clone)]
pub struct SearchQueryArgs {
    /// Free-text query passed to the provider's search primitive.
    #[arg(value_name = "QUERY")]
    pub query: String,
    /// Restrict matching to these fields (comma-separated or repeatable).
    /// Defaults to title, body, and comments.
    #[arg(
        long = "match",
        value_enum,
        value_delimiter = ',',
        default_values_t = [SearchMatchField::Title, SearchMatchField::Body, SearchMatchField::Comments],
        value_name = "FIELD"
    )]
    pub match_fields: Vec<SearchMatchField>,
    /// Cap the number of returned results (default: 30).
    #[arg(long, default_value_t = 30)]
    pub limit: u32,
}

/// Field a `search` query may match on. Mirrors `gh search --match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lower")]
pub enum SearchMatchField {
    Title,
    Body,
    Comments,
}

impl SearchMatchField {
    /// Stable lower-case rendering used in the `gh search --match` value and
    /// in the envelope's `match_fields`.
    pub fn as_str(self) -> &'static str {
        match self {
            SearchMatchField::Title => "title",
            SearchMatchField::Body => "body",
            SearchMatchField::Comments => "comments",
        }
    }
}
