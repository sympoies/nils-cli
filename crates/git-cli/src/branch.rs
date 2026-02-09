use crate::commit_shared::{git_output, git_status_success, git_stdout_trimmed};
use crate::prompt;
use std::collections::HashSet;
use std::process::Output;

pub fn dispatch(cmd: &str, args: &[String]) -> Option<i32> {
    match cmd {
        "cleanup" | "delete-merged" => Some(run_cleanup(args)),
        _ => None,
    }
}

struct CleanupArgs {
    base_ref: String,
    squash_mode: bool,
    help: bool,
}

fn run_cleanup(args: &[String]) -> i32 {
    let parsed = match parse_args(args) {
        Ok(value) => value,
        Err(code) => return code,
    };

    if parsed.help {
        print_help();
        return 0;
    }

    if !git_status_success(&["rev-parse", "--is-inside-work-tree"]) {
        eprintln!("❌ Not in a git repository");
        return 1;
    }

    let base_ref = parsed.base_ref;
    let squash_mode = parsed.squash_mode;

    if !git_status_success(&["rev-parse", "--verify", "--quiet", &base_ref]) {
        eprintln!("❌ Invalid base ref: {base_ref}");
        return 1;
    }

    let base_commit = match git_stdout_trimmed(&["rev-parse", &format!("{base_ref}^{{commit}}")]) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("❌ Unable to resolve base commit: {base_ref}");
            return 1;
        }
    };

    let head_commit = match git_stdout_trimmed(&["rev-parse", "HEAD"]) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("❌ Unable to resolve HEAD commit");
            return 1;
        }
    };

    let delete_flag = if base_commit != head_commit {
        "-D"
    } else {
        "-d"
    };

    let current_branch = match git_stdout_trimmed(&["rev-parse", "--abbrev-ref", "HEAD"]) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("❌ Unable to resolve current branch");
            return 1;
        }
    };

    let mut protected: HashSet<String> = ["main", "master", "develop", "trunk"]
        .iter()
        .map(|name| (*name).to_string())
        .collect();

    if current_branch != "HEAD" {
        protected.insert(current_branch.clone());
    }
    protected.insert(base_ref.clone());

    if let Some(base_local) = resolve_base_local(&base_ref) {
        protected.insert(base_local);
    }

    let merged_branches = match git_output(&[
        "for-each-ref",
        "--merged",
        &base_ref,
        "--format=%(refname:short)",
        "refs/heads",
    ]) {
        Ok(output) => parse_lines(&output),
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    let mut merged_set: HashSet<String> = HashSet::new();
    for branch in &merged_branches {
        merged_set.insert(branch.clone());
    }

    if !squash_mode && merged_branches.is_empty() {
        println!("✅ No merged local branches found.");
        return 0;
    }

    let mut candidates: Vec<String> = Vec::new();

    if squash_mode {
        let local_branches =
            match git_output(&["for-each-ref", "--format=%(refname:short)", "refs/heads"]) {
                Ok(output) => parse_lines(&output),
                Err(err) => {
                    eprintln!("{err:#}");
                    return 1;
                }
            };

        if local_branches.is_empty() {
            println!("✅ No local branches found.");
            return 0;
        }

        for branch in local_branches {
            if protected.contains(&branch) {
                continue;
            }

            if merged_set.contains(&branch) {
                candidates.push(branch);
                continue;
            }

            let cherry_output = match git_output(&["cherry", "-v", &base_ref, &branch]) {
                Ok(output) => output,
                Err(_) => {
                    eprintln!("❌ Failed to compare {branch} against {base_ref}");
                    return 1;
                }
            };

            let cherry_text = String::from_utf8_lossy(&cherry_output.stdout);
            let has_plus = cherry_text.lines().any(|line| line.starts_with('+'));
            if has_plus {
                continue;
            }

            candidates.push(branch);
        }
    } else {
        for branch in merged_branches {
            if protected.contains(&branch) {
                continue;
            }
            candidates.push(branch);
        }
    }

    if candidates.is_empty() {
        if squash_mode {
            println!("✅ No deletable branches found.");
        } else {
            println!("✅ No deletable merged branches.");
        }
        return 0;
    }

    if squash_mode {
        println!("🧹 Branches to delete (base: {base_ref}, mode: squash):");
    } else {
        println!("🧹 Merged branches to delete (base: {base_ref}):");
    }
    for branch in &candidates {
        println!("  - {branch}");
    }

    if prompt::confirm_or_abort("❓ Proceed with deleting these branches? [y/N] ").is_err() {
        return 1;
    }

    for branch in &candidates {
        let mut branch_delete_flag = delete_flag;
        if delete_flag == "-d" && squash_mode && !merged_set.contains(branch) {
            branch_delete_flag = "-D";
        }
        let _ = git_status_success(&["branch", branch_delete_flag, "--", branch]);
    }

    println!("✅ Deleted merged branches.");
    0
}

fn parse_args(args: &[String]) -> Result<CleanupArgs, i32> {
    let mut base_ref = "HEAD".to_string();
    let mut squash_mode = false;
    let mut help = false;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                help = true;
            }
            "-s" | "--squash" => {
                squash_mode = true;
            }
            "-b" | "--base" => {
                let Some(value) = args.get(i + 1) else {
                    return Err(2);
                };
                base_ref = value.to_string();
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    Ok(CleanupArgs {
        base_ref,
        squash_mode,
        help,
    })
}

fn print_help() {
    println!("Usage: git-delete-merged-branches [-b|--base <ref>] [-s|--squash]");
    println!("  -b, --base <ref>  Base ref used to determine merged branches (default: HEAD)");
    println!("  -s, --squash      Include branches already applied to base (git cherry)");
}

fn parse_lines(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.to_string())
        .collect()
}

fn resolve_base_local(base_ref: &str) -> Option<String> {
    let remote_ref = format!("refs/remotes/{base_ref}");
    if git_status_success(&["show-ref", "--verify", "--quiet", &remote_ref]) {
        return Some(
            base_ref
                .split_once('/')
                .map(|(_, tail)| tail.to_string())
                .unwrap_or_else(|| base_ref.to_string()),
        );
    }

    let local_ref = format!("refs/heads/{base_ref}");
    if git_status_success(&["show-ref", "--verify", "--quiet", &local_ref]) {
        return Some(base_ref.to_string());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{dispatch, parse_args, parse_lines, resolve_base_local};
    use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir};
    use pretty_assertions::assert_eq;
    use std::process::Command;

    #[test]
    fn dispatch_unknown_returns_none() {
        assert_eq!(dispatch("unknown", &[]), None);
    }

    #[test]
    fn cleanup_help_exits_success_without_git_runtime() {
        let args = vec!["--help".to_string()];
        assert_eq!(dispatch("cleanup", &args), Some(0));
        assert_eq!(dispatch("delete-merged", &args), Some(0));
    }

    #[test]
    fn parse_args_supports_base_and_squash_flags() {
        let args = vec![
            "--base".to_string(),
            "origin/main".to_string(),
            "--squash".to_string(),
            "--unknown".to_string(),
        ];
        let parsed = parse_args(&args).expect("parsed");
        assert_eq!(parsed.base_ref, "origin/main");
        assert!(parsed.squash_mode);
        assert!(!parsed.help);
    }

    #[test]
    fn parse_args_requires_value_for_base_flag() {
        let args = vec!["--base".to_string()];
        let err_code = match parse_args(&args) {
            Ok(_) => panic!("expected usage error"),
            Err(code) => code,
        };
        assert_eq!(err_code, 2);
    }

    #[test]
    fn parse_lines_skips_blank_entries() {
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg("printf 'main\\n\\nfeature/a\\n'")
            .output()
            .expect("output");
        let lines = parse_lines(&output);
        assert_eq!(lines, vec!["main".to_string(), "feature/a".to_string()]);
    }

    #[test]
    fn resolve_base_local_prefers_remote_then_local_then_none() {
        let lock = GlobalStateLock::new();

        let remote_stubs = StubBinDir::new();
        remote_stubs.write_exe(
            "git",
            r#"#!/bin/bash
set -euo pipefail
if [[ "${1:-}" == "show-ref" && "${2:-}" == "--verify" && "${3:-}" == "--quiet" ]]; then
  if [[ "${4:-}" == "refs/remotes/origin/main" ]]; then
    exit 0
  fi
  exit 1
fi
exit 1
"#,
        );
        let remote_guard = EnvGuard::set(&lock, "PATH", &remote_stubs.path_str());
        assert_eq!(resolve_base_local("origin/main"), Some("main".to_string()));
        drop(remote_guard);

        let local_stubs = StubBinDir::new();
        local_stubs.write_exe(
            "git",
            r#"#!/bin/bash
set -euo pipefail
if [[ "${1:-}" == "show-ref" && "${2:-}" == "--verify" && "${3:-}" == "--quiet" ]]; then
  if [[ "${4:-}" == "refs/heads/main" ]]; then
    exit 0
  fi
  exit 1
fi
exit 1
"#,
        );
        let local_guard = EnvGuard::set(&lock, "PATH", &local_stubs.path_str());
        assert_eq!(resolve_base_local("main"), Some("main".to_string()));
        drop(local_guard);

        let none_stubs = StubBinDir::new();
        none_stubs.write_exe(
            "git",
            r#"#!/bin/bash
set -euo pipefail
exit 1
"#,
        );
        let _none_guard = EnvGuard::set(&lock, "PATH", &none_stubs.path_str());
        assert_eq!(resolve_base_local("feature/topic"), None);
    }
}
