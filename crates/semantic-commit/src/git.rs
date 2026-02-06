use nils_common::git as common_git;

pub fn is_inside_work_tree() -> bool {
    common_git::is_inside_work_tree().unwrap_or(false)
}

pub fn has_staged_changes() -> anyhow::Result<bool> {
    let status = common_git::run_status_quiet(&["diff", "--cached", "--quiet", "--"])?;

    match status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Ok(!status.success()),
    }
}
