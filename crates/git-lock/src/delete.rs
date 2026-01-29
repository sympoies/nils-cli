use anyhow::Result;
use std::fs;

use crate::fs as lock_fs;
use crate::git;
use crate::prompt;

pub fn run(args: &[String]) -> Result<i32> {
    let repo_id = lock_fs::repo_id()?;
    let lock_dir = lock_fs::lock_dir_path();

    if !lock_dir.is_dir() {
        println!("❌ No git-locks found");
        return Ok(1);
    }

    let label_arg = args.first().map(String::as_str);
    let label = match lock_fs::resolve_label(&repo_id, label_arg)? {
        Some(label) => label,
        None => {
            println!("❌ No label provided and no latest git-lock exists");
            return Ok(1);
        }
    };

    let lock_file = lock_dir.join(format!("{repo_id}-{label}.lock"));
    if !lock_file.exists() {
        println!("❌ git-lock [{label}] not found");
        return Ok(1);
    }

    let content = fs::read_to_string(&lock_file)?;
    let mut lines = content.lines();
    let line1 = lines.next().unwrap_or("");
    let (hash, note) = lock_fs::parse_lock_line(line1);
    let timestamp = content
        .lines()
        .find_map(|line| line.strip_prefix(lock_fs::TIMESTAMP_PREFIX))
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let subject = git::log_subject(&hash)?.unwrap_or_default();

    println!("🗑️  Candidate for deletion:");
    println!("   🏷️  tag:     {label}");
    println!("   🧬 commit:  {hash}");
    if !subject.is_empty() {
        println!("   📄 message: {subject}");
    }
    if !note.is_empty() {
        println!("   📝 note:    {note}");
    }
    if !timestamp.is_empty() {
        println!("   📅 time:    {timestamp}");
    }
    println!();

    let prompt = "⚠️  Delete this git-lock? [y/N] ";
    if !prompt::confirm(prompt)? {
        return Ok(1);
    }

    fs::remove_file(&lock_file)?;
    println!("🗑️  Deleted git-lock [{repo_id}:{label}]");

    let latest_file = lock_dir.join(format!("{repo_id}-latest"));
    if latest_file.exists() {
        let latest_label = fs::read_to_string(&latest_file).unwrap_or_default();
        if latest_label.trim() == label {
            fs::remove_file(&latest_file)?;
            println!("🧼 Removed latest marker (was [{label}])");
        }
    }

    Ok(0)
}
