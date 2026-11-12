//! Head binding for `review-loop extend` when the author repaired after a stop.
use super::*;

/// Returns the head the extension record binds to.
///
/// `expected_head` is the provider head, already verified by the caller. It
/// binds directly when it is the ledger head or the stopped head. Otherwise the
/// author pushed a repair after the last observation: the extension binds the
/// ledger anchor (the stopped head, else the ledger head, both heads the ledger
/// already accepts) once the provider proves the repaired head descends from it.
/// Unrelated heads stay refused.
pub(super) fn bind<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    repository: &str,
    previous: &review_state::ReviewLoopState,
    expected_head: &str,
) -> Result<String, ForgeError> {
    let stopped = previous
        .hard_stop
        .as_ref()
        .map(|stop| stop.attempted_head_sha.as_str());
    if previous.head_sha == expected_head || stopped == Some(expected_head) {
        return Ok(expected_head.to_string());
    }
    let anchor = stopped.unwrap_or(&previous.head_sha);
    ensure_descendant(runner, ctx, repository, anchor, expected_head)?;
    Ok(anchor.to_string())
}

fn ensure_descendant<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    repository: &str,
    anchor: &str,
    head: &str,
) -> Result<(), ForgeError> {
    let mut argv = vec![OsString::from("api")];
    ctx.push_github_api_hostname(&mut argv);
    argv.push(format!("repos/{repository}/compare/{anchor}...{head}?per_page=1").into());
    let output = runner.run(&BackendCall::new(BackendProgram::Gh, argv))?;
    let value: serde_json::Value = serde_json::from_str(output.stdout.trim()).unwrap_or_default();
    let descends = value["status"] == "ahead"
        && value["behind_by"] == 0
        && value
            .pointer("/merge_base_commit/sha")
            .and_then(|v| v.as_str())
            == Some(anchor);
    if descends {
        return Ok(());
    }
    Err(conflict(
        "the provider pull-request head does not descend from the review-loop ledger head",
        format!("ledger_head={anchor}; expected_head={head}"),
    ))
}

fn conflict(message: &str, detail: String) -> ForgeError {
    ForgeError::validation(schema_err(), "review_state_conflict", message, Some(detail))
}
