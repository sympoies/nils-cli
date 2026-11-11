use crate::*;

#[cfg(target_os = "linux")]
pub(crate) fn open_verified_linux_control_group(
    control_group: &TmuxControlGroupIdentity,
    pane_pid: libc::pid_t,
    captured_members: &[TmuxProcessIdentity],
    allow_split_scope: bool,
) -> Result<PinnedLinuxControlGroup, SessionTerminationFailure> {
    let path = Path::new(&control_group.path);
    if !valid_tmux_spawn_control_group_path(path) {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    }
    if linux_process_control_group_path(unsafe { libc::getpid() })?.as_deref() == Some(path) {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    }
    if linux_process_control_group(pane_pid)?.as_ref() != Some(control_group) {
        return Err(SessionTerminationFailure::VerificationFailed);
    }
    if !allow_split_scope {
        verify_captured_members_cgroup(control_group, captured_members)?;
    }

    let full_path = linux_control_group_full_path(path)?;
    open_pinned_linux_control_group(control_group, &full_path)
}

#[cfg(target_os = "linux")]
pub(crate) fn freeze_and_revalidate_linux_control_group(
    pinned: &mut PinnedLinuxControlGroup,
    control_group: &TmuxControlGroupIdentity,
    pane_pid: libc::pid_t,
    captured_members: &[TmuxProcessIdentity],
    allow_split_scope: bool,
) -> Result<(), SessionTerminationFailure> {
    freeze_pinned_control_group(pinned, DELETE_TERMINATION_VERIFY_TIMEOUT)?;
    if linux_process_control_group(pane_pid)?.as_ref() != Some(control_group) {
        return Err(SessionTerminationFailure::VerificationFailed);
    }
    if !allow_split_scope {
        verify_captured_members_cgroup(control_group, captured_members)?;
    }
    ensure_linux_control_group_is_leaf(&pinned.directory_fd)?;
    Ok(())
}

fn verify_captured_members_cgroup(
    control_group: &TmuxControlGroupIdentity,
    captured_members: &[TmuxProcessIdentity],
) -> Result<(), SessionTerminationFailure> {
    for member in captured_members {
        let observed = linux_process_control_group(member.pid)?;
        if observed.as_ref() == Some(control_group) {
            continue;
        }
        let outside = observed.is_some()
            || linux_process_control_group_path(member.pid)?
                .is_some_and(|path| path != Path::new(&control_group.path));
        // None also describes a live process in the observer's original or an
        // unsupported cgroup. Only a still-live captured incarnation proves
        // the permanent mismatch; exit/reuse/uncertainty keeps the old retry.
        let live_captured_member = read_linux_process_identity(member.pid)?
            .is_some_and(|current| !current.zombie && current.start_time == member.start_time);
        return Err(if outside && live_captured_member {
            SessionTerminationFailure::ProcessSessionMemberCgroupMismatch
        } else {
            SessionTerminationFailure::VerificationFailed
        });
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(crate) fn recover_persisted_linux_control_group_at_root(
    control_group: &TmuxControlGroupIdentity,
    control_group_members: &[TmuxProcessIdentity],
    state: TmuxTerminationState,
    root: &Path,
) -> Result<LinuxControlGroupRecovery, SessionTerminationFailure> {
    let full_path = linux_control_group_full_path_at_root(Path::new(&control_group.path), root)?;
    let current_boot_id = linux_boot_id()?;
    match control_group.boot_id.as_deref() {
        Some(boot_id) if boot_id == current_boot_id => {}
        Some(_) => return Ok(LinuxControlGroupRecovery::DifferentBoot),
        None if !full_path.exists() => return Ok(LinuxControlGroupRecovery::CurrentBoot),
        None => return Err(SessionTerminationFailure::RuntimeIdentityUnavailable),
    }
    if matches!(state, TmuxTerminationState::KillConfirmed { .. })
        && (control_group_members.iter().any(|member| {
            crate::delete_recovery::codex_daemons::proven_pid(member.pid, Some(member.start_time))
        }) || crate::delete_recovery::codex_daemons::shared_group_at_root(control_group, root)?)
    {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    }
    let mut pinned_control_group = match fs::metadata(&full_path) {
        Ok(_) => Some(open_pinned_linux_control_group(control_group, &full_path)?),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(_) => return Err(SessionTerminationFailure::VerificationFailed),
    };
    let pinned_processes = if matches!(state, TmuxTerminationState::KillConfirmed { .. }) {
        control_group_members
            .iter()
            .map(pin_matching_linux_process)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if matches!(state, TmuxTerminationState::KillConfirmed { .. }) {
        if let Some(pinned) = pinned_control_group.as_ref() {
            write_control_group_file(&pinned.kill_fd, b"1")?;
        }
        for process in &pinned_processes {
            pidfd_send_signal(&process.pidfd, libc::SIGKILL, true)?;
        }
    }
    if state.thaw_on_recovery()
        && let Some(pinned) = pinned_control_group.as_mut()
    {
        pinned.thaw_on_drop = true;
        if !thaw_pinned_control_group(pinned) {
            return Err(SessionTerminationFailure::VerificationFailed);
        }
        pinned.thaw_on_drop = false;
    }
    Ok(LinuxControlGroupRecovery::CurrentBoot)
}
