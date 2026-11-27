use super::{CommitOperation, CommitOptions, run_git_output_with_pager};
use crate::default_branch::install::Installation;
use nils_common::forge_identity;
use std::io::Write;
use std::path::Path;
use std::process::ExitStatus;

pub(super) fn preflight_target(repo: Option<&Path>) -> bool {
    match select_target(repo) {
        Ok(()) => true,
        Err(err) => {
            eprintln!("{err}");
            false
        }
    }
}

fn select_target(repo: Option<&Path>) -> forge_identity::Result<()> {
    // Target selection is local. The real commit performs forge authorization.
    if forge_identity::load_ungated()?.is_some() {
        let remote = forge_identity::authoring_remote(repo)?;
        forge_identity::target_for_remote(repo, &remote, true)?;
    }
    Ok(())
}

pub(super) fn run(
    message_path: &Path,
    options: &CommitOptions,
    operation: CommitOperation,
    installation: Option<&mut Installation<'_>>,
) -> anyhow::Result<ExitStatus> {
    if let Some(installation) = installation {
        return installation.commit(message_path, options.signoff, &options.trailers);
    }
    let message_path = message_path.to_string_lossy();
    let mut args = vec!["commit"];
    if matches!(
        operation,
        CommitOperation::Amend | CommitOperation::MessageOnlyAmend
    ) {
        args.push("--amend");
    }
    if options.allow_empty {
        args.push("--allow-empty");
    }
    if options.signoff {
        args.push("--signoff");
    }
    for trailer in &options.trailers {
        args.extend(["--trailer", trailer]);
    }
    if options.no_edit {
        args.push("--no-edit");
    } else {
        args.extend(["-F", message_path.as_ref()]);
    }

    let output = run_git_output_with_pager(options.repo.as_deref(), &args)?;
    if !output.stderr.is_empty() {
        std::io::stderr().write_all(&output.stderr)?;
    }
    Ok(output.status)
}
