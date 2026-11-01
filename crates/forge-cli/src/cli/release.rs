//! GitHub release arguments.
use clap::{Args, Subcommand};
use std::path::PathBuf;

#[derive(Args, Debug)]
pub struct ReleaseArgs {
    #[command(subcommand)]
    pub command: ReleaseCommand,
}
#[derive(Subcommand, Debug)]
pub enum ReleaseCommand {
    /// Create a release with explicit title and notes.
    Create(ReleaseCreateArgs),
    /// Upload files to an existing release.
    Upload(ReleaseUploadArgs),
    /// Read a release, its resolved target commit and assets.
    View(ReleaseViewArgs),
    /// Replace notes and optionally the title, then verify readback.
    Edit(ReleaseEditArgs),
    /// Delete a release and optionally its exact tag.
    Delete(ReleaseDeleteArgs),
}
#[derive(Args, Debug)]
pub struct ReleaseCreateArgs {
    pub tag: String,
    #[arg(long)]
    pub title: String,
    /// Markdown notes file; use - to read stdin.
    #[arg(long)]
    pub notes_file: String,
    #[arg(long)]
    pub target: Option<String>,
    #[arg(long)]
    pub verify_tag: bool,
    #[arg(long)]
    pub draft: bool,
    #[arg(long)]
    pub prerelease: bool,
    /// Explicitly mark (or do not mark) this release as latest.
    #[arg(long)]
    pub latest: Option<bool>,
    /// Optional release asset files.
    pub assets: Vec<PathBuf>,
}
#[derive(Args, Debug)]
pub struct ReleaseUploadArgs {
    pub tag: String,
    #[arg(required = true)]
    pub assets: Vec<PathBuf>,
    /// Replace existing release assets with the same name.
    #[arg(long)]
    pub clobber: bool,
}

#[derive(Args, Debug)]
pub struct ReleaseViewArgs {
    pub tag: String,
}

#[derive(Args, Debug)]
pub struct ReleaseEditArgs {
    pub tag: String,
    /// Markdown notes file; use - to read stdin.
    #[arg(long)]
    pub notes_file: String,
    #[arg(long)]
    pub title: Option<String>,
}

#[derive(Args, Debug)]
pub struct ReleaseDeleteArgs {
    pub tag: String,
    /// Also delete the exact tag after verifying release removal.
    #[arg(long)]
    pub cleanup_tag: bool,
    /// Permit deleting a canonical SemVer tag's release (optional leading v).
    #[arg(long)]
    pub allow_version_tag: bool,
}
