use crate::git_cmd::run_git;
use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

pub fn is_git_repo() -> bool {
    Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

pub fn collect_staged() -> Result<Vec<String>> {
    let output = run_git(&[
        "diff",
        "--cached",
        "--name-status",
        "--diff-filter=ACMRTUXBD",
    ])?;
    Ok(lines(output))
}

pub fn collect_unstaged() -> Result<Vec<String>> {
    let output = run_git(&["diff", "--name-status", "--diff-filter=ACMRTUXBD"])?;
    Ok(lines(output))
}

pub fn collect_all() -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
    let staged = collect_staged()?;
    let unstaged = collect_unstaged()?;

    let mut combined = BTreeSet::new();
    for line in staged.iter().chain(unstaged.iter()) {
        if !line.trim().is_empty() {
            combined.insert(line.to_string());
        }
    }

    Ok((combined.into_iter().collect(), staged, unstaged))
}

pub fn collect_tracked(prefixes: &[String]) -> Result<Vec<String>> {
    let output = run_git(&["ls-files"])?;
    let files = lines(output);

    let mut filtered: Vec<String> = Vec::new();
    if prefixes.is_empty() {
        filtered = files;
    } else {
        for prefix in prefixes {
            let mut clean = prefix.trim_end_matches('/').to_string();
            if let Some(stripped) = clean.strip_prefix("./") {
                clean = stripped.to_string();
            }
            if clean.is_empty() || clean == "." {
                filtered.extend(files.iter().cloned());
                continue;
            }

            let path = Path::new(&clean);
            if path.is_dir() {
                let prefix_dir = format!("{}/", clean);
                filtered.extend(files.iter().filter(|f| f.starts_with(&prefix_dir)).cloned());
            } else if path.is_file() {
                filtered.extend(files.iter().filter(|f| *f == &clean).cloned());
            } else {
                filtered.extend(files.iter().filter(|f| f.starts_with(&clean)).cloned());
            }
        }
    }

    filtered.sort();
    filtered.dedup();

    let lines = filtered
        .into_iter()
        .filter(|f| !f.is_empty())
        .map(|f| format!("-\t{f}"))
        .collect();

    Ok(lines)
}

pub fn collect_untracked() -> Result<Vec<String>> {
    let output = run_git(&["ls-files", "--others", "--exclude-standard"])?;

    let lines = lines(output)
        .into_iter()
        .map(|f| format!("U\t{f}"))
        .collect();

    Ok(lines)
}

fn lines(output: String) -> Vec<String> {
    output
        .lines()
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}
