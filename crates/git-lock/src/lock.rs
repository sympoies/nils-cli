use anyhow::Result;

use crate::fs;
use crate::git;

pub fn run(args: &[String]) -> Result<i32> {
    let label = args.first().map(String::as_str).unwrap_or("");
    let note = args.get(1).map(String::as_str).unwrap_or("");
    let commit = args.get(2).map(String::as_str).unwrap_or("HEAD");

    let label = if label.is_empty() { "default" } else { label };

    let repo_id = fs::repo_id()?;
    fs::ensure_lock_dir()?;
    let lock_file = fs::lock_file(&repo_id, label);
    let latest_file = fs::latest_file(&repo_id);

    let hash = match git::rev_parse(commit)? {
        Some(value) if !value.is_empty() => value,
        _ => {
            println!("❌ Invalid commit: {commit}");
            return Ok(1);
        }
    };

    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let content = format!("{hash} # {note}\n{}{}\n", fs::TIMESTAMP_PREFIX, timestamp);
    std::fs::write(&lock_file, content)?;
    std::fs::write(&latest_file, format!("{label}\n"))?;

    print!("🔐 [{repo_id}:{label}] Locked: {hash}");
    if !note.is_empty() {
        print!("  # {note}");
    }
    println!();
    println!("    at {timestamp}");

    Ok(0)
}
