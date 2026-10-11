use super::{
    CheckoutIdentity, LeaseLock, LeaseRecord, checkout_state_dir,
    output_with_aggregate_limit_until, parse_lease, read_optional_private, resolve_checkout,
    resolve_state_root, sync_directory, unix_time, validate_lease, verify_no_symlink_components,
    verify_private_directory,
};
use anyhow::{Context, Result, ensure};
use std::fs::{self, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Keep the same lock inode as the runtime checkout writer through removal.
/// A caller-owned lease may be released after preservation succeeds.
pub(in crate::worktree) struct RemovalLeaseGuard {
    _lock: LeaseLock,
    identity: Option<(CheckoutIdentity, u64, u64)>,
    own_lease: Option<PathBuf>,
}

impl RemovalLeaseGuard {
    pub(in crate::worktree) fn release_own_lease(&self) -> Result<()> {
        if let Some(path) = &self.own_lease {
            fs::remove_file(path).context("caller checkout lease release failed")?;
            sync_directory(path.parent().context("checkout lease directory missing")?)?;
        }
        Ok(())
    }

    pub(in crate::worktree) fn verify_target(&self) -> Result<()> {
        if let Some((identity, device, inode)) = &self.identity {
            let after = resolve_checkout(
                &identity.root,
                false,
                Instant::now() + Duration::from_secs(10),
            )?;
            let metadata = fs::metadata(&after.git_dir)?;
            ensure!(
                &after == identity && (metadata.dev(), metadata.ino()) == (*device, *inode),
                "removal checkout identity changed"
            );
        }
        Ok(())
    }
}

pub(in crate::worktree) fn fence_removal(
    checkout: &Path,
    caller_key: Option<&str>,
) -> Result<(Option<RemovalLeaseGuard>, Option<String>, Vec<String>)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let identity = resolve_checkout(checkout, true, deadline)?;
    let root = resolve_state_root()?;
    let directory = root
        .join(&identity.repository_key)
        .join(&identity.checkout_key);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)?;
    let directory = checkout_state_dir(&root, &identity)?;
    let path = directory.join("lease.json");
    let read_lease = || -> Result<Option<LeaseRecord>> {
        let Some(bytes) = read_optional_private(&path, "checkout lease")? else {
            return Ok(None);
        };
        let lease = parse_lease(&bytes)?;
        validate_lease(&lease, &identity)?;
        Ok(Some(lease))
    };
    // A busy lock does not erase readable positive foreign ownership evidence.
    if let Ok(Some(lease)) = read_lease()
        && lease.expires_at() > unix_time()?
        && caller_key != Some(lease.session_key())
    {
        return Ok((None, Some(lease.session_key().to_owned()), Vec::new()));
    }
    let lock = LeaseLock::acquire_until(&directory, deadline)?;
    let mut warnings = Vec::new();
    let mut active = None;
    let mut own_lease = None;
    let lease = read_lease();
    match lease {
        Ok(Some(lease)) if lease.expires_at() > unix_time()? => {
            if caller_key == Some(lease.session_key()) {
                own_lease = Some(path);
            } else {
                active = Some(lease.session_key().to_owned());
            }
        }
        Ok(_) => {}
        Err(error) => warnings.push(format!("checkout lease unavailable: {error}")),
    }
    let metadata = fs::metadata(&identity.git_dir)?;
    Ok((
        Some(RemovalLeaseGuard {
            _lock: lock,
            identity: Some((identity, metadata.dev(), metadata.ino())),
            own_lease,
        }),
        active,
        warnings,
    ))
}

pub(in crate::worktree) fn removal_registry_lock(directory: &Path) -> Result<RemovalLeaseGuard> {
    // Registry and checkout leases use the same flock protocol and trust rules,
    // but the registry owner names its inode registry.lock.
    verify_no_symlink_components(directory)?;
    verify_private_directory(directory)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(directory.join("registry.lock"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1,
        "coordination registry lock is untrusted"
    );
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "coordination registry lock is busy"
    );
    Ok(RemovalLeaseGuard {
        _lock: LeaseLock(file),
        identity: None,
        own_lease: None,
    })
}

pub(in crate::worktree) fn removal_probe(command: &mut Command) -> Result<std::process::Output> {
    output_with_aggregate_limit_until(
        command,
        Instant::now() + Duration::from_secs(15),
        8 * 1024 * 1024,
        1024 * 1024,
        8 * 1024 * 1024,
    )
}
