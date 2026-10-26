use crate::*;

#[cfg(target_os = "linux")]
pub(crate) fn open_verified_linux_control_group(
    control_group: &TmuxControlGroupIdentity,
    pane_pid: libc::pid_t,
    captured_members: &[TmuxProcessIdentity],
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
    for member in captured_members {
        if linux_process_control_group(member.pid)?.as_ref() != Some(control_group) {
            return Err(SessionTerminationFailure::ProcessSessionMemberCgroupMismatch);
        }
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
) -> Result<(), SessionTerminationFailure> {
    freeze_pinned_control_group(pinned, DELETE_TERMINATION_VERIFY_TIMEOUT)?;
    if linux_process_control_group(pane_pid)?.as_ref() != Some(control_group) {
        return Err(SessionTerminationFailure::VerificationFailed);
    }
    for member in captured_members {
        if linux_process_control_group(member.pid)?.as_ref() != Some(control_group) {
            return Err(SessionTerminationFailure::ProcessSessionMemberCgroupMismatch);
        }
    }
    ensure_linux_control_group_is_leaf(&pinned.directory_fd)?;
    Ok(())
}
