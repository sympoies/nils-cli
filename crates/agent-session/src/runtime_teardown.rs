use crate::*;

#[cfg(target_os = "linux")]
pub(super) const CGROUP_MEMBERSHIP_OBSERVATION_DURATION: Duration = Duration::from_millis(100);

pub(super) fn retire_confirmed_broker(
    context: &CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
    state: Option<TmuxTerminationState>,
) -> Result<(), CliError> {
    if matches!(state, Some(TmuxTerminationState::KillConfirmed { .. })) {
        coordination::retire_after_verified_stop(context, record, tmux_bin)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn prepare_macos_process_runtime(
    identity: &TmuxRuntimeIdentity,
) -> Result<PinnedProcessRuntime, SessionTerminationFailure> {
    Ok(PinnedProcessRuntime {
        macos_pane: current_macos_pane(identity)?,
    })
}

#[cfg(target_os = "macos")]
fn current_macos_pane(
    identity: &TmuxRuntimeIdentity,
) -> Result<TmuxProcessIdentity, SessionTerminationFailure> {
    let group = identity
        .process_group_id
        .filter(|group| *group > 1 && *group != unsafe { libc::getpgrp() })
        .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
    if identity.macos_boot_id.is_none()
        || identity.macos_boot_id != capture_macos_boot_id()
        || identity.pane_pid <= 1
    {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    }
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            identity.pane_pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read != size {
        return Err(SessionTerminationFailure::ProcessIdentityUnavailable);
    }
    let info = unsafe { info.assume_init() };
    if info.pbi_pid != identity.pane_pid as u32
        || info.pbi_pgid != group as u32
        || info.pbi_status == libc::SZOMB
    {
        return Err(SessionTerminationFailure::RuntimeIdentityMismatch);
    }
    let start_time = info
        .pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .filter(|start| *start > 0)
        .ok_or(SessionTerminationFailure::ProcessIdentityUnavailable)?;
    Ok(TmuxProcessIdentity {
        pid: identity.pane_pid,
        start_time,
    })
}

pub(super) fn tmux_kill_captured_runtime_with_timeout(
    tmux_bin: &Path,
    identity: &TmuxRuntimeIdentity,
    pinned: &PinnedProcessRuntime,
    timeout: Duration,
) -> Result<TmuxIdentityKillOutcome, SessionTerminationFailure> {
    #[cfg(target_os = "macos")]
    {
        if current_macos_pane(identity)? != pinned.macos_pane {
            return Err(SessionTerminationFailure::RuntimeIdentityMismatch);
        }
        let group = identity
            .process_group_id
            .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
        // The exact tmux identity guard runs before the group signal, while
        // the same-boot, same-start-time pane still anchors that group.
        let guarded_kill = format!(
            "run-shell \"/bin/kill -KILL -- -{group}\"; kill-session -t {}",
            identity.session_id,
        );
        tmux_kill_identity(tmux_bin, identity, timeout, Some(guarded_kill))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pinned;
        tmux_kill_identity_with_timeout(tmux_bin, identity, timeout)
    }
}

pub(super) fn terminate_captured_process_runtime(
    identity: &TmuxRuntimeIdentity,
    pinned_runtime: &mut PinnedProcessRuntime,
    verify_timeout: Duration,
) -> Result<(), SessionTerminationFailure> {
    #[cfg(target_os = "linux")]
    {
        let process_session_id = identity
            .process_session_id
            .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
        if process_session_id <= 1 || process_session_id == unsafe { libc::getsid(0) } {
            return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
        }
        let thaw_after_kill = pinned_runtime
            .control_group
            .as_mut()
            .is_some_and(|control_group| {
                let thaw_after_kill = control_group.thaw_on_drop;
                control_group.thaw_on_drop = false;
                thaw_after_kill
            });
        kill_pinned_control_group(pinned_runtime)?;
        signal_pinned_process_runtime(pinned_runtime, libc::SIGKILL)?;
        if thaw_after_kill {
            let control_group = pinned_runtime
                .control_group
                .as_ref()
                .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
            if !thaw_pinned_control_group(control_group) {
                return Err(SessionTerminationFailure::VerificationFailed);
            }
            signal_pinned_process_runtime(pinned_runtime, libc::SIGKILL)?;
        }
        verify_stopped_pinned_process_runtime(pinned_runtime, verify_timeout)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pinned_runtime;
        verify_stopped_process_runtime(identity, verify_timeout)
    }
}

pub(super) fn tmux_kill_identity_with_timeout(
    tmux_bin: &Path,
    identity: &TmuxRuntimeIdentity,
    timeout: Duration,
) -> Result<TmuxIdentityKillOutcome, SessionTerminationFailure> {
    tmux_kill_identity(tmux_bin, identity, timeout, None)
}

fn tmux_kill_identity(
    tmux_bin: &Path,
    identity: &TmuxRuntimeIdentity,
    timeout: Duration,
    guarded_kill: Option<String>,
) -> Result<TmuxIdentityKillOutcome, SessionTerminationFailure> {
    #[cfg(target_os = "macos")]
    let group_termination = guarded_kill.is_some();
    let condition = format!(
        "#{{&&:#{{==:#{{session_id}},{}}},#{{&&:#{{==:#{{pane_id}},{}}},#{{==:#{{pane_pid}},{}}}}}}}",
        identity.session_id, identity.pane_id, identity.pane_pid
    );
    #[cfg(target_os = "macos")]
    let condition = if group_termination {
        format!("#{{&&:#{{==:#{{pane_dead}},0}},{condition}}}")
    } else {
        condition
    };
    let mut command = ProcessCommand::new(tmux_bin);
    command
        .arg("if-shell")
        .arg("-F")
        .arg("-t")
        .arg(&identity.pane_id)
        .arg(condition)
        .arg(guarded_kill.unwrap_or_else(|| format!("kill-session -t {}", identity.session_id)))
        .arg(format!(
            "display-message -p {TMUX_RUNTIME_IDENTITY_CHANGED_OUTPUT}"
        ));
    let output = run_output_with_timeout(command, timeout).map_err(|err| {
        if err.kind() == io::ErrorKind::TimedOut {
            SessionTerminationFailure::KillTimeout
        } else {
            SessionTerminationFailure::KillError
        }
    })?;
    if !output.status.success() {
        // Group termination can remove the last pane/session before tmux's
        // following kill-session command. Accept only verified old-boundary
        // absence, never a fallback kill by the reusable managed name.
        #[cfg(target_os = "macos")]
        if group_termination
            && verify_stopped_process_runtime(identity, timeout).is_ok()
            && verified_tmux_status_with_timeout(tmux_bin, &identity.session_id, timeout)
                == "stopped"
        {
            return Ok(TmuxIdentityKillOutcome::KillConfirmed);
        }
        return Err(SessionTerminationFailure::KillFailed);
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| SessionTerminationFailure::VerificationFailed)?;
    match stdout.trim() {
        "" => Ok(TmuxIdentityKillOutcome::KillConfirmed),
        TMUX_RUNTIME_IDENTITY_CHANGED_OUTPUT => Ok(TmuxIdentityKillOutcome::IdentityChanged),
        _ => Err(SessionTerminationFailure::VerificationFailed),
    }
}
