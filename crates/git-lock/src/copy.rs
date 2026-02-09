use anyhow::Result;
use std::fs;

use crate::git::DefaultGitBackend;
use crate::lock_view::LockDetails;
use crate::messages;
use crate::prompt;
use crate::store::LockStore;

pub fn run(args: &[String]) -> Result<i32> {
    let store = LockStore::open()?;
    let lock_dir = store.lock_dir().to_path_buf();

    if !lock_dir.is_dir() {
        println!("{}", messages::NO_GIT_LOCKS_FOUND);
        return Ok(1);
    }

    let src_label_arg = args.first().map(String::as_str);
    let dst_label = args.get(1).map(String::as_str).unwrap_or("");

    let src_label = match store.resolve_label(src_label_arg)? {
        Some(label) => label,
        None => {
            println!("{}", messages::COPY_USAGE);
            return Ok(1);
        }
    };

    if dst_label.trim().is_empty() {
        println!("{}", messages::TARGET_LABEL_MISSING);
        return Ok(1);
    }

    let src_file = store.lock_path(&src_label);
    let dst_file = store.lock_path(dst_label);

    if !src_file.exists() {
        println!(
            "❌ Source git-lock [{}:{src_label}] not found",
            store.repo_id()
        );
        return Ok(1);
    }

    if dst_file.exists() {
        let prompt = format!(
            "⚠️  Target git-lock [{}:{dst_label}] already exists. Overwrite? [y/N] ",
            store.repo_id()
        );
        if !prompt::confirm(&prompt)? {
            return Ok(1);
        }
    }

    fs::copy(&src_file, &dst_file)?;
    store.write_latest_label(dst_label)?;

    let git_backend = DefaultGitBackend;
    let details = LockDetails::load_from_path(&store, &src_label, &src_file, &git_backend)?;

    println!(
        "📋 Copied git-lock [{}:{src_label}] → [{}:{dst_label}]",
        store.repo_id(),
        store.repo_id()
    );
    println!("   🏷️  tag:     {src_label} → {dst_label}");
    println!("   🧬 commit:  {}", details.lock.hash);
    if let Some(subject) = details.subject.as_deref() {
        println!("   📄 message: {subject}");
    }
    if !details.lock.note.is_empty() {
        println!("   📝 note:    {}", details.lock.note);
    }
    if let Some(timestamp) = details.lock.timestamp.as_deref()
        && !timestamp.is_empty()
    {
        println!("   📅 time:    {timestamp}");
    }

    Ok(0)
}
