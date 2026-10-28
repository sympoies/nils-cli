use super::{KeychainArg, OutputModeArgs};
use clap::{Args, ValueHint};
use std::path::PathBuf;

#[derive(Args)]
pub struct PublishArgs {
    /// Immutable access-only profile payload with generation and publication attempt
    #[arg(long, value_name = "file", value_hint = ValueHint::FilePath)]
    pub snapshot: PathBuf,
    /// Accounts directory in which to publish this profile
    #[arg(long, value_name = "dir", value_hint = ValueHint::DirPath)]
    pub into: PathBuf,
    /// macOS Keychain handling for the projected login
    #[arg(long, value_enum, default_value_t = KeychainArg::Auto)]
    pub keychain: KeychainArg,
    #[command(flatten)]
    pub output: OutputModeArgs,
}
