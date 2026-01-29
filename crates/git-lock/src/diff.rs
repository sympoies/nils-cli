use anyhow::Result;
use std::process::Command;

use crate::fs as lock_fs;

pub fn run(args: &[String]) -> Result<i32> {
    let mut no_color = false;
    let mut positional: Vec<String> = Vec::new();

    for arg in args.iter().map(String::as_str) {
        match arg {
            "--no-color" | "no-color" => {
                no_color = true;
            }
            "--help" | "-h" => {
                println!("❗ Usage: git-lock diff <label1> <label2> [--no-color]");
                return Ok(0);
            }
            _ => positional.push(arg.to_string()),
        }
    }

    if positional.len() > 2 {
        println!("❗ Too many labels provided (expected 2)");
        println!("❗ Usage: git-lock diff <label1> <label2> [--no-color]");
        return Ok(1);
    }

    let repo_id = lock_fs::repo_id()?;

    let label1 = match lock_fs::resolve_label(&repo_id, positional.first().map(String::as_str))? {
        Some(label) => label,
        None => {
            println!("❗ Usage: git-lock diff <label1> <label2> [--no-color]");
            return Ok(1);
        }
    };

    let label2 = match lock_fs::resolve_label(&repo_id, positional.get(1).map(String::as_str))? {
        Some(label) => label,
        None => {
            println!("❗ Second label not provided or found");
            return Ok(1);
        }
    };

    let lock_dir = lock_fs::lock_dir_path();
    let file1 = lock_dir.join(format!("{repo_id}-{label1}.lock"));
    let file2 = lock_dir.join(format!("{repo_id}-{label2}.lock"));

    if !file1.exists() {
        println!("❌ git-lock [{label1}] not found for [{repo_id}]");
        return Ok(1);
    }
    if !file2.exists() {
        println!("❌ git-lock [{label2}] not found for [{repo_id}]");
        return Ok(1);
    }

    let line1 = std::fs::read_to_string(&file1)?;
    let line2 = std::fs::read_to_string(&file2)?;
    let hash1 = lock_fs::parse_lock_line(line1.lines().next().unwrap_or("")).0;
    let hash2 = lock_fs::parse_lock_line(line2.lines().next().unwrap_or("")).0;

    println!("🧮 Comparing commits: [{repo_id}:{label1}] → [{label2}]");
    println!("   🔖 {label1}: {hash1}");
    println!("   🔖 {label2}: {hash2}");
    println!();

    let mut log_args = vec!["log", "--oneline", "--graph", "--decorate"];
    if no_color || std::env::var_os("NO_COLOR").is_some() {
        log_args.push("--color=never");
    }
    let range = format!("{hash1}..{hash2}");
    log_args.push(&range);

    let status = Command::new("git").args(&log_args).status()?;

    Ok(status.code().unwrap_or(1))
}
