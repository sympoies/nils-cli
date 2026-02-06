use nils_common::git as common_git;
use std::path::PathBuf;

pub fn detect() -> PathBuf {
    if let Ok(Some(root)) = common_git::repo_root() {
        return root;
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
