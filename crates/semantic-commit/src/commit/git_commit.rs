use super::{CAT_PAGER_ENV, CommitOperation, CommitOptions, run_git_output_with_pager};
use crate::default_branch::install::Installation;
use nils_common::forge_identity;
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitStatus};

pub(super) fn preflight(
    message_path: &Path,
    options: &CommitOptions,
    operation: CommitOperation,
) -> forge_identity::Result<()> {
    let message_path = message_path.to_string_lossy();
    let args = arguments(&message_path, options, operation);
    let mut command = Command::new("git");
    command.args(&args).envs(CAT_PAGER_ENV);
    let repo = options.repo.as_deref();
    if let Some(repo) = repo {
        command.current_dir(repo);
    }
    forge_identity::prepare_git(&mut command, repo, &args)?;
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
    let args = arguments(&message_path, options, operation);
    let output = run_git_output_with_pager(options.repo.as_deref(), &args)?;
    if !output.stderr.is_empty() {
        std::io::stderr().write_all(&output.stderr)?;
    }
    Ok(output.status)
}

fn arguments<'a>(
    message_path: &'a str,
    options: &'a CommitOptions,
    operation: CommitOperation,
) -> Vec<&'a str> {
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
        args.extend(["-F", message_path]);
    }
    args
}
