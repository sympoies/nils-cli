//! Scheduler-only admission tied to the already consumed recovery generation.
use super::*;

/// A live submit releases the record lock for provider ingress. This separate
/// nonblocking fence distinguishes it from a crash-left checking claim. Closing
/// the descriptor releases the fence, including after process termination.
pub(super) fn try_attempt(
    context: &CliContext,
    id: &str,
) -> Result<Option<std::fs::File>, CliError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    let unavailable = |_| {
        CliError::runtime(
            "auto-resume-attempt-unavailable",
            "scheduler attempt fence is unavailable",
            None,
        )
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(session_dir(context, id).join("auto-resume-active.lock"))
        .map_err(unavailable)?;
    // SAFETY: the descriptor is owned by this live File; flock releases on close.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(file));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(unavailable(error))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recovery {
    pub(crate) generation: String,
    pub(crate) cause: String,
    pub(crate) predecessor: String,
    pub(crate) revision: u64,
}

impl Recovery {
    pub(crate) fn valid(&self) -> bool {
        crate::send::claude_notice::store::hash_valid(&self.generation)
            && matches!(
                self.cause.as_str(),
                USAGE_EXHAUSTION_CAUSE | PROVIDER_CAPACITY_CAUSE
            )
            && !self.predecessor.is_empty()
            && self.predecessor.len() <= 256
    }
}

pub(crate) fn claimed(context: &CliContext, record: &SessionRecord) -> Result<Recovery, CliError> {
    let refused = || {
        CliError::runtime(
            "notice-continuation-superseded",
            "scheduler continuation identity changed",
            None,
        )
    };
    let state = read_state(context, &record.id, &Timestamp::now().to_string())?;
    let predecessor = state.blocked_turn_id.clone().ok_or_else(refused)?;
    let activity = crate::activity::state_for_view(context, record).ok_or_else(refused)?;
    if state.state != "checking"
        || !state.enabled
        || !blocked_claim_is_eligible(context, record, &state)
        || activity
            .last_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.as_deref())
            != Some(predecessor.as_str())
    {
        return Err(refused());
    }
    let claim = Recovery {
        generation: crate::send::claude_notice::sha256(
            &serde_json::to_vec(&state).map_err(|_| refused())?,
        ),
        cause: state.recovery_cause.clone().ok_or_else(refused)?,
        predecessor,
        revision: state.blocked_revision.ok_or_else(refused)?,
    };
    if !claim.valid() {
        return Err(refused());
    }
    Ok(claim)
}

pub(crate) fn matches(context: &CliContext, record: &SessionRecord, claim: &Recovery) -> bool {
    claimed(context, record).ok().as_ref() == Some(claim)
}
