use crate::{confirm, fzf, util};

pub fn run(args: &[String]) -> i32 {
    if !is_git_repo() {
        eprintln!("❌ Not inside a Git repository. Aborting.");
        return 1;
    }

    let query = util::join_args(args);
    let tags = match util::run_capture("git", &["tag", "--sort=-creatordate"]) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };
    let list = tags
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    let input = format!("{}\n", list.join("\n"));

    let preview = r#"tag=$(printf "%s\n" {} | sed "s/^[* ]*//"); [[ -z "$tag" ]] && exit 0; hash=$(git rev-parse --verify --quiet "${tag}^{commit}"); [[ -z "$hash" ]] && printf "❌ Could not resolve tag to commit.\n" && exit 0; git log -n 100 --graph --color=always --decorate --abbrev-commit --date=iso-local --pretty=format:"%h %ad %an%d %s" "$hash""#;

    let args_vec: Vec<String> = vec![
        "--ansi".to_string(),
        "--reverse".to_string(),
        "--prompt".to_string(),
        "🏷️  Tag > ".to_string(),
        "--query".to_string(),
        query,
        "--preview-window=right:60%:wrap".to_string(),
        "--preview".to_string(),
        preview.to_string(),
    ];
    let args_ref: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();

    let (code, lines) = match fzf::run_lines(&input, &args_ref, &[]) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };
    if code != 0 {
        return 1;
    }

    let Some(selected) = lines.first() else {
        return 1;
    };
    let tag = selected.trim_start_matches(['*', ' ']).to_string();
    if tag.is_empty() {
        return 1;
    }

    let hash = resolve_tag_to_commit(&tag);
    let Some(hash) = hash else {
        eprintln!("❌ Could not resolve tag '{tag}' to a commit hash.");
        return 1;
    };

    match confirm::confirm(&format!("🚚 Checkout to tag '{tag}'? [y/N] ")) {
        Ok(true) => {}
        Ok(false) => return 1,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    }

    if util::run_output("git", &["checkout", &hash])
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        println!("✅ Checked out to tag {tag} (commit {hash})");
        0
    } else {
        println!("⚠️  Checkout to '{tag}' failed. Likely due to local changes or conflicts.");
        1
    }
}

fn resolve_tag_to_commit(tag: &str) -> Option<String> {
    let arg = format!("{tag}^{{commit}}");
    let out = util::run_output("git", &["rev-parse", "--verify", "--quiet", &arg]).ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let hash = stdout.lines().next().unwrap_or("").trim().to_string();
    if hash.is_empty() {
        None
    } else {
        Some(hash)
    }
}

fn is_git_repo() -> bool {
    util::run_output("git", &["rev-parse", "--is-inside-work-tree"])
        .map(|o| o.status.success())
        .unwrap_or(false)
}
