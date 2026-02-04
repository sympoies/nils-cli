use std::process::{Command, Output, Stdio};

pub fn dispatch(cmd: &str, args: &[String]) -> Option<i32> {
    match cmd {
        "pick" => Some(run_pick(args)),
        _ => None,
    }
}

struct PickArgs {
    target: String,
    commit_spec: String,
    name: String,
    remote_opt: Option<String>,
    want_force: bool,
    want_fetch: bool,
    want_stay: bool,
}

enum ParseResult {
    Help,
    Usage,
    Ok(PickArgs),
}

fn run_pick(args: &[String]) -> i32 {
    let parsed = match parse_pick_args(args) {
        ParseResult::Help => {
            print_pick_help();
            return 0;
        }
        ParseResult::Usage => {
            print_pick_usage_error();
            return 2;
        }
        ParseResult::Ok(value) => value,
    };

    if !git_success(&["rev-parse", "--git-dir"]) {
        eprintln!("❌ Not inside a Git repository.");
        return 1;
    }

    let op_warnings = detect_in_progress_ops();
    if !op_warnings.is_empty() {
        eprintln!("❌ Refusing to run during an in-progress Git operation:");
        for warning in op_warnings {
            eprintln!("   - {warning}");
        }
        return 1;
    }

    if !git_success_quiet(&["diff", "--quiet", "--no-ext-diff"]) {
        eprintln!("❌ Unstaged changes detected. Commit or stash before running git-pick.");
        return 1;
    }
    if !git_success_quiet(&["diff", "--cached", "--quiet", "--no-ext-diff"]) {
        eprintln!("❌ Staged changes detected. Commit or stash before running git-pick.");
        return 1;
    }

    let remotes = git_remotes();
    if remotes.is_empty() {
        eprintln!("❌ No git remotes found (need a remote to push CI branches).");
        return 1;
    }

    let mut remote = parsed.remote_opt.clone().unwrap_or_default();
    if remote.is_empty() {
        if remotes.iter().any(|name| name == "origin") {
            remote = "origin".to_string();
        } else {
            remote = remotes[0].clone();
        }
    }

    let mut target_branch = parsed.target.clone();
    let mut target_branch_for_name = parsed.target.clone();
    let mut target_is_remote = false;

    if let Some((maybe_remote, rest)) = parsed.target.split_once('/') {
        if remotes.iter().any(|name| name == maybe_remote) {
            let target_remote = maybe_remote.to_string();
            target_branch = rest.to_string();
            target_branch_for_name = target_branch.clone();

            target_is_remote = true;
            if parsed.remote_opt.is_none() {
                remote = target_remote;
            } else if remote != target_remote {
                eprintln!(
                    "❌ Target ref looks like '{}' (remote '{}') but --remote is '{}'.",
                    parsed.target, target_remote, remote
                );
                return 2;
            }
        }
    }

    if parsed.want_fetch {
        let status = git_status_quiet(&["fetch", "--prune", "--", &remote, &target_branch]);
        if status.unwrap_or(1) != 0 {
            eprintln!("⚠️  Fetch failed: git fetch --prune -- {remote} {target_branch}");
            eprintln!("   Continuing with local refs (or re-run with --no-fetch).");
        }
    }

    let base_ref = resolve_base_ref(&remote, &target_branch, &parsed.target, target_is_remote)
        .unwrap_or_else(|| {
            eprintln!("❌ Cannot resolve target ref: {}", parsed.target);
            String::new()
        });
    if base_ref.is_empty() {
        return 1;
    }

    let ci_branch = format!("ci/{target_branch_for_name}/{}", parsed.name);
    if !git_success_quiet(&["check-ref-format", "--branch", &ci_branch]) {
        eprintln!("❌ Invalid CI branch name: {ci_branch}");
        return 2;
    }

    let pick_commits = match resolve_pick_commits(&parsed.commit_spec) {
        Some(value) => value,
        None => return 1,
    };

    let orig_branch = git_stdout_trimmed_optional(&["symbolic-ref", "--quiet", "--short", "HEAD"]);
    let orig_sha = git_stdout_trimmed_optional(&["rev-parse", "--verify", "HEAD"]);

    let local_branch_exists = git_success_quiet(&[
        "show-ref",
        "--verify",
        "--quiet",
        &format!("refs/heads/{ci_branch}"),
    ]);

    if local_branch_exists && !parsed.want_force {
        eprintln!("❌ Local branch already exists: {ci_branch}");
        eprintln!("   Use --force to reset/rebuild it.");
        return 1;
    }

    if !parsed.want_force && !local_branch_exists && remote_branch_exists(&remote, &ci_branch) {
        eprintln!("❌ Remote branch already exists: {remote}/{ci_branch}");
        eprintln!("   Use --force to reset/rebuild it.");
        return 1;
    }

    println!("🌿 CI branch: {ci_branch}");
    println!("🔧 Base     : {base_ref}");
    println!(
        "🍒 Pick     : {} ({} commit(s))",
        parsed.commit_spec,
        pick_commits.len()
    );

    if local_branch_exists {
        if git_status_inherit(&["switch", "--quiet", "--", &ci_branch]).unwrap_or(1) != 0 {
            return 1;
        }
        if git_status_inherit(&["reset", "--hard", &base_ref]).unwrap_or(1) != 0 {
            return 1;
        }
    } else if git_status_inherit(&["switch", "--quiet", "-c", &ci_branch, &base_ref]).unwrap_or(1)
        != 0
    {
        return 1;
    }

    if git_status_inherit(&build_cherry_pick_args(&pick_commits)).unwrap_or(1) != 0 {
        eprintln!("❌ Cherry-pick failed on branch: {ci_branch}");
        eprintln!("🧠 Resolve conflicts then run: git cherry-pick --continue");
        eprintln!("    Or abort and retry:        git cherry-pick --abort");
        return 1;
    }

    let push_status = if parsed.want_force {
        git_status_inherit(&[
            "push",
            "-u",
            "--force-with-lease",
            "--",
            &remote,
            &ci_branch,
        ])
    } else {
        git_status_inherit(&["push", "-u", "--", &remote, &ci_branch])
    };
    if push_status.unwrap_or(1) != 0 {
        return 1;
    }

    println!("✅ Pushed: {remote}/{ci_branch} (CI should run on branch push)");
    println!("🧹 Cleanup:");
    println!("  git push --delete -- {remote} {ci_branch}");
    println!("  git branch -D -- {ci_branch}");

    if parsed.want_stay {
        return 0;
    }

    if let Some(branch) = orig_branch {
        let _ = git_status_inherit(&["switch", "--quiet", "--", &branch]);
    } else if let Some(sha) = orig_sha {
        let _ = git_status_inherit(&["switch", "--quiet", "--detach", &sha]);
    }

    0
}

fn parse_pick_args(args: &[String]) -> ParseResult {
    let mut remote_opt: Option<String> = None;
    let mut want_force = false;
    let mut want_fetch = true;
    let mut want_stay = false;
    let mut positional: Vec<String> = Vec::new();

    let mut idx = 0;
    while idx < args.len() {
        let arg = &args[idx];
        if arg == "--" {
            positional.extend(args.iter().skip(idx + 1).cloned());
            break;
        }

        match arg.as_str() {
            "-h" | "--help" => return ParseResult::Help,
            "-f" | "--force" => {
                want_force = true;
                idx += 1;
            }
            "--no-fetch" => {
                want_fetch = false;
                idx += 1;
            }
            "--stay" => {
                want_stay = true;
                idx += 1;
            }
            "-r" | "--remote" => {
                let Some(value) = args.get(idx + 1) else {
                    return ParseResult::Usage;
                };
                remote_opt = Some(value.to_string());
                idx += 2;
            }
            _ => {
                if let Some(value) = arg.strip_prefix("--remote=") {
                    remote_opt = Some(value.to_string());
                    idx += 1;
                } else if arg.starts_with('-') {
                    return ParseResult::Usage;
                } else {
                    positional.push(arg.to_string());
                    idx += 1;
                }
            }
        }
    }

    if positional.len() != 3 {
        return ParseResult::Usage;
    }

    ParseResult::Ok(PickArgs {
        target: positional[0].clone(),
        commit_spec: positional[1].clone(),
        name: positional[2].clone(),
        remote_opt,
        want_force,
        want_fetch,
        want_stay,
    })
}

fn print_pick_help() {
    println!("git-pick: create and push a CI branch with cherry-picked commits");
    println!();
    println!("Usage:");
    println!("  git-pick <target> <commit-or-range> <name>");
    println!();
    println!("Args:");
    println!("  <target>           Base branch/ref (e.g. main, release/x, origin/main)");
    println!("  <commit-or-range>  Passed to 'git cherry-pick' (e.g. abc123, A..B, A^..B)");
    println!("  <name>             Suffix for CI branch: ci/<target>/<name>");
    println!();
    println!("Options:");
    println!("  -r, --remote <name>  Remote to fetch/push (default: origin, else first remote)");
    println!("      --no-fetch       Skip 'git fetch' (uses existing local refs)");
    println!(
        "  -f, --force          Reset existing ci/<target>/<name> and force-push (with lease)"
    );
    println!("      --stay           Keep checked out on the CI branch");
}

fn print_pick_usage_error() {
    eprintln!("❌ Usage: git-pick <target> <commit-or-range> <name>");
    eprintln!("   Try: git-pick --help");
}

fn resolve_base_ref(
    remote: &str,
    target_branch: &str,
    target: &str,
    target_is_remote: bool,
) -> Option<String> {
    if target_is_remote
        && git_success_quiet(&[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{remote}/{target_branch}"),
        ])
    {
        return Some(format!("{remote}/{target_branch}"));
    }

    if git_success_quiet(&[
        "show-ref",
        "--verify",
        "--quiet",
        &format!("refs/heads/{target_branch}"),
    ]) {
        return Some(target_branch.to_string());
    }

    if git_success_quiet(&[
        "show-ref",
        "--verify",
        "--quiet",
        &format!("refs/remotes/{remote}/{target_branch}"),
    ]) {
        return Some(format!("{remote}/{target_branch}"));
    }

    let target_commit = format!("{target}^{{commit}}");
    if git_success_quiet(&["rev-parse", "--verify", "--quiet", &target_commit]) {
        return Some(target.to_string());
    }

    None
}

fn resolve_pick_commits(commit_spec: &str) -> Option<Vec<String>> {
    if commit_spec.contains("..") {
        let output = git_output(&["rev-list", "--reverse", commit_spec]);
        let mut commits: Vec<String> = Vec::new();
        if let Some(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                commits = stdout
                    .lines()
                    .map(|line| line.trim())
                    .filter(|line| !line.is_empty())
                    .map(|line| line.to_string())
                    .collect();
            }
        }
        if commits.is_empty() {
            eprintln!("❌ No commits resolved from range: {commit_spec}");
            return None;
        }
        return Some(commits);
    }

    let commit_ref = format!("{commit_spec}^{{commit}}");
    let commit_sha = git_stdout_trimmed_optional(&["rev-parse", "--verify", &commit_ref]);
    if let Some(commit_sha) = commit_sha {
        return Some(vec![commit_sha]);
    }

    eprintln!("❌ Cannot resolve commit: {commit_spec}");
    None
}

fn remote_branch_exists(remote: &str, branch: &str) -> bool {
    let output = git_output(&["ls-remote", "--heads", remote, branch]);
    let Some(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    !String::from_utf8_lossy(&output.stdout).trim().is_empty()
}

fn git_remotes() -> Vec<String> {
    let output = git_output(&["remote"]);
    let Some(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect()
}

fn detect_in_progress_ops() -> Vec<String> {
    let mut warnings = Vec::new();
    if git_path_exists("MERGE_HEAD", true) {
        warnings.push("merge in progress".to_string());
    }
    if git_path_exists("rebase-apply", false) || git_path_exists("rebase-merge", false) {
        warnings.push("rebase in progress".to_string());
    }
    if git_path_exists("CHERRY_PICK_HEAD", true) {
        warnings.push("cherry-pick in progress".to_string());
    }
    if git_path_exists("REVERT_HEAD", true) {
        warnings.push("revert in progress".to_string());
    }
    warnings
}

fn git_path_exists(name: &str, is_file: bool) -> bool {
    let output = git_stdout_trimmed_optional(&["rev-parse", "--git-path", name]);
    let Some(path) = output else {
        return false;
    };
    let path = std::path::Path::new(&path);
    if is_file {
        path.is_file()
    } else {
        path.is_dir()
    }
}

fn build_cherry_pick_args(commits: &[String]) -> Vec<&str> {
    let mut args: Vec<&str> = Vec::with_capacity(commits.len() + 2);
    args.push("cherry-pick");
    args.push("--");
    for commit in commits {
        args.push(commit);
    }
    args
}

fn git_command(args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdin(Stdio::null());
    cmd
}

fn git_output(args: &[&str]) -> Option<Output> {
    git_command(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()
}

fn git_status_inherit(args: &[&str]) -> Option<i32> {
    git_command(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .ok()
        .map(|status| status.code().unwrap_or(1))
}

fn git_status_quiet(args: &[&str]) -> Option<i32> {
    git_command(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .map(|status| status.code().unwrap_or(1))
}

fn git_success(args: &[&str]) -> bool {
    matches!(git_output(args), Some(output) if output.status.success())
}

fn git_success_quiet(args: &[&str]) -> bool {
    matches!(git_status_quiet(args), Some(code) if code == 0)
}

fn git_stdout_trimmed_optional(args: &[&str]) -> Option<String> {
    let output = git_output(args)?;
    if !output.status.success() {
        return None;
    }
    let value = trim_trailing_newlines(&String::from_utf8_lossy(&output.stdout));
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn trim_trailing_newlines(input: &str) -> String {
    input.trim_end_matches(['\n', '\r']).to_string()
}
