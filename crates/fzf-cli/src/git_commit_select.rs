use crate::{fzf, util};
use anyhow::{Context, Result};

pub struct CommitPick {
    pub query: String,
    pub hash: String,
}

pub fn pick_commit(default_query: &str) -> Result<Option<CommitPick>> {
    let log_out = util::run_capture(
        "git",
        &[
            "log",
            "--no-decorate",
            "--date=format:%m-%d %H:%M",
            "--pretty=format:%h %cd %an%d %s",
        ],
    )?;

    let preview = r#"git-scope commit {1} | sed "s/^📅.*/&\n/""#;

    let args_vec: Vec<String> = vec![
        "--ansi".to_string(),
        "--reverse".to_string(),
        "--prompt".to_string(),
        "🌀 Commit > ".to_string(),
        "--preview-window".to_string(),
        "right:50%:wrap".to_string(),
        "--preview".to_string(),
        preview.to_string(),
        "--print-query".to_string(),
        "--query".to_string(),
        default_query.to_string(),
    ];

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

    let stripped = util::strip_ansi(&selected);
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
