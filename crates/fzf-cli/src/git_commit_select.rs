use crate::{fzf, util};
use anyhow::{Context, Result};
use nils_common::shell::{self as common_shell, AnsiStripMode};

pub struct CommitPick {
    pub query: String,
    pub hash: String,
}

fn restore_bind(default_query: &str) -> String {
    if default_query.is_empty() {
        "focus:unbind(focus)+clear-query".to_string()
    } else {
        format!("focus:unbind(focus)+change-query[{default_query}]")
    }
}

pub fn pick_commit(default_query: &str, selected: Option<&str>) -> Result<Option<CommitPick>> {
    let log_out = util::run_capture(
        "git",
        &[
            "log",
            "--color=always",
            "--no-decorate",
            "--date=format:%m-%d %H:%M",
            "--pretty=format:%C(bold #82aaff)%h%C(reset) %C(#ecc48d)%cd%C(reset) %C(#7fdbca)%an%C(reset)%C(auto)%d%C(reset) %C(#d6deeb)%s%C(reset)",
        ],
    )?;

    let preview = r#"git-scope commit {1} | sed "s/^📅.*/&\n/""#;

    let mut args_vec: Vec<String> = vec![
        "--ansi".to_string(),
        "--reverse".to_string(),
        "--prompt".to_string(),
        "🌀 Commit > ".to_string(),
        "--preview-window".to_string(),
        "right:50%:wrap".to_string(),
        "--preview".to_string(),
        preview.to_string(),
        "--print-query".to_string(),
    ];

    if let Some(selected) = selected.filter(|s| !s.is_empty()) {
        args_vec.push("--track".to_string());
        args_vec.push("--bind".to_string());
        args_vec.push(restore_bind(default_query));
        args_vec.push("--query".to_string());
        args_vec.push(selected.to_string());
    } else {
        args_vec.push("--query".to_string());
        args_vec.push(default_query.to_string());
    }

    let args_ref: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();
    let (code, query, selected) = fzf::run_print_query(&format!("{log_out}\n"), &args_ref, &[])
        .context("fzf commit picker")?;

    if code != 0 {
        return Ok(None);
    }

    let query = query.unwrap_or_default();
    let Some(selected) = selected.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };

    let stripped = common_shell::strip_ansi(&selected, AnsiStripMode::CsiAnyTerminator);
    let hash = stripped
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();

    if hash.is_empty() {
        return Ok(None);
    }

    Ok(Some(CommitPick { query, hash }))
}

#[cfg(test)]
mod tests {
    use super::restore_bind;

    #[test]
    fn restore_bind_uses_clear_query_when_empty() {
        assert_eq!(
            restore_bind(""),
            "focus:unbind(focus)+clear-query".to_string()
        );
    }

    #[test]
    fn restore_bind_uses_single_bracket_action_argument() {
        assert_eq!(
            restore_bind("'gray"),
            "focus:unbind(focus)+change-query['gray]".to_string()
        );
    }
}
