use anyhow::Result;

use crate::git;
use crate::git::DefaultGitBackend;
use crate::lock_view::LockDetails;
use crate::prompt;
use crate::store::LockStore;

pub fn run(args: &[String]) -> Result<i32> {
    let label_arg = args.first().map(String::as_str);

    let store = LockStore::open()?;
    store.ensure_dir()?;

    let label = match store.resolve_label(label_arg)? {
        Some(label) => label,
        None => {
            println!("❌ No recent git-lock found for {}", store.repo_id());
            return Ok(1);
        }
    };

    let lock_file = store.lock_path(&label);
    if !lock_file.exists() {
        println!(
            "❌ No git-lock named '{label}' found for {}",
            store.repo_id()
        );
        return Ok(1);
    }

    let git_backend = DefaultGitBackend;
    let details = LockDetails::load_from_path(&store, &label, &lock_file, &git_backend)?;

    println!(
        "🔐 Found [{}:{label}] → {}",
        store.repo_id(),
        details.lock.hash
    );
    if !details.lock.note.is_empty() {
        println!("    # {}", details.lock.note);
    }
    if let Some(subject) = details.subject.as_deref() {
        println!("    commit message: {subject}");
    }
    println!();

    let prompt = format!("⚠️  Hard reset to [{label}]? [y/N] ");
    if !prompt::confirm(&prompt)? {
        return Ok(1);
    }

    let status = git::run_status_inherit(&["reset", "--hard", &details.lock.hash])?;
    if status != 0 {
        return Ok(status);
    }

    println!(
        "⏪ [{}:{label}] Reset to: {}",
        store.repo_id(),
        details.lock.hash
    );

    Ok(0)
}
