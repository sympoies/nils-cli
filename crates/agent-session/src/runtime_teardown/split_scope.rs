use crate::*;

pub(crate) fn set_termination_state(
    record: &mut SessionRecord,
    state: TmuxTerminationState,
    recover_split_cgroup: Option<bool>,
) -> Result<(), SessionTerminationFailure> {
    let recover_split_cgroup = match recover_split_cgroup {
        Some(enabled) => enabled,
        None => persisted_recovery_mode(record)?,
    };
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .filter(|launch_id| !launch_id.is_empty());
    let paused_members = record
        .extra
        .get(DELETE_TMUX_TERMINATION_STATE_KEY)
        .and_then(|value| value.get("split_scope_paused_members"))
        .cloned();
    let mut state_value = json!({
        "launch_id": launch_id,
        "state": match state {
            TmuxTerminationState::FreezePending { .. } => "freeze-pending",
            TmuxTerminationState::Pending { .. } => "pending",
            TmuxTerminationState::KillConfirmed { .. } => "kill-confirmed",
        },
        "thaw_on_recovery": state.thaw_on_recovery(),
        "recover_split_cgroup": recover_split_cgroup,
    });
    if let Some(paused_members) = paused_members {
        state_value["split_scope_paused_members"] = paused_members;
    }
    record
        .extra
        .insert(DELETE_TMUX_TERMINATION_STATE_KEY.to_string(), state_value);
    Ok(())
}

pub(crate) fn persisted_recovery_mode(
    record: &SessionRecord,
) -> Result<bool, SessionTerminationFailure> {
    let Some(value) = record.extra.get(DELETE_TMUX_TERMINATION_STATE_KEY) else {
        return Ok(false);
    };
    match value.get("recover_split_cgroup") {
        None => Ok(false),
        Some(Value::Bool(enabled)) => Ok(*enabled),
        _ => Err(SessionTerminationFailure::RuntimeIdentityUnavailable),
    }
}

pub(crate) fn persisted_termination_state(
    record: &SessionRecord,
) -> Result<Option<TmuxTerminationState>, SessionTerminationFailure> {
    let Some(value) = record.extra.get(DELETE_TMUX_TERMINATION_STATE_KEY) else {
        return Ok(None);
    };
    let launch_id = match value.get("launch_id") {
        Some(Value::Null) => None,
        Some(Value::String(launch_id)) if !launch_id.is_empty() => Some(launch_id.as_str()),
        _ => return Err(SessionTerminationFailure::RuntimeIdentityUnavailable),
    };
    let current_launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .filter(|launch_id| !launch_id.is_empty());
    if launch_id != current_launch_id {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    }
    let thaw_on_recovery = value
        .get("thaw_on_recovery")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match value.get("state").and_then(Value::as_str) {
        Some("freeze-pending") => Ok(Some(TmuxTerminationState::FreezePending {
            thaw_on_recovery,
        })),
        Some("pending") => Ok(Some(TmuxTerminationState::Pending { thaw_on_recovery })),
        Some("kill-confirmed") => Ok(Some(TmuxTerminationState::KillConfirmed {
            thaw_on_recovery,
        })),
        _ => Err(SessionTerminationFailure::RuntimeIdentityUnavailable),
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TmuxTerminationOptions {
    timeouts: (Duration, Duration),
    terminate_process_group: bool,
    recover_split_cgroup: bool,
}

impl TmuxTerminationOptions {
    pub(crate) fn regular(timeouts: (Duration, Duration), terminate_process_group: bool) -> Self {
        Self {
            timeouts,
            terminate_process_group,
            recover_split_cgroup: false,
        }
    }

    pub(crate) fn split_scope(
        timeouts: (Duration, Duration),
        terminate_process_group: bool,
    ) -> Self {
        Self {
            timeouts,
            terminate_process_group,
            recover_split_cgroup: true,
        }
    }
}

pub(crate) fn terminate_tmux_session_inner(
    context: &CliContext,
    record: &mut SessionRecord,
    tmux_bin: &Path,
    initial_identity: Option<TmuxRuntimeIdentity>,
    options: TmuxTerminationOptions,
) -> Result<(), SessionTerminationFailure> {
    let TmuxTerminationOptions {
        timeouts: (kill_timeout, verify_timeout),
        terminate_process_group,
        recover_split_cgroup,
    } = options;
    if crate::recorded_runtime_is_from_prior_boot(record) {
        record.extra.remove(DELETE_TMUX_TERMINATION_STATE_KEY);
        return Ok(());
    }
    crate::recover_interrupted_tmux_termination_locked(context, record)?;
    let mut identity_changes = 0;
    let mut initial_identity = initial_identity;
    loop {
        let runtime_probe = match initial_identity.take() {
            Some(identity) => TmuxRuntimeProbe::Running(Box::new(identity)),
            None => crate::capture_tmux_runtime_identity(
                context,
                record,
                tmux_bin,
                verify_timeout.min(DELETE_TERMINATION_PROBE_TIMEOUT),
            )?,
        };
        let mut identity = match runtime_probe {
            TmuxRuntimeProbe::Running(identity) => {
                let mut identity = *identity;
                let mut prior_identities = crate::persisted_prior_tmux_runtime_identities(record)?;
                if prior_identities
                    .iter()
                    .any(|prior| !prior.same_runtime_target(&identity))
                {
                    return Err(SessionTerminationFailure::RuntimeIdentityMismatch);
                }
                if let Some(persisted) = crate::persisted_tmux_runtime_identity(record)? {
                    if !persisted.same_runtime_target(&identity) {
                        return Err(SessionTerminationFailure::RuntimeIdentityMismatch);
                    }
                    if persisted.same_process_identity(&identity) {
                        identity.merge_process_evidence_from(&persisted);
                    } else if !prior_identities
                        .iter()
                        .any(|prior| prior.same_process_identity(&persisted))
                    {
                        prior_identities.push(persisted);
                    }
                }
                crate::persist_tmux_runtime_identities(record, &identity, &prior_identities)?;
                crate::write_session_record(context, record)
                    .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?;
                crate::verify_stopped_process_runtimes(&prior_identities, verify_timeout)?;
                if !prior_identities.is_empty() {
                    crate::persist_tmux_runtime_identities(record, &identity, &[])?;
                    crate::write_session_record(context, record)
                        .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?;
                }
                identity
            }
            TmuxRuntimeProbe::Stopped => {
                if crate::runtime_is_proven_never_launched(record) {
                    return Ok(());
                }
                let identity = crate::persisted_tmux_runtime_identity(record)?
                    .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
                let prior_identities = crate::persisted_prior_tmux_runtime_identities(record)?;
                if prior_identities
                    .iter()
                    .any(|prior| !prior.same_runtime_target(&identity))
                {
                    return Err(SessionTerminationFailure::RuntimeIdentityMismatch);
                }
                let verification_started = Instant::now();
                crate::verify_stopped_process_runtimes(&prior_identities, verify_timeout)?;
                let remaining = verify_timeout.saturating_sub(verification_started.elapsed());
                crate::verify_stopped_tmux_runtime(
                    tmux_bin,
                    &record.tmux_session,
                    &identity,
                    remaining,
                )?;
                match crate::persisted_tmux_termination_state(record)? {
                    Some(TmuxTerminationState::FreezePending { .. }) => {
                        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
                    }
                    Some(TmuxTerminationState::Pending { .. })
                    | Some(TmuxTerminationState::KillConfirmed { .. }) => {
                        record.extra.remove(DELETE_TMUX_TERMINATION_STATE_KEY);
                        crate::write_session_record(context, record)
                            .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?;
                    }
                    None => {}
                }
                return Ok(());
            }
        };

        if identity_changes >= DELETE_TERMINATION_IDENTITY_RETRY_LIMIT {
            return Err(SessionTerminationFailure::RuntimeIdentityChanged);
        }
        if terminate_process_group {
            match crate::terminate_verified_process_runtime_transaction(
                context,
                record,
                &mut identity,
                VerifiedRuntimeTerminationMode::LiveTmux {
                    tmux_bin,
                    kill_timeout,
                },
                verify_timeout,
                recover_split_cgroup,
            )? {
                VerifiedRuntimeTerminationOutcome::Complete => return Ok(()),
                VerifiedRuntimeTerminationOutcome::TmuxIdentityChanged => {
                    identity_changes += 1;
                    continue;
                }
            }
        }

        let thaw_on_recovery = false;
        crate::set_tmux_termination_state(
            record,
            TmuxTerminationState::Pending { thaw_on_recovery },
        )?;
        crate::write_session_record(context, record)
            .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?;
        match crate::tmux_kill_identity_with_timeout(tmux_bin, &identity, kill_timeout)? {
            TmuxIdentityKillOutcome::IdentityChanged => {
                record.extra.remove(DELETE_TMUX_TERMINATION_STATE_KEY);
                crate::write_session_record(context, record)
                    .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?;
                identity_changes += 1;
            }
            TmuxIdentityKillOutcome::KillConfirmed => {
                crate::set_tmux_termination_state(
                    record,
                    TmuxTerminationState::KillConfirmed { thaw_on_recovery },
                )?;
                crate::write_session_record(context, record)
                    .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?;
                let verification_started = Instant::now();
                let remaining = verify_timeout.saturating_sub(verification_started.elapsed());
                crate::verify_stopped_tmux_runtime(
                    tmux_bin,
                    &record.tmux_session,
                    &identity,
                    remaining,
                )?;
                record.extra.remove(DELETE_TMUX_TERMINATION_STATE_KEY);
                crate::write_session_record(context, record)
                    .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?;
                return Ok(());
            }
        }
    }
}

#[cfg(target_os = "linux")]
use crate::runtime_teardown::cgroup_membership::{
    freeze_and_revalidate_linux_control_group, open_verified_linux_control_group,
};

pub(crate) fn prepare_process_runtime(
    identity: &TmuxRuntimeIdentity,
    recover_split_cgroup: bool,
) -> Result<PinnedProcessRuntime, SessionTerminationFailure> {
    #[cfg(target_os = "linux")]
    {
        let process_session_id = identity
            .process_session_id
            .filter(|session_id| *session_id > 1 && *session_id != unsafe { libc::getsid(0) })
            .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
        let control_group_identity = identity
            .control_group
            .as_ref()
            .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
        let control_group = open_verified_linux_control_group(
            control_group_identity,
            identity.pane_pid,
            &identity.process_session_members,
            recover_split_cgroup,
        )?;
        if recover_split_cgroup {
            verify_complete_session_snapshot(identity, process_session_id)?;
        }
        let mut processes = crate::pin_linux_control_group_processes(
            &control_group,
            control_group_identity,
            identity.pane_pid,
            if recover_split_cgroup {
                &[]
            } else {
                &identity.process_session_members
            },
            false,
            DELETE_TERMINATION_VERIFY_TIMEOUT,
        )?;
        if recover_split_cgroup {
            for captured in &identity.process_session_members {
                if contains_identity(&processes, captured) {
                    continue;
                }
                processes.push(pin_process_in_session(captured, process_session_id)?);
            }
            crate::verify_captured_linux_process_session(
                process_session_id,
                &identity.process_session_members,
            )?;
        }
        for captured in &identity.control_group_members {
            if contains_identity(&processes, captured) {
                continue;
            }
            if let Some(process) = crate::pin_matching_linux_process(captured)? {
                processes.push(process);
            }
        }
        Ok(PinnedProcessRuntime {
            processes,
            control_group: Some(control_group),
        })
    }
    #[cfg(target_os = "macos")]
    {
        let _ = recover_split_cgroup;
        runtime_teardown::prepare_macos_process_runtime(identity)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (identity, recover_split_cgroup);
        Ok(PinnedProcessRuntime {})
    }
}

pub(crate) fn freeze_and_pin_process_runtime(
    identity: &TmuxRuntimeIdentity,
    pinned_runtime: &mut PinnedProcessRuntime,
    recover_split_cgroup: bool,
) -> Result<(), SessionTerminationFailure> {
    #[cfg(target_os = "linux")]
    {
        let process_session_id = identity
            .process_session_id
            .filter(|session_id| *session_id > 1 && *session_id != unsafe { libc::getsid(0) })
            .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
        let control_group_identity = identity
            .control_group
            .as_ref()
            .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
        let control_group = pinned_runtime
            .control_group
            .as_mut()
            .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
        freeze_and_revalidate_linux_control_group(
            control_group,
            control_group_identity,
            identity.pane_pid,
            &identity.process_session_members,
            recover_split_cgroup,
        )?;
        if recover_split_cgroup {
            verify_complete_session_snapshot(identity, process_session_id)?;
        } else if identity.process_session_members.is_empty() {
            let pane = crate::read_linux_process_identity(identity.pane_pid)?
                .filter(|pane| !pane.zombie && pane.session_id == process_session_id)
                .ok_or(SessionTerminationFailure::VerificationFailed)?;
            if pane.pid != identity.pane_pid {
                return Err(SessionTerminationFailure::VerificationFailed);
            }
        } else {
            crate::verify_captured_linux_process_session(
                process_session_id,
                &identity.process_session_members,
            )?;
        }
        let frozen_processes = crate::pin_linux_control_group_processes(
            control_group,
            control_group_identity,
            identity.pane_pid,
            if recover_split_cgroup {
                &[]
            } else {
                &identity.process_session_members
            },
            true,
            DELETE_TERMINATION_VERIFY_TIMEOUT,
        )?;
        for process in frozen_processes {
            if !contains_identity(&pinned_runtime.processes, &process.identity) {
                pinned_runtime.processes.push(process);
            }
        }
        if recover_split_cgroup {
            for captured in &identity.process_session_members {
                if contains_identity(&pinned_runtime.processes, captured) {
                    continue;
                }
                pinned_runtime
                    .processes
                    .push(pin_process_in_session(captured, process_session_id)?);
            }
            crate::verify_captured_linux_process_session(
                process_session_id,
                &identity.process_session_members,
            )?;
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (identity, pinned_runtime, recover_split_cgroup);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn verify_complete_session_snapshot(
    identity: &TmuxRuntimeIdentity,
    process_session_id: libc::pid_t,
) -> Result<(), SessionTerminationFailure> {
    if identity.process_session_members.is_empty()
        || !identity
            .process_session_members
            .iter()
            .any(|member| member.pid == identity.pane_pid)
    {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    }
    crate::verify_captured_linux_process_session(
        process_session_id,
        &identity.process_session_members,
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn terminate_persisted_split_members(
    context: &CliContext,
    record: &mut SessionRecord,
    identity: &TmuxRuntimeIdentity,
    timeout: Duration,
) -> Result<(), SessionTerminationFailure> {
    let process_session_id = identity
        .process_session_id
        .filter(|session_id| *session_id > 1 && *session_id != unsafe { libc::getsid(0) })
        .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
    let captured = &identity.process_session_members;
    if captured.is_empty() {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    }
    let current = match crate::linux_process_session_members(process_session_id) {
        Ok(members) => members,
        Err(reason) => {
            resume_persisted_paused_members(context, record, identity)?;
            return Err(reason);
        }
    };
    let mut pinned = Vec::with_capacity(current.len());
    for member in &current {
        let Some(captured_member) = captured.iter().find(|captured| {
            captured.pid == member.pid && captured.start_time == member.start_time
        }) else {
            resume_persisted_paused_members(context, record, identity)?;
            return Err(SessionTerminationFailure::VerificationFailed);
        };
        match pin_process_in_session(captured_member, process_session_id) {
            Ok(process) => pinned.push(process),
            Err(reason) => {
                resume_persisted_paused_members(context, record, identity)?;
                return Err(reason);
            }
        }
    }
    let revalidated = match crate::linux_process_session_members(process_session_id) {
        Ok(members) => members,
        Err(reason) => {
            resume_persisted_paused_members(context, record, identity)?;
            return Err(reason);
        }
    };
    if revalidated.len() != pinned.len()
        || revalidated.iter().any(|member| {
            !pinned.iter().any(|pinned| {
                pinned.identity.pid == member.pid && pinned.identity.start_time == member.start_time
            })
        })
    {
        resume_persisted_paused_members(context, record, identity)?;
        return Err(SessionTerminationFailure::VerificationFailed);
    }
    let mut live_identity = identity.clone();
    live_identity.process_session_members = revalidated
        .iter()
        .map(|member| TmuxProcessIdentity {
            pid: member.pid,
            start_time: member.start_time,
        })
        .collect();
    let _stop_guard = stop_pinned_session_members(
        context,
        record,
        &live_identity,
        &pinned,
        &live_identity.process_session_members,
    )?;
    for process in &pinned {
        crate::pidfd_send_signal(&process.pidfd, libc::SIGKILL, true)?;
    }
    verify_process_session_stopped(process_session_id, timeout)
}

#[cfg(target_os = "linux")]
pub(crate) fn stop_split_scope_members(
    context: &CliContext,
    record: &mut SessionRecord,
    identity: &TmuxRuntimeIdentity,
    pinned_runtime: &PinnedProcessRuntime,
) -> Result<SplitScopeStopGuard, SessionTerminationFailure> {
    stop_pinned_session_members(
        context,
        record,
        identity,
        &pinned_runtime.processes,
        &identity.process_session_members,
    )
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
pub(crate) struct SplitScopeStopGuard {
    stopped_members: Vec<OwnedFd>,
}

#[cfg(target_os = "linux")]
impl Drop for SplitScopeStopGuard {
    fn drop(&mut self) {
        for pidfd in &self.stopped_members {
            let _ = crate::pidfd_send_signal(pidfd, libc::SIGCONT, true);
        }
    }
}

#[cfg(target_os = "linux")]
fn stop_pinned_session_members(
    context: &CliContext,
    record: &mut SessionRecord,
    identity: &TmuxRuntimeIdentity,
    pinned: &[PinnedLinuxProcess],
    expected_members: &[TmuxProcessIdentity],
) -> Result<SplitScopeStopGuard, SessionTerminationFailure> {
    let process_session_id = identity
        .process_session_id
        .filter(|session_id| *session_id > 1 && *session_id != unsafe { libc::getsid(0) })
        .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
    let control_group = identity.control_group.as_ref();
    let mut guard = SplitScopeStopGuard {
        stopped_members: Vec::new(),
    };
    let stop_result = (|| {
        let control_group =
            control_group.ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
        for captured in expected_members {
            let Some(process) = pinned.iter().find(|process| {
                process.identity.pid == captured.pid
                    && process.identity.start_time == captured.start_time
            }) else {
                return Err(SessionTerminationFailure::VerificationFailed);
            };
            let current_cgroup_path = crate::linux_process_control_group_path(captured.pid)?
                .ok_or(SessionTerminationFailure::VerificationFailed)?;
            if current_cgroup_path.as_path() == Path::new(&control_group.path) {
                continue;
            }
            let _current = crate::read_linux_process_identity(captured.pid)?
                .filter(|current| {
                    current.pid == captured.pid
                        && current.session_id == process_session_id
                        && current.start_time == captured.start_time
                        && !current.zombie
                })
                .ok_or(SessionTerminationFailure::VerificationFailed)?;
            if linux_process_is_stopped(captured.pid)? {
                continue;
            }
            let raw_fd =
                unsafe { libc::fcntl(process.pidfd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
            if raw_fd < 0 {
                return Err(SessionTerminationFailure::VerificationFailed);
            }
            let pidfd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
            guard.stopped_members.push(pidfd);
            persist_paused_split_scope_members(context, record, std::slice::from_ref(captured))?;
            crate::pidfd_send_signal(guard.stopped_members.last().unwrap(), libc::SIGSTOP, false)?;
            wait_for_process_stop(captured, process_session_id)?;
        }
        crate::verify_captured_linux_process_session(process_session_id, expected_members)
    })();
    if let Err(reason) = stop_result {
        drop(guard);
        return match resume_persisted_paused_members(context, record, identity) {
            Ok(()) => Err(reason),
            Err(cleanup_reason) => Err(cleanup_reason),
        };
    }
    Ok(guard)
}

#[cfg(target_os = "linux")]
fn persist_paused_split_scope_members(
    context: &CliContext,
    record: &mut SessionRecord,
    additional: &[TmuxProcessIdentity],
) -> Result<(), SessionTerminationFailure> {
    let mut paused = record
        .extra
        .get(DELETE_TMUX_TERMINATION_STATE_KEY)
        .and_then(|value| value.get("split_scope_paused_members"))
        .cloned()
        .map(serde_json::from_value::<Vec<TmuxProcessIdentity>>)
        .transpose()
        .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?
        .unwrap_or_default();
    for member in additional {
        if !paused.iter().any(|prior| prior == member) {
            paused.push(member.clone());
        }
    }
    paused.sort_unstable_by_key(|member| member.pid);
    let Some(state) = record
        .extra
        .get_mut(DELETE_TMUX_TERMINATION_STATE_KEY)
        .and_then(Value::as_object_mut)
    else {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    };
    state.insert(
        "split_scope_paused_members".to_owned(),
        serde_json::to_value(paused)
            .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?,
    );
    crate::write_session_record(context, record)
        .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)
}

#[cfg(target_os = "linux")]
fn resume_persisted_paused_members(
    context: &CliContext,
    record: &mut SessionRecord,
    identity: &TmuxRuntimeIdentity,
) -> Result<(), SessionTerminationFailure> {
    let paused = record
        .extra
        .get(DELETE_TMUX_TERMINATION_STATE_KEY)
        .and_then(|value| value.get("split_scope_paused_members"))
        .cloned()
        .map(serde_json::from_value::<Vec<TmuxProcessIdentity>>)
        .transpose()
        .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?
        .unwrap_or_default();
    let process_session_id = identity
        .process_session_id
        .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
    let mut unresolved = Vec::new();
    let mut first_failure = None;
    for paused_member in paused {
        if !identity.process_session_members.iter().any(|captured| {
            captured.pid == paused_member.pid && captured.start_time == paused_member.start_time
        }) {
            first_failure.get_or_insert(SessionTerminationFailure::RuntimeIdentityUnavailable);
            unresolved.push(paused_member);
            continue;
        }
        let current = match crate::read_linux_process_identity(paused_member.pid) {
            Ok(current) => current,
            Err(reason) => {
                first_failure.get_or_insert(reason);
                unresolved.push(paused_member);
                continue;
            }
        };
        let Some(current) = current else {
            continue;
        };
        if current.start_time != paused_member.start_time
            || current.session_id != process_session_id
            || current.zombie
        {
            first_failure.get_or_insert(SessionTerminationFailure::VerificationFailed);
            unresolved.push(paused_member);
            continue;
        }
        let stopped = match linux_process_is_stopped(paused_member.pid) {
            Ok(stopped) => stopped,
            Err(reason) => {
                first_failure.get_or_insert(reason);
                unresolved.push(paused_member);
                continue;
            }
        };
        if stopped {
            let process = match pin_process_in_session(&paused_member, process_session_id) {
                Ok(process) => process,
                Err(reason) => {
                    first_failure.get_or_insert(reason);
                    unresolved.push(paused_member);
                    continue;
                }
            };
            if let Err(reason) = crate::pidfd_send_signal(&process.pidfd, libc::SIGCONT, false) {
                first_failure.get_or_insert(reason);
                unresolved.push(paused_member);
            }
        }
    }
    let Some(state) = record
        .extra
        .get_mut(DELETE_TMUX_TERMINATION_STATE_KEY)
        .and_then(Value::as_object_mut)
    else {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    };
    if unresolved.is_empty() {
        state.remove("split_scope_paused_members");
    } else {
        state.insert(
            "split_scope_paused_members".to_owned(),
            serde_json::to_value(unresolved)
                .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?,
        );
    }
    crate::write_session_record(context, record)
        .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?;
    match first_failure {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

#[cfg(target_os = "linux")]
fn wait_for_process_stop(
    captured: &TmuxProcessIdentity,
    process_session_id: libc::pid_t,
) -> Result<(), SessionTerminationFailure> {
    let started_at = Instant::now();
    loop {
        let current = crate::read_linux_process_identity(captured.pid)?
            .filter(|current| {
                current.pid == captured.pid
                    && current.start_time == captured.start_time
                    && current.session_id == process_session_id
                    && !current.zombie
            })
            .ok_or(SessionTerminationFailure::VerificationFailed)?;
        let _ = current;
        if linux_process_is_stopped(captured.pid)? {
            return Ok(());
        }
        if started_at.elapsed() >= DELETE_TERMINATION_VERIFY_TIMEOUT {
            return Err(SessionTerminationFailure::VerificationFailed);
        }
        thread::sleep(
            DELETE_TERMINATION_VERIFY_POLL_INTERVAL
                .min(DELETE_TERMINATION_VERIFY_TIMEOUT.saturating_sub(started_at.elapsed())),
        );
    }
}

#[cfg(target_os = "linux")]
fn linux_process_is_stopped(pid: libc::pid_t) -> Result<bool, SessionTerminationFailure> {
    let tasks = fs::read_dir(format!("/proc/{pid}/task"))
        .map_err(|_| SessionTerminationFailure::VerificationFailed)?;
    let mut observed = false;
    for task in tasks {
        let task = task.map_err(|_| SessionTerminationFailure::VerificationFailed)?;
        let stat_path = task.path().join("stat");
        let stat = match fs::read_to_string(&stat_path) {
            Ok(stat) => stat,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return Err(SessionTerminationFailure::VerificationFailed),
        };
        let state = stat
            .rsplit_once(") ")
            .and_then(|(_, rest)| rest.as_bytes().first().copied())
            .ok_or(SessionTerminationFailure::VerificationFailed)?;
        observed = true;
        if !matches!(state, b'T' | b't') {
            return Ok(false);
        }
    }
    Ok(observed)
}

#[cfg(target_os = "linux")]
pub(crate) fn verify_process_session_stopped(
    process_session_id: libc::pid_t,
    timeout: Duration,
) -> Result<(), SessionTerminationFailure> {
    let started_at = Instant::now();
    loop {
        if crate::linux_process_session_members(process_session_id)?.is_empty() {
            return Ok(());
        }
        if started_at.elapsed() >= timeout {
            return Err(SessionTerminationFailure::ProcessStillRunning);
        }
        thread::sleep(
            DELETE_TERMINATION_VERIFY_POLL_INTERVAL
                .min(timeout.saturating_sub(started_at.elapsed())),
        );
    }
}

#[cfg(target_os = "linux")]
fn contains_identity(processes: &[PinnedLinuxProcess], captured: &TmuxProcessIdentity) -> bool {
    processes.iter().any(|process| {
        process.identity.pid == captured.pid && process.identity.start_time == captured.start_time
    })
}

#[cfg(target_os = "linux")]
pub(crate) fn pin_process_in_session(
    captured: &TmuxProcessIdentity,
    process_session_id: libc::pid_t,
) -> Result<PinnedLinuxProcess, SessionTerminationFailure> {
    if captured.pid <= 1 || captured.pid == unsafe { libc::getpid() } || captured.start_time == 0 {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    }
    let validate = || -> Result<_, SessionTerminationFailure> {
        Ok(
            crate::read_linux_process_identity(captured.pid)?.filter(|identity| {
                identity.pid == captured.pid
                    && identity.session_id == process_session_id
                    && identity.start_time == captured.start_time
                    && !identity.zombie
            }),
        )
    };
    validate()?.ok_or(SessionTerminationFailure::VerificationFailed)?;
    let raw_fd = unsafe { libc::syscall(libc::SYS_pidfd_open, captured.pid, 0) };
    if raw_fd < 0 {
        return Err(SessionTerminationFailure::VerificationFailed);
    }
    let pidfd = unsafe { OwnedFd::from_raw_fd(raw_fd as libc::c_int) };
    validate()?.ok_or(SessionTerminationFailure::VerificationFailed)?;
    crate::pidfd_send_signal(&pidfd, 0, false)?;
    Ok(PinnedLinuxProcess {
        identity: captured.clone(),
        pidfd,
    })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::{
        os::unix::process::CommandExt,
        process::{Child, Command, Stdio},
        time::Instant,
    };

    struct TestProcessSession(Child);

    impl Drop for TestProcessSession {
        fn drop(&mut self) {
            let pid = self.0.id() as libc::pid_t;
            // SAFETY: this test owns the isolated process group it created.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
            let _ = self.0.wait();
        }
    }

    fn split_session_fixture(
        root: &Path,
    ) -> (
        CliContext,
        SessionRecord,
        TestProcessSession,
        TmuxRuntimeIdentity,
    ) {
        let state_dir = root.join("state");
        let session_dir = state_dir.join("sessions/split-quiesce-fixture");
        fs::create_dir_all(&session_dir).unwrap();
        let context = CliContext {
            state_dir,
            host: None,
        };
        let runtime = RuntimeInfo {
            kind: "tmux".to_string(),
            tmux_session: "fixture-session".to_string(),
            generation: 1,
            started_at: "2026-01-01T00:00:00Z".to_string(),
            launch_id: "fixture-launch".to_string(),
            extra: BTreeMap::new(),
        };
        let record: SessionRecord = serde_json::from_value(json!({
            "schema_version": "agent-session.session.v1",
            "id": "split-quiesce-fixture",
            "agent": "codex",
            "mode": "interactive",
            "cwd": root.to_string_lossy(),
            "tmux_session": "fixture-session",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "runtime": runtime,
            "delete_tmux_termination_state": {
                "launch_id": "fixture-launch",
                "state": "pending",
                "thaw_on_recovery": false,
                "recover_split_cgroup": true
            }
        }))
        .unwrap();
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60 & wait", "fixture"]);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let process_session = TestProcessSession(command.spawn().unwrap());
        let session_id = process_session.0.id() as libc::pid_t;
        let deadline = Instant::now() + Duration::from_secs(2);
        let members = loop {
            let members = crate::linux_process_session_members(session_id).unwrap();
            if members.len() >= 2 {
                break members;
            }
            assert!(
                Instant::now() < deadline,
                "fixture did not spawn both members"
            );
            thread::sleep(Duration::from_millis(5));
        };
        let process_session_members = members
            .iter()
            .map(|member| TmuxProcessIdentity {
                pid: member.pid,
                start_time: member.start_time,
            })
            .collect::<Vec<_>>();
        let identity = TmuxRuntimeIdentity {
            macos_boot_id: None,
            launch_id: Some("fixture-launch".to_string()),
            session_id: "$1".to_string(),
            pane_id: "%1".to_string(),
            pane_pid: session_id,
            pane_start_time: Some(members[0].start_time),
            process_group_id: Some(session_id),
            process_session_id: Some(session_id),
            process_session_members,
            control_group_members: Vec::new(),
            control_group: Some(TmuxControlGroupIdentity {
                path: format!(
                    "/user.slice/user-1000.slice/user@1000.service/app.slice/tmux-spawn-{}.scope",
                    uuid::Uuid::new_v4()
                ),
                device: 0,
                inode: 0,
                boot_id: None,
            }),
            cgroup_mount: None,
            pid_namespace: None,
        };
        crate::write_session_document(&context, &record).unwrap();
        (context, record, process_session, identity)
    }

    fn pinned_session_members(identity: &TmuxRuntimeIdentity) -> Vec<PinnedLinuxProcess> {
        let process_session_id = identity.process_session_id.unwrap();
        identity
            .process_session_members
            .iter()
            .map(|member| pin_process_in_session(member, process_session_id).unwrap())
            .collect()
    }

    fn assert_process_stop_state(pid: libc::pid_t, stopped: bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let observed = linux_process_is_stopped(pid).unwrap_or(false);
            if observed == stopped {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "process {pid} stop state did not converge"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn persisted_resume_continues_after_earlier_stale_identity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, _process_session, identity) = split_session_fixture(tmp.path());
        let members = &identity.process_session_members;
        assert!(members.len() >= 2);
        let stale_actual = members[0].clone();
        let exact = members[1].clone();
        for member in [&stale_actual, &exact] {
            let pinned = pin_process_in_session(member, identity.process_session_id.unwrap())
                .expect("pin isolated fixture member");
            crate::pidfd_send_signal(&pinned.pidfd, libc::SIGSTOP, false).unwrap();
            wait_for_process_stop(member, identity.process_session_id.unwrap()).unwrap();
        }

        let mut stale = stale_actual.clone();
        stale.start_time += 1;
        persist_paused_split_scope_members(&context, &mut record, &[stale.clone(), exact.clone()])
            .unwrap();

        let result = resume_persisted_paused_members(&context, &mut record, &identity);

        assert!(result.is_err());
        assert_process_stop_state(stale_actual.pid, true);
        assert_process_stop_state(exact.pid, false);
        let unresolved: Vec<TmuxProcessIdentity> = serde_json::from_value(
            record.extra[DELETE_TMUX_TERMINATION_STATE_KEY]["split_scope_paused_members"].clone(),
        )
        .unwrap();
        assert_eq!(unresolved, vec![stale]);
        let persisted: SessionRecord = serde_json::from_slice(
            &fs::read(
                context
                    .state_dir
                    .join("sessions/split-quiesce-fixture/session.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            persisted.extra[DELETE_TMUX_TERMINATION_STATE_KEY]["split_scope_paused_members"],
            serde_json::to_value(&unresolved).unwrap()
        );
    }

    #[test]
    fn failed_stop_attempt_resumes_prior_persisted_members() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, _process_session, identity) = split_session_fixture(tmp.path());
        let members = &identity.process_session_members;
        assert!(members.len() >= 2);
        let newly_paused = members[0].clone();
        let previously_paused = members[1].clone();
        let prior =
            pin_process_in_session(&previously_paused, identity.process_session_id.unwrap())
                .expect("pin prior isolated fixture member");
        crate::pidfd_send_signal(&prior.pidfd, libc::SIGSTOP, false).unwrap();
        wait_for_process_stop(&previously_paused, identity.process_session_id.unwrap()).unwrap();
        persist_paused_split_scope_members(
            &context,
            &mut record,
            std::slice::from_ref(&previously_paused),
        )
        .unwrap();
        let pinned = pinned_session_members(&identity);

        let result =
            stop_pinned_session_members(&context, &mut record, &identity, &pinned[..1], members);

        assert!(result.is_err());
        assert_process_stop_state(newly_paused.pid, false);
        assert_process_stop_state(previously_paused.pid, false);
        assert!(
            record.extra[DELETE_TMUX_TERMINATION_STATE_KEY]
                .get("split_scope_paused_members")
                .is_none()
        );
    }

    #[test]
    fn changed_start_time_or_session_is_refused_without_signaling() {
        let mut command = Command::new("sleep");
        command.arg("30").stdin(Stdio::null()).stdout(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut child = command.spawn().expect("spawn identity target");
        let pid = child.id() as libc::pid_t;
        let identity = crate::read_linux_process_identity(pid)
            .expect("read target identity")
            .expect("target is live");
        let captured = TmuxProcessIdentity {
            pid,
            start_time: identity.start_time,
        };

        let mut stale_incarnation = captured.clone();
        stale_incarnation.start_time += 1;
        assert!(pin_process_in_session(&stale_incarnation, identity.session_id).is_err());
        assert!(pin_process_in_session(&captured, identity.session_id + 1).is_err());
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);

        child.kill().expect("stop fixture process");
        child.wait().expect("reap fixture process");
    }

    #[test]
    fn stop_guard_tracks_only_owned_pause_identities_and_resumes_on_drop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, _process_session, identity) = split_session_fixture(tmp.path());
        let pinned = pinned_session_members(&identity);
        let already_stopped = identity.process_session_members[0].clone();
        let first_pidfd =
            pin_process_in_session(&already_stopped, identity.process_session_id.unwrap()).unwrap();
        crate::pidfd_send_signal(&first_pidfd.pidfd, libc::SIGSTOP, false).unwrap();
        wait_for_process_stop(&already_stopped, identity.process_session_id.unwrap()).unwrap();

        let guard = stop_pinned_session_members(
            &context,
            &mut record,
            &identity,
            &pinned,
            &identity.process_session_members,
        )
        .unwrap();
        for member in &identity.process_session_members {
            assert!(linux_process_is_stopped(member.pid).unwrap());
        }
        let paused: Vec<TmuxProcessIdentity> = serde_json::from_value(
            record.extra[DELETE_TMUX_TERMINATION_STATE_KEY]["split_scope_paused_members"].clone(),
        )
        .unwrap();
        assert!(!paused.contains(&already_stopped));
        assert!(!paused.is_empty());

        drop(guard);
        for member in &identity.process_session_members {
            assert_process_stop_state(member.pid, member == &already_stopped);
        }

        resume_persisted_paused_members(&context, &mut record, &identity).unwrap();
        assert!(
            record.extra[DELETE_TMUX_TERMINATION_STATE_KEY]
                .get("split_scope_paused_members")
                .is_none()
        );
        assert!(linux_process_is_stopped(already_stopped.pid).unwrap());
        crate::pidfd_send_signal(&first_pidfd.pidfd, libc::SIGCONT, false).unwrap();
    }

    #[test]
    fn stop_guard_resumes_prior_members_when_a_later_pin_is_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, _process_session, identity) = split_session_fixture(tmp.path());
        let pinned = pinned_session_members(&identity);
        assert!(identity.process_session_members.len() >= 2);

        let error = stop_pinned_session_members(
            &context,
            &mut record,
            &identity,
            &pinned[..1],
            &identity.process_session_members,
        )
        .unwrap_err();
        assert_eq!(error, SessionTerminationFailure::VerificationFailed);
        for member in &identity.process_session_members {
            assert_process_stop_state(member.pid, false);
        }
        resume_persisted_paused_members(&context, &mut record, &identity).unwrap();
    }
}
