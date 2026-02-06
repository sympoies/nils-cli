use crate::{confirm, fzf, util};
use nils_common::git as common_git;

pub fn run(args: &[String]) -> i32 {
    if !common_git::is_inside_work_tree().unwrap_or(false) {
        eprintln!("❌ Not inside a Git repository. Aborting.");
        return 1;
    }

    let query = util::join_args(args);
    let branches = match util::run_capture("git", &["branch", "--sort=-committerdate"]) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    let list = branches
        .lines()
        .map(|l| {
            l.trim_start()
                .trim_start_matches('*')
                .trim_start()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

    let input = format!("{}\n", list.join("\n"));

    let preview = r#"branch=$(printf "%s\n" {} | sed "s/^[* ]*//"); [[ -z "$branch" ]] && exit 0; git log -n 100 --graph --color=always --decorate --abbrev-commit --date=iso-local --pretty=format:"%h %ad %an%d %s" "$branch""#;

    let args_vec: Vec<String> = vec![
        "--ansi".to_string(),
        "--reverse".to_string(),
        "--prompt".to_string(),
        "🌿 Branch > ".to_string(),
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
    let branch = selected.trim_start_matches(['*', ' ']).to_string();
    if branch.is_empty() {
        return 1;
    }

    match confirm::confirm(&format!("🚚 Checkout to branch '{branch}'? [y/N] ")) {
        Ok(true) => {}
        Ok(false) => return 1,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    }

    if common_git::run_status_quiet(&["checkout", &branch])
        .map(|status| status.success())
        .unwrap_or(false)
    {
        println!("✅ Checked out to {branch}");
        0
    } else {
        println!("⚠️  Checkout to '{branch}' failed. Likely due to local changes or conflicts.");
        1
    }
}
