//! Finite, noninteractive replay owner. Never abort or retry a started replay.
mod git;
mod options;
mod state;

use nils_common::forge_identity;
pub(crate) use options::clap_command;
use std::time::{Duration, Instant};

pub(crate) fn run(args: &[String]) -> i32 {
    let options = match options::parse(args) {
        Ok(options) => options,
        Err(code) => return code,
    };
    match replay(&options) {
        Ok(()) => {
            println!("Replay completed with authenticated signing policy");
            0
        }
        Err(error) => {
            eprintln!("error: {error}");
            eprintln!("{}", recovery(&error));
            1
        }
    }
}

fn recovery(error: &str) -> &'static str {
    match error {
        "rebase_managed_worktree_required" => {
            "Use a git-cli managed linked worktree; select it with --repo <path>."
        }
        "rebase_clean_worktree_required" => {
            "Preserve or commit pending changes before starting replay."
        }
        "identity_replay_cached_default_required" => {
            "Cache the selected remote's default branch with git remote set-head <remote> --auto, inspect the branch, then retry."
        }
        "identity_replay_default_branch_forbidden" => {
            "Replay is restricted to a non-default managed branch; select the intended feature worktree."
        }
        "identity_replay_signing_state_unsupported" | "identity_replay_state_unsupported" => {
            "Inspect and preserve the current resolution and replay state. This owner accepts linear pick replay with keyless signing only; use explicit Git abort recovery and restart with semantic-commit rebase <upstream> if the saved program is unsupported."
        }
        _ => {
            "Inspect the current HEAD, index and rebase state. Resolve conflicts and stage the resolution, then run semantic-commit rebase --continue. For identity/signing failures, run forge-cli identity doctor --operation commit and repair the selected identity before continuing. No automatic abort or retry was performed."
        }
    }
}

fn replay(options: &options::Options) -> Result<(), String> {
    let state = state::preflight(options)?;
    let mut command = git::command(&state.root);
    if let Some(upstream) = &state.upstream {
        command.args([
            "rebase",
            "--merge",
            "--no-autostash",
            "--no-update-refs",
            "--no-rebase-merges",
            "--no-autosquash",
            "--no-fork-point",
            "--gpg-sign",
            upstream,
        ]);
    } else {
        command.args(["rebase", "--continue"]);
    }
    let output = forge_identity::execute_replay_commit(
        &mut command,
        &state.root,
        &state.branch,
        Instant::now() + Duration::from_secs(120),
    )
    .map_err(|error| error.code.to_owned())?;
    if !output.status.success() {
        return Err("rebase_stopped_state_retained".into());
    }
    Ok(())
}
