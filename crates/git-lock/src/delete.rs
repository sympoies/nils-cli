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

    let label_arg = args.first().map(String::as_str);
    let label = match store.resolve_label(label_arg)? {
        Some(label) => label,
        None => {
            println!("❌ No label provided and no latest git-lock exists");
            return Ok(1);
        }
    };

    let lock_file = store.lock_path(&label);
    if !lock_file.exists() {
        println!("❌ git-lock [{label}] not found");
        return Ok(1);
    }

    let git_backend = DefaultGitBackend;
    let details = LockDetails::load_from_path(&store, &label, &lock_file, &git_backend)?;

    println!("🗑️  Candidate for deletion:");
    println!("   🏷️  tag:     {label}");
    println!("   🧬 commit:  {}", details.lock.hash);
    if let Some(subject) = details.subject.as_deref() {
        println!("   📄 message: {subject}");
    }
    if !details.lock.note.is_empty() {
        println!("   📝 note:    {}", details.lock.note);
    }
    if let Some(timestamp) = details.lock.timestamp.as_deref() {
        if !timestamp.is_empty() {
            println!("   📅 time:    {timestamp}");
        }
    }
    println!();

    let prompt = "⚠️  Delete this git-lock? [y/N] ";
    if !prompt::confirm(prompt)? {
        return Ok(1);
    }

    fs::remove_file(&lock_file)?;
    println!("🗑️  Deleted git-lock [{}:{label}]", store.repo_id());

    if store.remove_latest_if_matches(&label)? {
        println!("🧼 Removed latest marker (was [{label}])");
    }

    Ok(0)
}
