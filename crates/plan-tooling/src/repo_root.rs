use nils_common::git as common_git;
use std::path::PathBuf;

pub fn detect() -> PathBuf {
    common_git::repo_root_or_cwd()
}
