use anyhow::Result;
use std::fs;
use std::path::PathBuf;

use crate::fs as lock_fs;
use crate::git;
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};

pub fn run(_args: &[String]) -> Result<i32> {
    let repo_id = lock_fs::repo_id()?;
    let lock_dir = lock_fs::lock_dir_path();

    if !lock_dir.is_dir() {
        println!("📬 No git-locks found for [{repo_id}]");
        return Ok(0);
    }

    let latest = lock_fs::read_latest_label(&repo_id)?.unwrap_or_default();

    let mut entries = collect_entries(&repo_id, &lock_dir)?;
    if entries.is_empty() {
        println!("📬 No git-locks found for [{repo_id}]");
        return Ok(0);
    }

    entries.sort_by(|a, b| b.epoch.cmp(&a.epoch));

    println!("🔐 git-lock list for [{repo_id}]:");

    for entry in entries {
        println!();
        print!(" - 🏷️  tag:     {}", entry.label);
        if entry.label == latest {
            print!("  ⭐ (latest)");
        }
        println!();
        println!("   🧬 commit:  {}", entry.lock.hash);

        if let Some(subject) = git::log_subject(&entry.lock.hash)? {
            if !subject.is_empty() {
                println!("   📄 message: {subject}");
            }
        }

        if !entry.lock.note.is_empty() {
            println!("   📝 note:    {}", entry.lock.note);
        }

        if let Some(timestamp) = entry.lock.timestamp {
            if !timestamp.is_empty() {
                println!("   📅 time:    {timestamp}");
            }
        }
    }

    Ok(0)
}

struct Entry {
    epoch: i64,
    label: String,
    lock: lock_fs::LockFile,
}

fn collect_entries(repo_id: &str, lock_dir: &PathBuf) -> Result<Vec<Entry>> {
    let prefix = format!("{repo_id}-");

    let dir_entries = match fs::read_dir(lock_dir) {
        Ok(value) => value,
        Err(_) => return Ok(Vec::new()),
    };

    let mut candidates: Vec<(PathBuf, String)> = Vec::new();
    for entry in dir_entries {
        let entry = match entry {
            Ok(value) => value,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|v| v.to_str()) {
            Some(value) => value.to_string(),
            None => continue,
        };
        if !file_name.starts_with(&prefix) || !file_name.ends_with(".lock") {
            continue;
        }

        let label = file_name
            .trim_end_matches(".lock")
            .trim_start_matches(&prefix)
            .to_string();

        candidates.push((path, label));
    }

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let progress = Progress::new(
        candidates.len() as u64,
        ProgressOptions::default().with_finish(ProgressFinish::Clear),
    );
    progress.set_message("scanning");

    let mut entries = Vec::new();
    for (path, label) in candidates {
        let lock = match lock_fs::read_lock_file(&path) {
            Ok(v) => v,
            Err(err) => {
                progress.finish_and_clear();
                return Err(err);
            }
        };
        let epoch = lock
            .timestamp
            .as_deref()
            .map(lock_fs::timestamp_epoch)
            .unwrap_or(0);

        entries.push(Entry { epoch, label, lock });
        progress.inc(1);
    }

    progress.finish_and_clear();
    Ok(entries)
}
