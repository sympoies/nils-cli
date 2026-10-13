//! Advertise ingress before waiting for the record lock. Guard probes are brief;
//! writers share this lock and wait for a probe rather than dropping an event.
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use super::ActivityLockMode;
use crate::{CliContext, CliError, SessionRecordLock, session_dir};

const INTENT_FILE: &str = ".activity-ingress.lock";

pub(super) struct IngressRecordLock {
    _record: SessionRecordLock,
    _intent: File,
}

fn open(context: &CliContext, id: &str) -> Result<File, CliError> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(session_dir(context, id).join(INTENT_FILE))
        .map_err(lock_error)
}

pub(super) fn acquire_record(
    context: &CliContext,
    id: &str,
    mode: ActivityLockMode,
) -> Result<IngressRecordLock, CliError> {
    let intent = open(context, id)?;
    loop {
        // SAFETY: intent owns this valid file descriptor.
        if unsafe { libc::flock(intent.as_raw_fd(), libc::LOCK_SH) } == 0 {
            break;
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(lock_error(err));
        }
    }
    let record = match mode {
        ActivityLockMode::Blocking => crate::acquire_session_record_lock(context, id)?,
        ActivityLockMode::NonBlocking => crate::try_acquire_session_record_lock(context, id)?
            .ok_or_else(|| {
                CliError::runtime(
                    "activity-lock-busy",
                    "activity state is busy",
                    Some(serde_json::json!({"id": id})),
                )
            })?,
        ActivityLockMode::Timed(timeout) => {
            crate::acquire_session_record_lock_timed(context, id, timeout)?
        }
    };
    Ok(IngressRecordLock {
        _record: record,
        _intent: intent,
    })
}

pub(crate) fn probe_lock(context: &CliContext, id: &str) -> Result<Option<File>, CliError> {
    let file = open(context, id)?;
    // SAFETY: file owns this valid descriptor. Never wait behind an ingress.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(file));
    }
    let err = io::Error::last_os_error();
    if err.kind() == io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(lock_error(err))
    }
}

pub(crate) fn event_pending(context: &CliContext, id: &str) -> Result<bool, CliError> {
    // Closing this temporary descriptor releases the probe before any pane I/O.
    Ok(probe_lock(context, id)?.is_none())
}

fn lock_error(err: io::Error) -> CliError {
    CliError::runtime(
        "activity-ingress-lock-failed",
        format!("provider ingress locking failed: {err}"),
        None,
    )
}
