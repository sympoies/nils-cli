//! Same-boot macOS evidence for the temp sweep under the session-record lock.
use super::{
    heartbeat_lock_path, macos_process_group_is_empty, stopped_macos_broker_matches_record,
};
use crate::coordination::{
    coordination_dir, heartbeat_path, lock_registry_observational, now_epoch, read_private_file,
};
use crate::{CliContext, CoordinationRuntimeStatus, SessionRecord, TmuxRuntimeIdentity};
use std::fs;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use CoordinationRuntimeStatus::{Running, Stopped, Unknown};

pub(crate) fn macos_tmp_runtime_status(
    context: &CliContext,
    tmux: &Path,
    record: &SessionRecord,
    identity: &TmuxRuntimeIdentity,
    boot: &str,
) -> CoordinationRuntimeStatus {
    if identity.macos_boot_id.as_deref() != Some(boot)
        || record.runtime.as_ref().is_none_or(|runtime| {
            runtime.kind != "tmux" || runtime.tmux_session != record.tmux_session
        })
        || !fs::symlink_metadata(coordination_dir(context, &record.id))
            .is_ok_and(|metadata| metadata.is_dir() && private(&metadata))
    {
        return Unknown;
    }
    let Ok(locked) = lock_registry_observational(context) else {
        return Unknown;
    };
    let Some(broker) = locked.registry.brokers.get(&record.id) else {
        return Unknown;
    };
    let now = now_epoch();
    if !(1..=now).contains(&broker.heartbeat_epoch)
        || !stopped_macos_broker_matches_record(context, broker, record, identity)
    {
        return Unknown;
    }
    let owner = heartbeat_owner_status(
        context,
        &record.id,
        &broker.incarnation,
        broker.heartbeat_epoch,
    );
    if owner != Stopped {
        return owner;
    }
    // Lifecycle writers that could replace this broker also acquire the
    // session-record lock. Release the registry before probing tmux and ps.
    drop(locked);

    let Ok(prior) = crate::persisted_prior_tmux_runtime_identities(record) else {
        return Unknown;
    };
    if std::iter::once(identity).chain(&prior).all(|identity| {
        identity.macos_boot_id.as_deref() == Some(boot)
            && crate::verified_tmux_status_with_timeout(
                tmux,
                &identity.session_id,
                crate::DELETE_TERMINATION_VERIFY_TIMEOUT,
            ) == "stopped"
            && crate::coordination::retirement::process_absent(identity.pane_pid)
            && macos_process_group_is_empty(identity)
    }) {
        Stopped
    } else {
        Unknown
    }
}

fn private(metadata: &fs::Metadata) -> bool {
    // SAFETY: geteuid has no preconditions.
    metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0
}

fn heartbeat_owner_status(
    context: &CliContext,
    id: &str,
    incarnation: &str,
    revoked_at: i64,
) -> CoordinationRuntimeStatus {
    let lock = match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(heartbeat_lock_path(context, id, incarnation))
    {
        Ok(file) => Some(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(_) => return Unknown,
    };
    if let Some(file) = lock.as_ref() {
        if !file
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.nlink() == 1 && private(&metadata))
        {
            return Unknown;
        }
        // SAFETY: the file owns a valid descriptor. No blocking lock is taken.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return if io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
                Running
            } else {
                Unknown
            };
        }
    }
    let path = heartbeat_path(&context.state_dir, id);
    match read_private_file(&path, 256) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Stopped,
        Ok(bytes) if lock.is_some() => {
            // A cached beat can accompany exact revocation if it predates the
            // stop. The kernel lock above provides writer liveness evidence.
            let beat = std::str::from_utf8(&bytes)
                .ok()
                .and_then(|value| value.trim().rsplit_once(':'));
            if beat.is_some_and(|(owner, epoch)| {
                owner == incarnation
                    && epoch
                        .parse()
                        .is_ok_and(|epoch| (1..=revoked_at).contains(&epoch))
            }) {
                Stopped
            } else {
                Unknown
            }
        }
        _ => Unknown,
    }
}
