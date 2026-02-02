use anyhow::Result;

use crate::git;
use crate::store::LockStore;

pub fn run(args: &[String]) -> Result<i32> {
    let label = args.first().map(String::as_str).unwrap_or("");
    let note = args.get(1).map(String::as_str).unwrap_or("");
    let commit = args.get(2).map(String::as_str).unwrap_or("HEAD");

    let label = if label.is_empty() { "default" } else { label };

    let store = LockStore::open()?;
    store.ensure_dir()?;

    let hash = match git::rev_parse(commit)? {
        Some(value) if !value.is_empty() => value,
        _ => {
            println!("❌ Invalid commit: {commit}");
            return Ok(1);
        }
    };

    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let content = format!(
        "{hash} # {note}\n{}{}\n",
        crate::fs::TIMESTAMP_PREFIX,
        timestamp
    );
    store.write_lock_content(label, &content)?;
    store.write_latest_label(label)?;

    print!("🔐 [{}:{label}] Locked: {hash}", store.repo_id());
    if !note.is_empty() {
        print!("  # {note}");
    }
    println!();
    println!("    at {timestamp}");

    Ok(0)
}
