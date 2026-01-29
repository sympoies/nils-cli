use crate::print::print_file_content;
use crate::render::{color_reset_for_commit, kind_color_for_commit, render_tree_for_commit};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::process::{Command, Stdio};

pub fn render_commit(
    commit: &str,
    parent_selector: Option<&str>,
    no_color: bool,
    print: bool,
) -> Result<()> {
    print_commit_metadata(commit, no_color)?;
    print_commit_message(commit)?;

    let files = render_commit_files(commit, parent_selector, no_color)?;

    if print && !files.is_empty() {
        println!();
        println!("📦 Printing file contents:");
        for file in files {
            print_file_content(&file)?;
            println!();
        }
    }

    Ok(())
}

fn print_commit_metadata(commit: &str, no_color: bool) -> Result<()> {
    println!();
    if no_color {
        run_git(&[
            "log",
            "-1",
            "--color=never",
            "--date=format:%Y-%m-%d %H:%M:%S %z",
            "--pretty=format:🔖 %h %s%n👤 %an <%ae>%n📅 %ad",
            commit,
        ])?;
    } else {
        run_git(&[
            "log",
            "-1",
            "--date=format:%Y-%m-%d %H:%M:%S %z",
            "--pretty=format:🔖 %C(bold #82aaff)%h%C(reset) %C(#d6deeb)%s%C(reset)%n👤 %C(#7fdbca)%an%C(reset) <%C(#d6deeb)%ae%C(reset)>%n📅 %C(#ecc48d)%ad%C(reset)",
            commit,
        ])?;
    }
    Ok(())
}

fn print_commit_message(commit: &str) -> Result<()> {
    println!("\n📝 Commit Message:");
    let output = run_git(&["log", "-1", "--pretty=format:%B", commit])?;
    for (idx, line) in output.lines().enumerate() {
        if idx == 0 {
            println!("   {line}");
        } else if line.trim().is_empty() {
            println!();
        } else {
            println!("   {line}");
        }
    }
    Ok(())
}

fn render_commit_files(
    commit: &str,
    parent_selector: Option<&str>,
    no_color: bool,
) -> Result<Vec<String>> {
    let parents = commit_parents(commit)?;
    let parent_count = parents.len();
    let is_merge = parent_count > 1;

    let mut preface_lines: Vec<String> = Vec::new();
    let mut selected_index = 1usize;

    if is_merge {
        if let Some(selector) = parent_selector {
            match selector.parse::<usize>() {
                Ok(value) => selected_index = value,
                Err(_) => {
                    preface_lines.push(format!(
                        "  ⚠️  Invalid --parent value '{selector}' — falling back to parent #1"
                    ));
                    selected_index = 1;
                }
            }
        }

        if selected_index < 1 || selected_index > parent_count {
            preface_lines.push(format!(
                "  ⚠️  Parent index {} out of range (1-{}) — falling back to parent #1",
                selected_index, parent_count
            ));
            selected_index = 1;
        }
    }

    let ns_lines: String;
    let numstat_lines: String;
    let mut selected_parent_short = String::new();

    if is_merge {
        let selected_parent_hash = &parents[selected_index - 1];
        selected_parent_short = run_git(&["rev-parse", "--short", selected_parent_hash])?
            .trim()
            .to_string();

        ns_lines = run_git(&[
            "-c",
            "core.quotepath=false",
            "diff",
            "--name-status",
            selected_parent_hash,
            commit,
        ])?;
        numstat_lines = run_git(&[
            "-c",
            "core.quotepath=false",
            "diff",
            "--numstat",
            selected_parent_hash,
            commit,
        ])?;

        if ns_lines.trim().is_empty() {
            println!("\n📄 Changed files:");
            println!(
                "  ℹ️  Merge commit vs parent #{} ({}) has no file-level changes",
                selected_index,
                if selected_parent_short.is_empty() {
                    selected_parent_hash
                } else {
                    &selected_parent_short
                }
            );
            return Ok(Vec::new());
        }
    } else {
        ns_lines = run_git(&[
            "-c",
            "core.quotepath=false",
            "show",
            "--pretty=format:",
            "--name-status",
            commit,
        ])?;
        numstat_lines = run_git(&[
            "-c",
            "core.quotepath=false",
            "show",
            "--pretty=format:",
            "--numstat",
            commit,
        ])?;

        if ns_lines.trim().is_empty() || numstat_lines.trim().is_empty() {
            println!("\n📄 Changed files:");
            println!("  ℹ️  No file-level changes recorded for this commit");
            return Ok(Vec::new());
        }
    }

    let mut numstat_by_path: HashMap<String, (String, String)> = HashMap::new();
    for line in numstat_lines.lines() {
        let mut parts = line.split('\t');
        let add = match parts.next() {
            Some(v) => v.to_string(),
            None => continue,
        };
        let del = match parts.next() {
            Some(v) => v.to_string(),
            None => continue,
        };
        let raw_path = match parts.next() {
            Some(v) => v.to_string(),
            None => continue,
        };

        let canonical = canonical_path(&raw_path);
        numstat_by_path.insert(canonical, (add, del));
    }

    println!("\n📄 Changed files:");
    for line in preface_lines {
        println!("{line}");
    }

    if is_merge {
        println!(
            "  ℹ️  Merge commit with {} parents — showing diff against parent #{} ({})",
            parent_count,
            selected_index,
            if selected_parent_short.is_empty() {
                parents[selected_index - 1].as_str()
            } else {
                &selected_parent_short
            }
        );
    }

    let mut files: Vec<String> = Vec::new();
    let mut total_add = 0i64;
    let mut total_del = 0i64;
    let reset = color_reset_for_commit(no_color);

    for line in ns_lines.lines() {
        let mut parts = line.split('\t');
        let kind = match parts.next() {
            Some(v) => v.to_string(),
            None => continue,
        };
        let src = match parts.next() {
            Some(v) => v.to_string(),
            None => continue,
        };
        let dest = parts.next().map(|v| v.to_string());

        let display_path = if is_rename_or_copy(&kind) {
            match dest.as_ref() {
                Some(d) => format!("{src} -> {d}"),
                None => src.clone(),
            }
        } else {
            src.clone()
        };

        let file_path = if is_rename_or_copy(&kind) {
            dest.clone().unwrap_or_else(|| src.clone())
        } else {
            src.clone()
        };

        files.push(file_path.clone());

        let (mut add, mut del) = ("-".to_string(), "-".to_string());
        if let Some((a, d)) = numstat_by_path.get(&file_path) {
            add = a.clone();
            del = d.clone();
            if let Ok(val) = a.parse::<i64>() {
                total_add += val;
            }
            if let Ok(val) = d.parse::<i64>() {
                total_del += val;
            }
        }

        let color = kind_color_for_commit(&kind, no_color);
        println!(
            "  {color}➤ [{}] {display_path}  [+{add} / -{del}]{reset}",
            kind
        );
    }

    println!("\n  📊 Total: +{total_add} / -{total_del}");
    render_tree_for_commit(&files, no_color)?;

    Ok(files)
}

fn commit_parents(commit: &str) -> Result<Vec<String>> {
    let output = run_git(&["show", "-s", "--pretty=%P", commit])?;
    let parents = output
        .split_whitespace()
        .map(|v| v.to_string())
        .collect::<Vec<_>>();
    Ok(parents)
}

fn is_rename_or_copy(kind: &str) -> bool {
    kind.starts_with('R') || kind.starts_with('C')
}

fn canonical_path(raw: &str) -> String {
    if raw.contains("=>") {
        if raw.contains('{') && raw.contains('}') {
            let (prefix, after_open) = raw.split_once('{').unwrap_or((raw, ""));
            let (inside, suffix) = after_open.split_once('}').unwrap_or((after_open, ""));

            let mut new_part = inside.split("=>").last().unwrap_or(inside).trim();
            if new_part.starts_with(' ') {
                new_part = new_part.trim_start();
            }

            format!("{prefix}{new_part}{suffix}")
        } else {
            let mut new_part = raw.split("=>").last().unwrap_or(raw).trim();
            if new_part.starts_with(' ') {
                new_part = new_part.trim_start();
            }
            new_part.to_string()
        }
    } else {
        raw.to_string()
    }
}

fn run_git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("git {args:?}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {args:?} failed: {stderr}");
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
