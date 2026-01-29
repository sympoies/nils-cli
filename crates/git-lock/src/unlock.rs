use anyhow::Result;

use crate::fs;
use crate::git;
use crate::prompt;

pub fn run(args: &[String]) -> Result<i32> {
    let label_arg = args.first().map(String::as_str);

    let repo_id = fs::repo_id()?;
    fs::ensure_lock_dir()?;

    let label = match fs::resolve_label(&repo_id, label_arg)? {
        Some(label) => label,
        None => {
            println!("❌ No recent git-lock found for {repo_id}");
            return Ok(1);
        }
    };

    let lock_file = fs::lock_file(&repo_id, &label);
    if !lock_file.exists() {
        println!("❌ No git-lock named '{label}' found for {repo_id}");
        return Ok(1);
    }

    let lock = fs::read_lock_file(&lock_file)?;
    let msg = git::log_subject(&lock.hash)?.unwrap_or_default();

    println!("🔐 Found [{repo_id}:{label}] → {}", lock.hash);
    if !lock.note.is_empty() {
        println!("    # {}", lock.note);
    }
    if !msg.is_empty() {
        println!("    commit message: {msg}");
    }
    println!();

    let prompt = format!("⚠️  Hard reset to [{label}]? [y/N] ");
    if !prompt::confirm(&prompt)? {
        return Ok(1);
    }

    let status = git::run_status_inherit(&["reset", "--hard", &lock.hash])?;
    if status != 0 {
        return Ok(status);
    }

    println!("⏪ [{repo_id}:{label}] Reset to: {}", lock.hash);

    Ok(0)
}
