//! Auth recovery never types into a failed provider or sends a continuation.
use super::*;

pub(crate) struct Fence {
    _fence: RestartFence,
}

pub(crate) fn fence(context: &CliContext, record: &SessionRecord) -> Result<Fence, CliError> {
    if std::env::var("AGENT_SESSION_ID").is_ok_and(|id| id == record.id) {
        return Err(refusal(
            "restart-own-session",
            "a session cannot stop its own runtime",
            record,
        ));
    }
    let launch = launch_id(record).ok_or_else(|| {
        refusal(
            "restart-identity-unverified",
            "runtime identity is missing",
            record,
        )
    })?;
    Ok(Fence {
        _fence: RestartFence::acquire(context, record, &launch)?,
    })
}

pub(crate) fn retire(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
) -> Result<(), CliError> {
    coordination::retire_after_verified_stop(context, record, tmux)?;
    await_prior_lease(context, record)
}

pub(crate) fn ready(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
) -> Result<(), CliError> {
    let launch = launch_id(record).unwrap_or_default();
    await_ready(
        context,
        record,
        &launch,
        tmux,
        Instant::now() + Duration::from_secs(10),
    )
    .map_err(|_| {
        refusal(
            "auth-recovery-not-ready",
            "the resumed provider did not become ready",
            record,
        )
    })
}
