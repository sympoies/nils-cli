use nils_common::{git as shared_git, process};
use std::path::Path;

pub fn command_exists(program: &str) -> bool {
    process::cmd_exists(program)
}

pub fn is_inside_work_tree(repo: Option<&Path>) -> bool {
    match repo {
        Some(repo) => shared_git::is_inside_work_tree_in(repo),
        None => shared_git::is_inside_work_tree(),
    }
    .unwrap_or(false)
}

pub fn has_staged_changes(repo: Option<&Path>) -> anyhow::Result<bool> {
    Ok(match repo {
        Some(repo) => shared_git::has_staged_changes_in(repo)?,
        None => shared_git::has_staged_changes()?,
    })
}
