use crate::{confirm, git_commit_select, util};
use nils_common::git as common_git;

pub fn run(args: &[String]) -> i32 {
    if !common_git::is_inside_work_tree().unwrap_or(false) {
        eprintln!("❌ Not inside a Git repository. Aborting.");
        return 1;
    }

    let query = util::join_args(args);
    let pick = match git_commit_select::pick_commit(&query, None) {
        Ok(Some(p)) => p,
        Ok(None) => return 1,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    let ref_hash = pick.hash;

    match confirm::confirm(&format!("🚚 Checkout to commit {ref_hash}? [y/N] ")) {
        Ok(true) => {}
        Ok(false) => return 1,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    }

    if common_git::run_status_quiet(&["checkout", &ref_hash])
        .map(|status| status.success())
        .unwrap_or(false)
    {
        return 0;
    }

    println!("⚠️  Checkout to '{ref_hash}' failed. Likely due to local changes.");
    match confirm::confirm("📦 Stash your current changes and retry checkout? [y/N] ") {
        Ok(true) => {}
        Ok(false) => return 1,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    }

    let timestamp = util::run_capture("date", &["+%F_%H%M"])
        .unwrap_or_default()
        .trim()
        .to_string();
    let subject = util::run_capture("git", &["log", "-1", "--pretty=%s", "HEAD"])
        .unwrap_or_default()
        .trim()
        .to_string();
    let stash_msg = format!("auto-stash {timestamp} HEAD - {subject}");

    if common_git::run_status_quiet(&["stash", "push", "-u", "-m", &stash_msg])
        .map(|status| status.success())
        .unwrap_or(false)
    {
        println!("📦 Changes stashed: {stash_msg}");
    } else {
        return 1;
    }

    if common_git::run_status_quiet(&["checkout", &ref_hash])
        .map(|status| status.success())
        .unwrap_or(false)
    {
        println!("✅ Checked out to {ref_hash}");
        0
    } else {
        1
    }
}
