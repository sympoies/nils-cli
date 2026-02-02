use anyhow::Result;
use std::process::Command;

use crate::git;
use crate::messages;
use crate::prompt;
use crate::store::LockStore;

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
        println!("{}", messages::TAG_USAGE);
        return Ok(1);
    }

    let store = LockStore::open()?;
    let label = match store.resolve_label(Some(positional[0].as_str()))? {
        Some(label) => label,
        None => {
            println!("❌ git-lock label not provided or not found");
            return Ok(1);
        }
    };

    let tag_name = positional[1].trim();
    if tag_name.is_empty() {
        println!("{}", messages::TAG_USAGE);
        return Ok(1);
    }

    let lock_dir = store.lock_dir();
    let lock_file = store.lock_path(&label);

    if !lock_file.exists() {
        println!(
            "❌ git-lock [{label}] not found in [{}] for [{repo_id}]",
            lock_dir.display(),
            repo_id = store.repo_id()
        );
        return Ok(1);
    }

    let lock = store.read_lock_at_path(&lock_file)?;
    let hash = lock.hash;

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
