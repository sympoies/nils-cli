//! Dedicated linear replay execution; generic Git preparation still refuses rebase.
use super::{Error, Result, git, probe};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    time::Instant,
};

const START: &[&str] = &[
    "rebase",
    "--merge",
    "--no-autostash",
    "--no-update-refs",
    "--no-rebase-merges",
    "--no-autosquash",
    "--no-fork-point",
    "--gpg-sign",
];

fn saved(path: &Path) -> Result<String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| Error::new("identity_replay_state_unavailable"))?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(Error::new("identity_replay_state_unsupported"));
    }
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .map_err(|_| Error::new("identity_replay_state_unavailable"))
}

fn binding(command: &Command, cwd: &Path, branch: &str) -> Result<bool> {
    let args: Vec<_> = command.get_args().map(|arg| arg.to_str()).collect();
    let continuing = args == [Some("rebase"), Some("--continue")];
    let initial = args.len() == START.len() + 1
        && args[..START.len()]
            .iter()
            .copied()
            .eq(START.iter().copied().map(Some))
        && args.last().copied().flatten().is_some_and(|oid| {
            matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
    if command.get_program() != "git"
        || (!continuing && !initial)
        || command
            .get_current_dir()
            .and_then(|path| fs::canonicalize(path).ok())
            != fs::canonicalize(cwd).ok()
    {
        return Err(Error::new("identity_replay_command_unsupported"));
    }
    let reference = format!("refs/heads/{branch}");
    git::raw(Some(cwd), &["check-ref-format", &reference])?;
    if continuing {
        let mut symbolic = Command::new("git");
        symbolic
            .current_dir(cwd)
            .args(["symbolic-ref", "--quiet", "HEAD"]);
        if probe::run(&mut symbolic)?.status.code() != Some(1) {
            return Err(Error::new("identity_replay_branch_mismatch"));
        }
        let state = PathBuf::from(git::raw(
            Some(cwd),
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "rebase-merge",
            ],
        )?);
        if saved(&state.join("head-name"))? != reference
            || saved(&state.join("orig-head"))?
                != git::raw(Some(cwd), &["rev-parse", "--verify", &reference])?
        {
            return Err(Error::new("identity_replay_branch_mismatch"));
        }
        // A saved explicit key overrides child user.signingkey. Git rejects
        // --gpg-sign together with --continue, so admit only keyless signing.
        if saved(&state.join("gpg_sign_opt"))? != "-S" {
            return Err(Error::new("identity_replay_signing_state_unsupported"));
        }
        if state.join("autostash").exists()
            || state.join("update-refs").exists()
            || state.join("reschedule-failed-exec").exists()
        {
            return Err(Error::new("identity_replay_state_unsupported"));
        }
        for name in ["git-rebase-todo", "done"] {
            if saved(&state.join(name))?
                .lines()
                .map(str::trim)
                .any(|line| {
                    !line.is_empty() && !line.starts_with('#') && !line.starts_with("pick ")
                })
            {
                return Err(Error::new("identity_replay_state_unsupported"));
            }
        }
    } else if git::raw(Some(cwd), &["symbolic-ref", "--quiet", "HEAD"])? != reference {
        return Err(Error::new("identity_replay_branch_mismatch"));
    }
    let remote = git::authoring_remote_for_branch(Some(cwd), Some(branch))?;
    let prefix = format!("refs/remotes/{remote}/");
    let default = git::raw(
        Some(cwd),
        &["symbolic-ref", "--quiet", &format!("{prefix}HEAD")],
    )
    .map_err(|_| Error::new("identity_replay_cached_default_required"))?;
    let default = default
        .strip_prefix(&prefix)
        .ok_or(Error::new("identity_replay_cached_default_required"))?;
    if branch == default {
        return Err(Error::new("identity_replay_default_branch_forbidden"));
    }
    Ok(continuing)
}

/// Execute only the finite replay command admitted by its CLI owner. Recheck the
/// original branch binding after authentication, before starting Git. Preserve
/// Git's original authors; the selected profile supplies committer and signer.
pub fn execute_replay_commit(
    command: &mut Command,
    cwd: &Path,
    original_branch: &str,
    deadline: Instant,
) -> Result<Output> {
    let result: Result<Output> = probe::with_deadline(Some(deadline), || {
        // Only the CLI owner's noninteractive presentation environment is
        // admitted before identity preparation adds its protected configuration.
        for (key, value) in command.get_envs() {
            let expected = match key.to_str() {
                Some("GIT_PAGER" | "PAGER") => "cat",
                Some("GIT_EDITOR" | "GIT_SEQUENCE_EDITOR") => "true",
                Some("GIT_TERMINAL_PROMPT") => "0",
                _ => return Err(Error::new("identity_git_override")),
            };
            if value.and_then(|value| value.to_str()) != Some(expected) {
                return Err(Error::new("identity_git_override"));
            }
        }
        binding(command, cwd, original_branch)?;
        let auth = git::prepare_replay_commit(command, cwd, original_branch)?;
        command
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_AUTHOR_EMAIL");
        let result = binding(command, cwd, original_branch).and_then(|_| probe::run(command));
        let success = result.as_ref().is_ok_and(|output| output.status.success());
        let head = success
            .then(|| git::raw(Some(cwd), &["rev-parse", "HEAD"]))
            .transpose();
        auth.finish(success, head.as_ref().ok().and_then(|head| head.as_deref()))?;
        let mut output = result?;
        auth.redact_output(&mut output);
        head?;
        Ok(output)
    });
    if let Err(error) = &result {
        super::audit_refusal(error.code)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn replay_binding_refuses_another_branch_before_authorization() {
        let repo = nils_test_support::git::init_repo_main_with_initial_commit();
        let mut command = Command::new("git");
        command
            .current_dir(repo.path())
            .args(START)
            .arg("0".repeat(40));
        assert_eq!(
            binding(&command, repo.path(), "fix/other")
                .unwrap_err()
                .code,
            "identity_replay_branch_mismatch"
        );
    }

    #[test]
    fn replay_binding_refuses_forged_continuation_branch() {
        let repo = nils_test_support::git::init_repo_main_with_initial_commit();
        nils_test_support::git::git(repo.path(), &["checkout", "--detach"]);
        let state = repo.path().join(".git/rebase-merge");
        fs::create_dir(&state).unwrap();
        fs::write(state.join("head-name"), "refs/heads/fix/original\n").unwrap();
        let mut command = Command::new("git");
        command
            .current_dir(repo.path())
            .args(["rebase", "--continue"]);
        assert_eq!(
            binding(&command, repo.path(), "fix/other")
                .unwrap_err()
                .code,
            "identity_replay_branch_mismatch"
        );
    }
}
