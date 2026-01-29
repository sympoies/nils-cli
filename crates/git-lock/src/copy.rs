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

    let src_label_arg = args.first().map(String::as_str);
    let dst_label = args.get(1).map(String::as_str).unwrap_or("");

    let src_label = match lock_fs::resolve_label(&repo_id, src_label_arg)? {
        Some(label) => label,
        None => {
            println!("❗ Usage: git-lock-copy <source-label> <target-label>");
            return Ok(1);
        }
    };

    if dst_label.trim().is_empty() {
        println!("❗ Target label is missing");
        return Ok(1);
    }

    let src_file = lock_dir.join(format!("{repo_id}-{src_label}.lock"));
    let dst_file = lock_dir.join(format!("{repo_id}-{dst_label}.lock"));

    if !src_file.exists() {
        println!("❌ Source git-lock [{repo_id}:{src_label}] not found");
        return Ok(1);
    }

    if dst_file.exists() {
        let prompt = format!(
            "⚠️  Target git-lock [{repo_id}:{dst_label}] already exists. Overwrite? [y/N] "
        );
        if !prompt::confirm(&prompt)? {
            return Ok(1);
        }
    }

    fs::copy(&src_file, &dst_file)?;
    fs::write(
        lock_dir.join(format!("{repo_id}-latest")),
        format!("{dst_label}\n"),
    )?;

    let content = fs::read_to_string(&src_file)?;
    let mut lines = content.lines();
    let line1 = lines.next().unwrap_or("");
    let (hash, note) = lock_fs::parse_lock_line(line1);
    let timestamp = content
        .lines()
        .find_map(|line| line.strip_prefix(lock_fs::TIMESTAMP_PREFIX))
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let subject = git::log_subject(&hash)?.unwrap_or_default();

    println!("📋 Copied git-lock [{repo_id}:{src_label}] → [{repo_id}:{dst_label}]");
    println!("   🏷️  tag:     {src_label} → {dst_label}");
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

    Ok(0)
}
