use anyhow::Result;
use nils_common::env as shared_env;
use std::process::Command;

use crate::messages;
use crate::store::LockStore;

pub fn run(args: &[String]) -> Result<i32> {
    let mut no_color = false;
    let mut positional: Vec<String> = Vec::new();

    for arg in args.iter().map(String::as_str) {
        match arg {
            "--no-color" | "no-color" => {
                no_color = true;
            }
            "--help" | "-h" => {
                println!("{}", messages::DIFF_USAGE);
                return Ok(0);
            }
            _ => positional.push(arg.to_string()),
        }
    }

    if positional.len() > 2 {
        println!("❗ Too many labels provided (expected 2)");
        println!("{}", messages::DIFF_USAGE);
        return Ok(1);
    }

    let store = LockStore::open()?;

    let label1 = match store.resolve_label(positional.first().map(String::as_str))? {
        Some(label) => label,
        None => {
            println!("{}", messages::DIFF_USAGE);
            return Ok(1);
        }
    };

    let label2 = match store.resolve_label(positional.get(1).map(String::as_str))? {
        Some(label) => label,
        None => {
            println!("❗ Second label not provided or found");
            return Ok(1);
        }
    };

    let file1 = store.lock_path(&label1);
    let file2 = store.lock_path(&label2);

    if !file1.exists() {
        println!("❌ git-lock [{label1}] not found for [{}]", store.repo_id());
        return Ok(1);
    }
    if !file2.exists() {
        println!("❌ git-lock [{label2}] not found for [{}]", store.repo_id());
        return Ok(1);
    }

    let lock1 = store.read_lock_at_path(&file1)?;
    let lock2 = store.read_lock_at_path(&file2)?;
    let hash1 = lock1.hash;
    let hash2 = lock2.hash;

    println!(
        "🧮 Comparing commits: [{}:{label1}] → [{label2}]",
        store.repo_id()
    );
    println!("   🔖 {label1}: {hash1}");
    println!("   🔖 {label2}: {hash2}");
    println!();

    let mut log_args = vec!["log", "--oneline", "--graph", "--decorate"];
    if shared_env::no_color_requested(no_color) {
        log_args.push("--color=never");
    }
    let range = format!("{hash1}..{hash2}");
    log_args.push(&range);

    let status = Command::new("git").args(&log_args).status()?;

    Ok(status.code().unwrap_or(1))
}
