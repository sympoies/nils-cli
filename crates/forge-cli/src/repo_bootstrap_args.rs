use crate::cli::{RepoBootstrapOwnerKind, RepoBootstrapVisibility};
use clap::{ArgAction, Args};
use std::path::PathBuf;

#[derive(Args, Debug, Clone)]
pub struct RepoBootstrapArgs {
    /// Whether the owner is the authenticated user or an organization/group.
    #[arg(long = "owner-kind", value_enum)]
    pub owner_kind: RepoBootstrapOwnerKind,
    /// GitHub/GitLab repository visibility (Forgejo bootstrap remains private).
    #[arg(long, value_enum, default_value = "private")]
    pub visibility: RepoBootstrapVisibility,
    /// Adopt an existing empty GitHub/GitLab repository instead of creating one.
    #[arg(long = "existing-empty", action = ArgAction::SetTrue)]
    pub existing_empty: bool,
    /// Exact branch name to create and establish as the remote default.
    #[arg(long = "default-branch", value_name = "BRANCH")]
    pub default_branch: String,
    /// Regular file to copy into the repository root. Repeat for multiple files.
    #[arg(long = "file", value_name = "PATH", required = true)]
    pub files: Vec<PathBuf>,
    /// Semantic Commit message for the signed zero-parent root commit.
    #[arg(long, value_name = "TEXT")]
    pub message: String,
    /// UTF-8 file describing the user's explicit bootstrap authorization.
    #[arg(long = "reason-file", value_name = "PATH")]
    pub reason_file: PathBuf,
    /// Abandon an incomplete receipt and preserve its local state.
    #[arg(long, conflicts_with = "resume")]
    pub abandon: bool,
    /// Resume an existing durable bootstrap receipt after exact read-back.
    #[arg(long, action = ArgAction::SetTrue)]
    pub resume: bool,
}
