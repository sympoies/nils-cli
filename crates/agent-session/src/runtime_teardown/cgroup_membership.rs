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
