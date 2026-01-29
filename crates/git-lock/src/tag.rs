use anyhow::Result;
use std::process::Command;

use crate::fs as lock_fs;
use crate::git;
use crate::prompt;

pub fn run(args: &[String]) -> Result<i32> {
    let mut do_push = false;
    let mut tag_msg: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--push" => {
                do_push = true;
                i += 1;
            }
            "-m" => {
                i += 1;
                let msg = args.get(i).map(String::as_str).unwrap_or("");
                tag_msg = Some(msg.to_string());
                i += 1;
            }
            _ => {
                positional.push(args[i].to_string());
                i += 1;
            }
        }
    }

    if positional.len() != 2 {
        println!("❗ Usage: git-lock tag <git-lock-label> <tag-name> [-m <tag-message>] [--push]");
        return Ok(1);
    }

    let repo_id = lock_fs::repo_id()?;
    let label = match lock_fs::resolve_label(&repo_id, Some(positional[0].as_str()))? {
        Some(label) => label,
        None => {
            println!("❌ git-lock label not provided or not found");
            return Ok(1);
        }
    };

    let tag_name = positional[1].trim();
    if tag_name.is_empty() {
        println!("❗ Usage: git-lock tag <git-lock-label> <tag-name> [-m <tag-message>] [--push]");
        return Ok(1);
    }

    let lock_dir = lock_fs::lock_dir_path();
    let lock_file = lock_dir.join(format!("{repo_id}-{label}.lock"));

    if !lock_file.exists() {
        println!(
            "❌ git-lock [{label}] not found in [{}] for [{repo_id}]",
            lock_dir.display()
        );
        return Ok(1);
    }

    let content = std::fs::read_to_string(&lock_file)?;
    let line1 = content.lines().next().unwrap_or("");
    let (hash, _) = lock_fs::parse_lock_line(line1);

    let mut message = tag_msg.unwrap_or_default();
    if message.trim().is_empty() {
        message = git::show_subject(&hash)?.unwrap_or_default();
    }

    if git::tag_exists(tag_name)? {
        println!("⚠️  Git tag [{tag_name}] already exists.");
        if !prompt::confirm("❓ Overwrite it? [y/N] ")? {
            return Ok(1);
        }
        let status = git::run_status_inherit(&["tag", "-d", tag_name])?;
        if status != 0 {
            println!("❌ Failed to delete existing tag [{tag_name}]");
            return Ok(1);
        }
    }

    let status = Command::new("git")
        .args(["tag", "-a", tag_name, &hash, "-m", &message])
        .status()?;
    if !status.success() {
        return Ok(status.code().unwrap_or(1));
    }

    println!("🏷️  Created tag [{tag_name}] at commit [{hash}]");
    println!("📝 Message: {message}");

    if do_push {
        let status = Command::new("git")
            .args(["push", "origin", tag_name])
            .status()?;
        if !status.success() {
            return Ok(status.code().unwrap_or(1));
        }
        println!("🚀 Pushed tag [{tag_name}] to origin");

        let status = Command::new("git").args(["tag", "-d", tag_name]).status()?;
        if status.success() {
            println!("🧹 Deleted local tag [{tag_name}]");
        }
    }

    Ok(0)
}
