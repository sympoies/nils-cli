use super::*;

mod shared_daemons;

pub(super) fn capture_tmux_runtime_identity(
    context: &CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
    timeout: Duration,
) -> Result<TmuxRuntimeProbe, SessionTerminationFailure> {
    capture_tmux_identity(context, record, tmux_bin, timeout, true)
}

/// Capture the runtime identity, resolving a probe that lost a race with the
/// runtime's own exit.
///
/// The identity is read through several tmux and process probes. When the
/// session closes between them (the first `display-message` saw it), a later
/// probe fails with an unavailable or mismatched identity even though nothing
/// is left to identify. Re-check the exact session: a positively absent
/// session is a stopped runtime, not an identity that cannot be established.
fn capture_tmux_identity(
    context: &CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
    timeout: Duration,
    capture_process_boundary: bool,
) -> Result<TmuxRuntimeProbe, SessionTerminationFailure> {
    match capture_tmux_identity_once(context, record, tmux_bin, timeout, capture_process_boundary) {
        Err(
            SessionTerminationFailure::RuntimeIdentityUnavailable
            | SessionTerminationFailure::RuntimeIdentityMismatch,
        ) if verified_tmux_status_with_timeout(
            tmux_bin,
            &format!("={}", record.tmux_session),
            timeout,
        ) == "stopped" =>
        {
            Ok(TmuxRuntimeProbe::Stopped)
        }
        probe => probe,
    }
}

fn capture_tmux_identity_once(
    context: &CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
    timeout: Duration,
    capture_process_boundary: bool,
) -> Result<TmuxRuntimeProbe, SessionTerminationFailure> {
    #[cfg(test)]
    if let Some(probe) = tmux_probe_fixture::read(tmux_bin) {
        return probe;
    }
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .filter(|launch_id| !launch_id.is_empty())
        .map(ToOwned::to_owned);
    let mut command = ProcessCommand::new(tmux_bin);
    command
        .env("LC_ALL", "C")
        .arg("display-message")
        .arg("-p")
        .arg("-t")
        .arg(managed_tmux_pane_target(&record.tmux_session))
        .arg(TMUX_RUNTIME_IDENTITY_FORMAT);
    let output = run_output_with_timeout(command, timeout)
        .map_err(|_| SessionTerminationFailure::VerificationFailed)?;
    if !output.status.success() {
        return if tmux_output_reports_absent(&output) {
            Ok(TmuxRuntimeProbe::Stopped)
        } else {
            Err(SessionTerminationFailure::VerificationFailed)
        };
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|_| SessionTerminationFailure::RuntimeIdentityUnavailable)?;
    if output.trim().is_empty() {
        let exact_session_target = format!("={}", record.tmux_session);
        return match verified_tmux_status_with_timeout(tmux_bin, &exact_session_target, timeout)
            .as_str()
        {
            "stopped" => Ok(TmuxRuntimeProbe::Stopped),
            "running" => Err(SessionTerminationFailure::RuntimeIdentityUnavailable),
            _ => Err(SessionTerminationFailure::VerificationFailed),
        };
    }
    let (session_id, pane_id, pane_pid) = parse_tmux_runtime_identity_fields(&output)
        .ok_or(SessionTerminationFailure::RuntimeIdentityUnavailable)?;
    #[cfg(target_os = "linux")]
    let (pane_start_time_ticks, pane_pidfd) = pin_live_linux_process_incarnation(pane_pid)?;
    #[cfg(target_os = "linux")]
    let pane_start_time = Some(pane_start_time_ticks);
    #[cfg(not(target_os = "linux"))]
    let pane_start_time = None;

    let expected_state_dir = display_path(&context.state_dir);
    let expected = [
        ("AGENT_SESSION_ID", record.id.as_str()),
        ("AGENT_SESSION_STATE_DIR", expected_state_dir.as_str()),
    ];
    for (name, expected_value) in expected {
        let value = tmux_environment_value(tmux_bin, session_id, name, timeout)?;
        if value != expected_value {
            return Err(SessionTerminationFailure::RuntimeIdentityMismatch);
        }
    }
    if let Some(expected_launch_id) = launch_id.as_deref()
        && tmux_environment_value(tmux_bin, session_id, "AGENT_SESSION_RUNTIME_ID", timeout)?
            != expected_launch_id
    {
        return Err(SessionTerminationFailure::RuntimeIdentityMismatch);
    }
    let process_group_id = process_group_id(pane_pid)?;
    if process_group_id <= 1 || process_group_id == unsafe { libc::getpgrp() } {
        return Err(SessionTerminationFailure::RuntimeIdentityUnavailable);
    }
    let process_session_id = if capture_process_boundary {
        process_session_id(pane_pid)?
    } else {
        None
    };
    let process_session_members = if capture_process_boundary {
        process_session_members(process_session_id, pane_pid)?
    } else {
        Vec::new()
    };
    let control_group = if capture_process_boundary {
        linux_process_control_group(pane_pid)?
    } else {
        None
    };
    #[cfg(target_os = "linux")]
    let cgroup_mount = control_group
        .as_ref()
        .and_then(|_| capture_linux_cgroup_mount_identity());
    #[cfg(not(target_os = "linux"))]
    let cgroup_mount = None;
    let pid_namespace = if capture_process_boundary {
        linux_process_identity_namespace(pane_pid)?
    } else {
        None
    };

    if !tmux_pane_identity_matches(tmux_bin, record, session_id, pane_id, pane_pid, timeout) {
        return Err(SessionTerminationFailure::RuntimeIdentityMismatch);
    }
    #[cfg(target_os = "linux")]
    verify_pinned_linux_process_incarnation(pane_pid, pane_start_time_ticks, &pane_pidfd)?;
    Ok(TmuxRuntimeProbe::Running(Box::new(TmuxRuntimeIdentity {
        macos_boot_id: capture_macos_boot_id(),
        launch_id,
        session_id: session_id.to_string(),
        pane_id: pane_id.to_string(),
        pane_pid,
        pane_start_time,
        process_group_id: Some(process_group_id),
        process_session_id,
        process_session_members,
        control_group_members: Vec::new(),
        control_group,
        cgroup_mount,
        pid_namespace,
    })))
}

#[derive(Debug, Serialize)]
struct ProcessSummary {
    pid: libc::pid_t,
    /// Executable name only: arguments and environment can contain secrets.
    command: String,
}

#[derive(Default)]
struct ProcessScan {
    processes: Vec<ProcessSummary>,
    shared_daemons: Vec<shared_daemons::RetainedDaemon>,
    complete: bool,
    inspected: usize,
    uninspected: usize,
    reasons: std::collections::BTreeMap<&'static str, usize>,
}

#[derive(Debug, Default, Serialize)]
pub struct ProcessInventory {
    pub complete: bool,
    pub inspected_processes: usize,
    pub uninspected_processes: usize,
    pub reasons: std::collections::BTreeMap<&'static str, usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub retained_shared_daemons: Vec<shared_daemons::RetainedDaemon>,
}

impl ProcessScan {
    fn incomplete(&mut self, reason: &'static str, uninspected: usize) {
        self.complete = false;
        self.uninspected += uninspected;
        *self.reasons.entry(reason).or_default() += 1;
    }

    fn read_failed(&mut self, error: &io::Error) {
        self.incomplete(
            if error.kind() == io::ErrorKind::PermissionDenied {
                "permission-denied"
            } else {
                "process-read-failed"
            },
            1,
        );
    }

    fn inventory(&self) -> ProcessInventory {
        let mut reasons = self.reasons.clone();
        if !self.complete && reasons.is_empty() {
            reasons.insert("scan-unavailable", 1);
        }
        ProcessInventory {
            complete: self.complete,
            inspected_processes: self.inspected,
            uninspected_processes: self.uninspected,
            reasons,
            retained_shared_daemons: self.shared_daemons.clone(),
        }
    }
}

fn inventory_warning(inventory: &ProcessInventory) -> String {
    if inventory.complete {
        return String::new();
    }
    let reasons = inventory
        .reasons
        .iter()
        .map(|(reason, count)| format!("{reason}: {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "process inventory incomplete (inspected: {}, uninspected: {}; {}); additional processes may remain\n",
        inventory.inspected_processes, inventory.uninspected_processes, reasons
    )
}

pub(super) fn run_delete(context: &CliContext, args: cli::DeleteArgs) -> i32 {
    if args.if_stopped {
        return match crate::lifecycle::attempt(context, &args.id, "delete", || {
            delete_if_stopped(context, &args)
        }) {
            Ok(result) => {
                render_single_success(DELETE_COMMAND, args.format, &result, render_delete_text)
            }
            Err(error) => render_error(DELETE_COMMAND, args.format, error),
        };
    }
    if args.leave_processes {
        return match crate::lifecycle::attempt(context, &args.id, "delete", || {
            delete_leaving_processes(context, &args)
        }) {
            Ok(result) => render_single_success(DELETE_COMMAND, args.format, &result, |result| {
                let mut text = format!(
                    "deleted {} (tmux stopped; process termination not verified)\n",
                    result.deleted.id
                );
                for process in &result.survivors {
                    text.push_str(&format!(
                        "left running: {} {}\n",
                        process.pid, process.command
                    ));
                }
                text.push_str(&shared_daemons::render_retained(
                    &result.deleted.process_inventory,
                ));
                if !result.survivor_scan_complete {
                    text.push_str(&inventory_warning(&result.deleted.process_inventory));
                }
                for child in result
                    .deleted
                    .children
                    .iter()
                    .flat_map(|children| &children.orphaned)
                {
                    text.push_str(&format!("orphaned child: {}\n", child.session_id));
                }
                text
            }),
            Err(error) => render_error(DELETE_COMMAND, args.format, error),
        };
    }
    match delete_session_guarding_children(
        context,
        &board::machine_identity(None, context),
        &args.id,
        resolve_tmux_bin(args.tmux_bin.as_deref()),
        args.orphan_children,
    ) {
        Ok(result) => {
            render_single_success(DELETE_COMMAND, args.format, &result, render_delete_text)
        }
        Err(err) => render_error(DELETE_COMMAND, args.format, err),
    }
}

fn delete_if_stopped(
    context: &CliContext,
    args: &cli::DeleteArgs,
) -> Result<DeleteResult, CliError> {
    let machine = board::machine_identity(None, context);
    let children = lineage::guard_children(context, &machine, &args.id, args.orphan_children)?;
    let observed = load_session_record(context, &args.id)?;
    let _record_lock = acquire_session_record_lock(context, &observed.id)?;
    let record = load_session_record(context, &observed.id)?;
    ensure_same_session_identity(&observed, &record)?;
    let resolved = resolve_session_record_path(context, &record.id)?;

    let expected = args.expected_incarnation.as_deref().unwrap_or_default();
    let actual = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .filter(|incarnation| !incarnation.is_empty());
    if actual != Some(expected) {
        return Err(CliError::data(
            "session-incarnation-conflict",
            "session runtime changed before conditional delete",
            Some(json!({
                "id": record.id,
                "expected_session_incarnation": expected,
                "actual_session_incarnation": actual,
            })),
        ));
    }

    match session_status(
        context,
        &resolve_tmux_bin(args.tmux_bin.as_deref()),
        &record,
    )
    .as_str()
    {
        "stopped" => {}
        "running" => return Err(conditional_state_mismatch(&record, "running")),
        _ => return Err(conditional_status_unknown(&record)),
    }
    ensure_session_deletion_supported(&record)?;
    if persisted_tmux_termination_state(&record)
        .map_err(|_| conditional_status_unknown(&record))?
        .is_some()
    {
        return Err(CliError::runtime(
            "session-termination-in-progress",
            "conditional deletion must recover interrupted termination before removing session state",
            Some(json!({"id": record.id, "action": "retry-delete", "retryable": true})),
        ));
    }
    let prior = persisted_prior_tmux_runtime_identities(&record)
        .map_err(|_| conditional_status_unknown(&record))?;
    match persisted_tmux_runtime_identity(&record)
        .map_err(|_| conditional_status_unknown(&record))?
    {
        Some(identity) => match verify_stopped_tmux_runtime(
            &resolve_tmux_bin(args.tmux_bin.as_deref()),
            &record.tmux_session,
            &identity,
            DELETE_TERMINATION_VERIFY_TIMEOUT,
        ) {
            Ok(()) => {}
            Err(SessionTerminationFailure::StillRunning) => {
                return Err(conditional_state_mismatch(&record, "running"));
            }
            Err(SessionTerminationFailure::ProcessStillRunning) => {
                return Err(conditional_state_mismatch(&record, "working"));
            }
            Err(_) => return Err(conditional_status_unknown(&record)),
        },
        None if runtime_is_proven_never_launched(&record) && prior.is_empty() => {}
        None => return Err(conditional_status_unknown(&record)),
    }
    if verify_stopped_process_runtimes(&prior, DELETE_TERMINATION_VERIFY_TIMEOUT).is_err() {
        return Err(conditional_status_unknown(&record));
    }
    let scan = associated_processes(&record);
    if !scan.processes.is_empty() {
        return Err(conditional_state_mismatch(&record, "working"));
    }

    if args.if_unread_empty {
        coordination::delete::ensure_unread_empty(context, &record.id)?;
    }
    let registry_fence = SessionRegistryFence::from_record(&record);
    let mut deleted = finish_session_delete(
        context,
        record,
        resolved.session_dir,
        registry_fence,
        board::CloseReason::Deleted,
    )?;
    deleted.killed = false;
    deleted.termination_scope = "none";
    deleted.children = Some(children);
    Ok(deleted)
}

fn conditional_state_mismatch(record: &SessionRecord, state: &str) -> CliError {
    CliError::data(
        "session-state-mismatch",
        "session is not stopped and quiescent",
        Some(json!({ "id": record.id, "state": state })),
    )
}

fn conditional_status_unknown(record: &SessionRecord) -> CliError {
    CliError::runtime(
        "session-status-unknown",
        "session stopped state could not be verified",
        Some(json!({ "id": record.id, "state": "unknown" })),
    )
}

#[derive(Serialize)]
struct RecoveryDeleteResult {
    #[serde(flatten)]
    deleted: DeleteResult,
    survivors: Vec<ProcessSummary>,
    survivor_scan_complete: bool,
}

/// Explicitly relinquish management of processes, without broadening signaling
/// authority. This path never signals a PID or a process group.
fn delete_leaving_processes(
    context: &CliContext,
    args: &cli::DeleteArgs,
) -> Result<RecoveryDeleteResult, CliError> {
    let children = lineage::guard_children(
        context,
        &board::machine_identity(None, context),
        &args.id,
        args.orphan_children,
    )?;
    let observed = load_session_record(context, &args.id)?;
    let _lock = acquire_session_record_lock(context, &observed.id)?;
    let resolved = resolve_session_record_path(context, &observed.id)?;
    let record = read_session_record(&resolved.record_path)?;
    ensure_same_session_identity(&observed, &record)?;
    validate_record_id(&record, &resolved.expected_id, &resolved.record_path)?;
    ensure_session_deletion_supported(&record)?;
    ensure_session_lifecycle_mutation_allowed(context, &record)?;
    let tmux = resolve_tmux_bin(args.tmux_bin.as_deref());
    let error =
        |reason| session_termination_error(&record, reason, SessionTerminationOperation::Delete);
    // Do not abandon a frozen or partially killed transaction. Its durable
    // recovery still owns those processes and its thaw obligations.
    if persisted_tmux_termination_state(&record)
        .map_err(error)?
        .is_some()
    {
        return Err(CliError::runtime(
            "session-termination-in-progress",
            "ordinary deletion must recover the interrupted termination before leaving processes",
            Some(json!({"id": record.id, "action": "retry-delete", "retryable": true})),
        ));
    }
    match capture_tmux_identity(
        context,
        &record,
        &tmux,
        DELETE_TERMINATION_PROBE_TIMEOUT,
        false,
    )
    .map_err(error)?
    {
        TmuxRuntimeProbe::Running(identity) => {
            if persisted_tmux_runtime_identity(&record)
                .map_err(error)?
                .is_some_and(|old| !old.same_runtime_target(&identity))
            {
                return Err(error(SessionTerminationFailure::RuntimeIdentityMismatch));
            }
            match tmux_kill_identity_with_timeout(&tmux, &identity, PANE_INPUT_COMMAND_TIMEOUT)
                .map_err(error)?
            {
                TmuxIdentityKillOutcome::KillConfirmed => {}
                TmuxIdentityKillOutcome::IdentityChanged => {
                    return Err(error(SessionTerminationFailure::RuntimeIdentityChanged));
                }
            }
            // A reused managed name and the captured numeric target must both
            // be absent. Never kill a replacement target on this recovery path.
            verify_tmux_target_stopped(&tmux, &identity, DELETE_TERMINATION_VERIFY_TIMEOUT)
                .map_err(error)?;
        }
        TmuxRuntimeProbe::Stopped => {}
    }
    let target = format!("={}", record.tmux_session);
    if verified_tmux_status_with_timeout(&tmux, &target, DELETE_TERMINATION_PROBE_TIMEOUT)
        != "stopped"
    {
        return Err(error(SessionTerminationFailure::TmuxStatusUnavailable));
    }
    let scan = associated_processes(&record);
    let inventory = scan.inventory();
    let fence = SessionRegistryFence::from_record(&record);
    let mut deleted = commit_session_delete(
        context,
        record,
        resolved.session_dir,
        fence,
        board::CloseReason::Deleted,
        inventory,
    )?;
    // Compatibility field: ordinary deletion uses true only after verifying
    // every retained boundary. Explicit recovery cannot make that claim.
    deleted.killed = false;
    deleted.termination_scope = "tmux-only";
    deleted.children = Some(children);
    Ok(RecoveryDeleteResult {
        deleted,
        survivors: scan.processes,
        survivor_scan_complete: scan.complete,
    })
}

pub(super) fn finish_session_delete(
    context: &CliContext,
    record: SessionRecord,
    session_dir: PathBuf,
    registry_fence: SessionRegistryFence,
    close_reason: board::CloseReason,
) -> Result<DeleteResult, CliError> {
    ensure_session_deletion_supported(&record)?;
    let scan = associated_processes(&record);
    finish_session_delete_with_scan(
        context,
        record,
        session_dir,
        registry_fence,
        close_reason,
        scan,
    )
}

fn finish_session_delete_with_scan(
    context: &CliContext,
    record: SessionRecord,
    session_dir: PathBuf,
    registry_fence: SessionRegistryFence,
    close_reason: board::CloseReason,
    scan: ProcessScan,
) -> Result<DeleteResult, CliError> {
    if !scan.processes.is_empty() {
        return Err(termination_error_with_scan(
            &record,
            SessionTerminationFailure::ProcessStillRunning,
            SessionTerminationOperation::Delete,
            scan,
        ));
    }
    commit_session_delete(
        context,
        record,
        session_dir,
        registry_fence,
        close_reason,
        scan.inventory(),
    )
}

fn commit_session_delete(
    context: &CliContext,
    record: SessionRecord,
    session_dir: PathBuf,
    registry_fence: SessionRegistryFence,
    close_reason: board::CloseReason,
    process_inventory: ProcessInventory,
) -> Result<DeleteResult, CliError> {
    ensure_session_deletion_supported(&record)?;
    ensure_session_lifecycle_mutation_allowed(context, &record)
        .map_err(|error| contention::close_error(error, close_reason))?;
    // Built while the record's state is readable; appended to the closed
    // ledger only after the removal commits, and never failing it.
    let closed = board::closed_record(context, &record, close_reason);
    coordination::revoke(context, &record)
        .map_err(|error| contention::close_error(error, close_reason))?;
    codex_app_server::cleanup_runtime_files(context, &record)?;
    let cleanup_pending = commit_session_directory_delete(context, &record.id, &session_dir)?;
    board::record_close(context, closed);
    Ok(DeleteResult {
        children: None,
        id: record.id,
        tmux_session: record.tmux_session,
        killed: true,
        deleted: true,
        session_dir: display_path(&session_dir),
        cleanup_pending,
        registry_fence,
        process_inventory,
        termination_scope: "recorded-boundaries",
    })
}

pub(super) fn session_termination_error(
    record: &SessionRecord,
    reason: SessionTerminationFailure,
    operation: SessionTerminationOperation,
) -> CliError {
    let scan = if operation == SessionTerminationOperation::Delete {
        associated_processes(record)
    } else {
        ProcessScan::default()
    };
    termination_error_with_scan(record, reason, operation, scan)
}

fn termination_error_with_scan(
    record: &SessionRecord,
    reason: SessionTerminationFailure,
    operation: SessionTerminationOperation,
    scan: ProcessScan,
) -> CliError {
    let mut details = json!({
        "id": record.id, "tmux_session": record.tmux_session,
        "reason": reason.reason(), "retryable": reason.retryable(),
        "action": reason.action(operation),
    });
    if operation == SessionTerminationOperation::Delete
        && (!scan.processes.is_empty() || !scan.shared_daemons.is_empty() || !scan.complete)
    {
        details["blocking_processes"] = json!(scan.processes);
        details["process_scan_complete"] = json!(scan.complete);
        details["process_inventory"] = json!(scan.inventory());
        // Ownership/identity errors still require their original remedy.
        if !scan.processes.is_empty()
            && matches!(
                reason,
                SessionTerminationFailure::VerificationFailed
                    | SessionTerminationFailure::ProcessStillRunning
                    | SessionTerminationFailure::ProcessIdentityUnavailable
                    | SessionTerminationFailure::RuntimeIdentityUnavailable
                    | SessionTerminationFailure::ProcessStatusUnavailable
            )
        {
            details["retryable"] = json!(false);
            details["action"] = json!("inspect-processes-or-delete-with-leave-processes");
        }
    }
    let error = CliError::runtime(
        "session-termination-failed",
        format!("{}; session metadata was retained", reason.message()),
        Some(details),
    );
    if operation != SessionTerminationOperation::Delete {
        return error;
    }
    let mut hint = scan
        .processes
        .iter()
        .map(|process| format!("blocking process: {} {}\n", process.pid, process.command))
        .collect::<String>();
    hint.push_str(&shared_daemons::render_retained(&scan.inventory()));
    hint.push_str(&inventory_warning(&scan.inventory()));
    if !scan.processes.is_empty() {
        hint.push_str("inspect associated processes or use delete --leave-processes\n");
    }
    if hint.is_empty() {
        error
    } else {
        error.with_hint(hint.trim_end())
    }
}

fn associated_processes(record: &SessionRecord) -> ProcessScan {
    let Some(runtime) = record
        .runtime
        .as_ref()
        .filter(|runtime| !runtime.launch_id.is_empty())
    else {
        return ProcessScan::default();
    };
    scan_processes(&record.id, &runtime.launch_id)
}

fn matches_runtime(bytes: &[u8], id: &str, runtime: &str) -> bool {
    let id = format!("AGENT_SESSION_ID={id}");
    let runtime = format!("AGENT_SESSION_RUNTIME_ID={runtime}");
    bytes
        .split(|byte| *byte == 0)
        .any(|entry| entry == id.as_bytes())
        && bytes
            .split(|byte| *byte == 0)
            .any(|entry| entry == runtime.as_bytes())
}

fn command_name(command: &str) -> String {
    Path::new(command.trim())
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .take(80)
        .map(|ch| if ch.is_control() { '?' } else { ch })
        .collect()
}

#[cfg(target_os = "linux")]
fn scan_processes(id: &str, runtime: &str) -> ProcessScan {
    scan_linux_processes(id, runtime, Path::new("/proc"), read_linux_process_identity)
}

#[cfg(target_os = "linux")]
fn scan_linux_processes(
    id: &str,
    runtime: &str,
    root: &Path,
    mut read_identity: impl FnMut(
        libc::pid_t,
    )
        -> Result<Option<LinuxProcessIdentity>, SessionTerminationFailure>,
) -> ProcessScan {
    let mut scan = ProcessScan {
        complete: true,
        ..Default::default()
    };
    let Ok(entries) = fs::read_dir(root) else {
        return ProcessScan::default();
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    for entry in entries {
        if Instant::now() >= deadline {
            scan.incomplete("time-budget-exhausted", 0);
            break;
        }
        if scan.processes.len() + scan.shared_daemons.len() >= 128 {
            scan.incomplete("entry-limit-reached", 0);
            break;
        }
        let Ok(entry) = entry else {
            scan.incomplete("process-list-failed", 0);
            continue;
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        if pid <= 1 || pid == unsafe { libc::getpid() } {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.uid() != unsafe { libc::geteuid() } {
            continue;
        }
        let before = match read_identity(pid) {
            Ok(Some(identity)) if !identity.zombie => identity,
            Ok(_) => continue,
            Err(_) => {
                scan.incomplete("process-identity-unavailable", 1);
                continue;
            }
        };
        let bytes = match fs::read(entry.path().join("environ")) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                scan.read_failed(&error);
                continue;
            }
        };
        scan.inspected += 1;
        if !matches_runtime(&bytes, id, runtime) {
            continue;
        }
        let command =
            fs::read_to_string(entry.path().join("comm")).unwrap_or_else(|_| "unknown".into());
        let shared_daemon = shared_daemons::linux_shared_daemon(&entry.path());
        // Recheck incarnation after reading diagnostics; reused PIDs must not
        // acquire a former process's runtime marker or command.
        if matches!(read_identity(pid), Ok(Some(after)) if after.start_time == before.start_time && !after.zombie)
        {
            if let Some(command) = shared_daemon {
                scan.shared_daemons.push(shared_daemons::RetainedDaemon {
                    pid,
                    command,
                    reason: "shared-daemon-retained",
                });
            } else {
                scan.processes.push(ProcessSummary {
                    pid,
                    command: command_name(&command),
                });
            }
        } else {
            scan.incomplete("process-changed", 0);
        }
    }
    scan.processes.sort_by_key(|process| process.pid);
    scan.shared_daemons.sort_by_key(|process| process.pid);
    scan
}

#[cfg(target_os = "macos")]
fn scan_processes(id: &str, runtime: &str) -> ProcessScan {
    let Some(argument_limit) = macos_argument_limit() else {
        return ProcessScan::default();
    };
    let mut command = ProcessCommand::new("/bin/ps");
    command.args(["-axo", "pid=,uid=,stat=,comm="]);
    let output = match macos_process_listing(command) {
        Ok(output) => output,
        Err(error) => return failed_macos_process_listing(&error),
    };
    if !output.status.success() {
        return failed_macos_process_listing(&io::Error::other("process listing failed"));
    }
    scan_macos_processes(id, runtime, &output.stdout, |pid| {
        macos_process_arguments(pid, argument_limit)
    })
}

#[cfg(any(target_os = "macos", test))]
fn macos_process_listing(command: ProcessCommand) -> io::Result<std::process::Output> {
    // The 4 KiB tmux-probe cap silently omits later PIDs from a full ps table.
    // Keep inventory separately bounded and never accept a truncated prefix.
    run_output_with_timeout_and_strict_cap(command, Duration::from_secs(1), 1024 * 1024)
}

#[cfg(any(target_os = "macos", test))]
fn failed_macos_process_listing(error: &io::Error) -> ProcessScan {
    let mut scan = ProcessScan::default();
    scan.incomplete(
        if error.kind() == io::ErrorKind::InvalidData {
            "process-list-output-limit"
        } else {
            "process-list-failed"
        },
        0,
    );
    scan
}

#[cfg(any(target_os = "macos", test))]
fn scan_macos_processes(
    id: &str,
    runtime: &str,
    listing: &[u8],
    mut read_arguments: impl FnMut(libc::pid_t) -> io::Result<Vec<u8>>,
) -> ProcessScan {
    let mut scan = ProcessScan {
        complete: true,
        ..Default::default()
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    for line in String::from_utf8_lossy(listing).lines() {
        if Instant::now() >= deadline {
            scan.incomplete("time-budget-exhausted", 0);
            break;
        }
        let mut fields = line.split_whitespace();
        let Some(pid) = fields
            .next()
            .and_then(|pid| pid.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        if fields.next().and_then(|uid| uid.parse::<u32>().ok()) != Some(unsafe { libc::geteuid() })
            || pid <= 1
            || pid == unsafe { libc::getpid() }
        {
            continue;
        }
        if fields.next().is_none_or(|state| state.starts_with('Z')) {
            continue;
        }
        let bytes = match read_arguments(pid) {
            Ok(bytes) => bytes,
            Err(error) => {
                scan.read_failed(&error);
                continue;
            }
        };
        classify_macos_environment(
            &mut scan,
            pid,
            &fields.collect::<Vec<_>>().join(" "),
            &bytes,
            id,
            runtime,
        );
        if scan.processes.len() + scan.shared_daemons.len() >= 128 {
            scan.incomplete("entry-limit-reached", 0);
            break;
        }
    }
    scan.processes.sort_by_key(|process| process.pid);
    scan
}

#[cfg(any(target_os = "macos", test))]
fn classify_macos_environment(
    scan: &mut ProcessScan,
    pid: libc::pid_t,
    command: &str,
    bytes: &[u8],
    id: &str,
    runtime: &str,
) {
    match macos_environment(bytes) {
        Some(environment) => {
            scan.inspected += 1;
            if matches_runtime(environment, id, runtime) {
                scan.processes.push(ProcessSummary {
                    pid,
                    command: command_name(command),
                });
            }
        }
        // A successful argv read can still omit the environment under SIP.
        None => scan.incomplete("argv-only-or-sip-redacted", 1),
    }
}

#[cfg(target_os = "macos")]
fn macos_process_arguments(pid: libc::pid_t, argument_limit: usize) -> io::Result<Vec<u8>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    // Darwin rejects a procargs buffer larger than KERN_ARGMAX, even
    // when the process arguments would fit in that buffer.
    let mut bytes = vec![0u8; argument_limit];
    let mut length = bytes.len();
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    bytes.truncate(length);
    Ok(bytes)
}

#[cfg(target_os = "macos")]
fn macos_argument_limit() -> Option<usize> {
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let mut limit: libc::c_int = 0;
    let mut length = std::mem::size_of_val(&limit);
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut limit as *mut libc::c_int).cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0 && length == std::mem::size_of_val(&limit))
        .then(|| usize::try_from(limit).ok())
        .flatten()
        .filter(|limit| *limit > 4 && *limit <= 1024 * 1024)
}

#[cfg(any(target_os = "macos", test))]
fn macos_environment(bytes: &[u8]) -> Option<&[u8]> {
    let argc = i32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?);
    if argc < 0 {
        return None;
    }
    let mut offset = 4;
    offset += bytes.get(offset..)?.iter().position(|byte| *byte == 0)? + 1;
    while bytes.get(offset) == Some(&0) {
        offset += 1;
    }
    for _ in 0..argc {
        offset += bytes.get(offset..)?.iter().position(|byte| *byte == 0)? + 1;
    }
    bytes.get(offset..).filter(|environment| {
        environment
            .split(|byte| *byte == 0)
            .any(|entry| entry.contains(&b'='))
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn scan_processes(_: &str, _: &str) -> ProcessScan {
    ProcessScan::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// A tmux whose `display-message` still reports a live pane but whose later
    /// probes find nothing, with `has-session` answering as given.
    fn closing_session_tmux(dir: &Path, pane_pid: u32, session_present: bool) -> PathBuf {
        let tmux = dir.join("tmux");
        let has_session = if session_present {
            "exit 0"
        } else {
            "echo \"can't find session: $3\" >&2; exit 1"
        };
        fs::write(
            &tmux,
            format!(
                "#!/bin/sh\ncase \"$1\" in\ndisplay-message) printf '$91 %%91 {pane_pid}\\n'; exit 0 ;;\nhas-session) {has_session} ;;\nesac\nexit 1\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        tmux
    }

    #[test]
    fn identity_capture_reports_a_session_that_closed_mid_probe_as_stopped() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = crate::tests::test_context(tmp.path());
        let id = crate::tests::create_test_record_id(&context, AgentKind::Codex, None, None);
        let record = load_session_record(&context, &id).unwrap();
        let pane = crate::tests::TestProcessGroup::spawn();

        // `show-environment` fails after `display-message` saw the pane. The
        // session is now absent, so nothing remains whose identity is unknown.
        let tmux = closing_session_tmux(tmp.path(), pane.pid(), false);
        assert!(matches!(
            crate::capture_tmux_runtime_identity(&context, &record, &tmux, Duration::from_secs(1)),
            Ok(crate::TmuxRuntimeProbe::Stopped)
        ));

        // A session that is still present keeps the failure closed.
        let tmux = closing_session_tmux(tmp.path(), pane.pid(), true);
        assert_eq!(
            crate::capture_tmux_runtime_identity(&context, &record, &tmux, Duration::from_secs(1))
                .err(),
            Some(crate::SessionTerminationFailure::RuntimeIdentityUnavailable)
        );
    }

    #[test]
    fn plain_kill_accepts_a_session_that_closed_before_the_guarded_command() {
        let tmp = tempfile::TempDir::new().unwrap();
        let identity = crate::TmuxRuntimeIdentity {
            macos_boot_id: crate::capture_macos_boot_id(),
            launch_id: None,
            session_id: "$91".to_owned(),
            pane_id: "%91".to_owned(),
            pane_pid: 4242,
            pane_start_time: None,
            process_group_id: Some(4242),
            process_session_id: None,
            process_session_members: Vec::new(),
            control_group_members: Vec::new(),
            control_group: None,
            cgroup_mount: None,
            pid_namespace: None,
        };

        // `if-shell` fails because tmux no longer finds the pane to guard.
        let tmux = closing_session_tmux(tmp.path(), 4242, false);
        assert_eq!(
            crate::tmux_kill_identity_with_timeout(&tmux, &identity, Duration::from_secs(1)).ok(),
            Some(crate::TmuxIdentityKillOutcome::KillConfirmed)
        );

        // A session that is still present is a real kill failure.
        let tmux = closing_session_tmux(tmp.path(), 4242, true);
        assert_eq!(
            crate::tmux_kill_identity_with_timeout(&tmux, &identity, Duration::from_secs(1)).err(),
            Some(crate::SessionTerminationFailure::KillFailed)
        );
    }

    #[test]
    fn macos_process_listing_finds_marked_candidates_after_four_kib() {
        let uid = unsafe { libc::geteuid() };
        let mut listing = String::new();
        for pid in 100..300 {
            listing.push_str(&format!("{pid} {uid} S /bin/ordinary-process\n"));
        }
        let survivor_pid = 999_999;
        assert!(listing.len() > 4096);
        listing.push_str(&format!("{survivor_pid} {uid} S /bin/detached-provider\n"));
        let mut command = ProcessCommand::new("/bin/sh");
        command.args(["-c", &format!("printf '%s' '{listing}'")]);
        let output = macos_process_listing(command).unwrap();
        let scan = scan_macos_processes("session", "runtime", &output.stdout, |pid| {
            let mut arguments = 1i32.to_ne_bytes().to_vec();
            arguments.extend_from_slice(b"/bin/provider\0\0provider\0");
            arguments.extend_from_slice(if pid == survivor_pid {
                b"AGENT_SESSION_ID=session\0AGENT_SESSION_RUNTIME_ID=runtime\0"
            } else {
                b"OTHER=value\0"
            });
            Ok(arguments)
        });
        assert!(scan.complete);
        assert_eq!(scan.processes.len(), 1);
        assert_eq!(scan.processes[0].pid, survivor_pid);
        assert_eq!(scan.processes[0].command, "detached-provider");
    }

    #[test]
    fn macos_process_listing_rejects_overflow_instead_of_returning_a_prefix() {
        let mut command = ProcessCommand::new("/bin/sh");
        command.args(["-c", "printf '%1048577s' ''"]);
        let error = macos_process_listing(command).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let scan = failed_macos_process_listing(&error);
        assert!(!scan.complete);
        assert!(scan.processes.is_empty());
        assert_eq!(scan.inventory().reasons["process-list-output-limit"], 1);
    }

    // A locally built child keeps its environment inspectable under macOS
    // SIP; platform executables may intentionally hide theirs from sysctl.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_marker_child() {
        if std::env::var_os("NILS_TEST_MARKER_CHILD").is_some() {
            thread::sleep(Duration::from_secs(30));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_platform_child_reports_redaction_as_incomplete() {
        struct PlatformChild(std::process::Child);
        impl Drop for PlatformChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let runtime = uuid::Uuid::new_v4().to_string();
        let child = PlatformChild(
            ProcessCommand::new("/bin/sleep")
                .arg("30")
                .env("AGENT_SESSION_ID", "platform-marker")
                .env("AGENT_SESSION_RUNTIME_ID", &runtime)
                .spawn()
                .unwrap(),
        );
        let mut scan = ProcessScan {
            complete: true,
            ..Default::default()
        };
        let bytes = match macos_process_arguments(
            child.0.id() as libc::pid_t,
            macos_argument_limit().unwrap(),
        ) {
            Ok(bytes) => bytes,
            Err(error) => {
                scan.read_failed(&error);
                assert!(!scan.complete);
                assert_eq!(scan.inventory().uninspected_processes, 1);
                assert_eq!(
                    scan.inventory().reasons[if error.kind() == io::ErrorKind::PermissionDenied {
                        "permission-denied"
                    } else {
                        "process-read-failed"
                    }],
                    1
                );
                assert!(scan.processes.is_empty());
                return;
            }
        };
        let redacted = macos_environment(&bytes).is_none();
        classify_macos_environment(
            &mut scan,
            child.0.id() as libc::pid_t,
            "/bin/sleep",
            &bytes,
            "platform-marker",
            &runtime,
        );
        if redacted {
            assert!(!scan.complete);
            assert_eq!(scan.inventory().uninspected_processes, 1);
            assert_eq!(scan.inventory().reasons["argv-only-or-sip-redacted"], 1);
            assert!(scan.processes.is_empty());
        } else {
            // Hosts with unrestricted inspection must detect the live marker.
            assert!(scan.complete);
            assert_eq!(scan.processes.len(), 1);
            assert_eq!(scan.processes[0].pid, child.0.id() as libc::pid_t);
            assert_eq!(scan.processes[0].command, "sleep");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn delete_scan_rejects_reused_pid_instead_of_reporting_stale_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let pid = 41;
        let process = tmp.path().join(pid.to_string());
        fs::create_dir(&process).unwrap();
        fs::write(
            process.join("environ"),
            b"AGENT_SESSION_ID=session\0AGENT_SESSION_RUNTIME_ID=runtime\0",
        )
        .unwrap();
        fs::write(process.join("comm"), "provider\n").unwrap();
        for replacement_start in [100, 101] {
            let mut reads = 0;
            let scan = scan_linux_processes("session", "runtime", tmp.path(), |observed| {
                assert_eq!(observed, pid);
                reads += 1;
                Ok(Some(LinuxProcessIdentity {
                    pid,
                    process_group_id: pid,
                    session_id: pid,
                    start_time: if reads == 1 { 100 } else { replacement_start },
                    zombie: false,
                }))
            });
            assert_eq!(reads, 2, "identity must be checked after reading markers");
            if replacement_start == 100 {
                assert_eq!(scan.processes.len(), 1);
                assert_eq!(scan.processes[0].pid, pid);
                assert_eq!(scan.processes[0].command, "provider");
            } else {
                assert!(
                    scan.processes.is_empty(),
                    "reused PID must not inherit old markers or command"
                );
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn cache_daemon_scan(commands: &[(&str, u32, &[u8])]) -> ProcessScan {
        let tmp = tempfile::tempdir().unwrap();
        for (index, (executable, parent, arguments)) in commands.iter().enumerate() {
            let pid = 41000 + index;
            let process = tmp.path().join(pid.to_string());
            fs::create_dir(&process).unwrap();
            fs::write(
                process.join("environ"),
                b"AGENT_SESSION_ID=session\0AGENT_SESSION_RUNTIME_ID=runtime\0",
            )
            .unwrap();
            fs::write(process.join("comm"), "sccache\n").unwrap();
            fs::write(
                process.join("stat"),
                format!("{pid} (sccache) S {parent} 0 0"),
            )
            .unwrap();
            fs::write(process.join("cmdline"), arguments).unwrap();
            std::os::unix::fs::symlink(executable, process.join("exe")).unwrap();
        }
        scan_linux_processes("session", "runtime", tmp.path(), |pid| {
            Ok(Some(LinuxProcessIdentity {
                pid,
                process_group_id: pid,
                session_id: pid,
                start_time: 100,
                zombie: false,
            }))
        })
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shared_cache_daemon_allows_delete_and_reports_retention() {
        let scan = cache_daemon_scan(&[("/opt/bin/sccache", 1, b"/opt/bin/sccache\0")]);
        assert!(
            scan.processes.is_empty(),
            "shared daemon must not block deletion"
        );
        let (_tmp, context, record) = session_fixture();
        let directory = session_dir(&context, &record.id);
        let result = finish_session_delete_with_scan(
            &context,
            record.clone(),
            directory.clone(),
            SessionRegistryFence::from_record(&record),
            board::CloseReason::Deleted,
            scan,
        )
        .unwrap();
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(
            value["process_inventory"]["retained_shared_daemons"][0],
            json!({"pid": 41000, "command": "sccache", "reason": "shared-daemon-retained"})
        );
        assert!(render_delete_text(&result).contains("shared-daemon-retained: 41000 sccache"));
        assert!(!directory.exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn shared_cache_daemon_does_not_hide_other_survivors() {
        for candidate in [
            ("/opt/bin/worker", 1, b"worker\0".as_slice()),
            ("/opt/bin/sccache", 42, b"sccache\0".as_slice()),
            ("/opt/bin/sccache", 1, b"sccache\0rustc\0".as_slice()),
        ] {
            let scan = cache_daemon_scan(&[("/opt/bin/sccache", 1, b"sccache\0"), candidate]);
            assert_eq!(scan.processes.len(), 1);
            assert_eq!(scan.processes[0].pid, 41001);
            let (_tmp, context, record) = session_fixture();
            let directory = session_dir(&context, &record.id);
            let error = finish_session_delete_with_scan(
                &context,
                record.clone(),
                directory.clone(),
                SessionRegistryFence::from_record(&record),
                board::CloseReason::Deleted,
                scan,
            )
            .unwrap_err();
            assert!(error.0.message.contains("surviving processes"));
            let details = error.0.details.unwrap();
            assert_eq!(details["blocking_processes"].as_array().unwrap().len(), 1);
            assert_eq!(
                details["process_inventory"]["retained_shared_daemons"][0]["reason"],
                "shared-daemon-retained"
            );
            assert!(directory.join("session.json").exists());
        }
    }

    #[test]
    fn runtime_markers_require_exact_values_and_both_names() {
        assert!(matches_runtime(
            b"AGENT_SESSION_ID=session\0AGENT_SESSION_RUNTIME_ID=runtime\0",
            "session",
            "runtime"
        ));
        assert!(!matches_runtime(
            b"AGENT_SESSION_ID=session\0AGENT_SESSION_RUNTIME_ID=runtime-other\0",
            "session",
            "runtime"
        ));
        assert!(!matches_runtime(
            b"AGENT_SESSION_RUNTIME_ID=runtime\0",
            "session",
            "runtime"
        ));
        assert_eq!(command_name("/usr/bin/provider\n"), "provider");
    }

    #[test]
    fn macos_procargs_skips_executable_padding_and_all_arguments() {
        let mut bytes = 2i32.to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"/bin/provider\0\0\0provider\0AGENT_SESSION_ID=argument\0AGENT_SESSION_ID=session\0AGENT_SESSION_RUNTIME_ID=runtime\0");
        assert_eq!(
            macos_environment(&bytes).unwrap(),
            b"AGENT_SESSION_ID=session\0AGENT_SESSION_RUNTIME_ID=runtime\0"
        );
        assert!(matches_runtime(
            macos_environment(&bytes).unwrap(),
            "session",
            "runtime"
        ));
        assert!(!matches_runtime(
            macos_environment(&bytes).unwrap(),
            "argument",
            "runtime"
        ));
        assert!(macos_environment(&[0, 0]).is_none());
        let mut omitted = 1i32.to_ne_bytes().to_vec();
        omitted.extend_from_slice(b"/bin/provider\0\0provider\0");
        assert!(macos_environment(&omitted).is_none());
    }

    #[test]
    fn macos_process_inventory_distinguishes_redacted_and_readable_environments() {
        for environment in [
            None,
            Some("OTHER=value\0"),
            Some("AGENT_SESSION_ID=session\0AGENT_SESSION_RUNTIME_ID=runtime\0"),
        ] {
            let mut bytes = 1i32.to_ne_bytes().to_vec();
            bytes.extend_from_slice(b"/bin/provider\0\0provider\0");
            if let Some(environment) = environment {
                bytes.extend_from_slice(environment.as_bytes());
            }
            let mut scan = ProcessScan {
                complete: true,
                ..Default::default()
            };
            classify_macos_environment(
                &mut scan,
                41,
                "/bin/provider",
                &bytes,
                "session",
                "runtime",
            );
            let inventory = scan.inventory();
            if let Some(environment) = environment {
                assert_eq!(inventory.complete, true);
                assert_eq!(inventory.inspected_processes, 1);
                assert_eq!(inventory.uninspected_processes, 0);
                assert!(inventory.reasons.is_empty());
                let marked = environment.starts_with("AGENT_SESSION_ID=");
                assert_eq!(scan.processes.len(), usize::from(marked));
                if marked {
                    assert_eq!(scan.processes[0].command, "provider");
                }
            } else {
                assert_eq!(inventory.complete, false);
                assert_eq!(inventory.inspected_processes, 0);
                assert_eq!(inventory.uninspected_processes, 1);
                assert_eq!(inventory.reasons["argv-only-or-sip-redacted"], 1);
                assert!(scan.processes.is_empty());
            }
        }
    }
    fn session_fixture() -> (tempfile::TempDir, CliContext, SessionRecord) {
        let tmp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = create_record(RecordRequest {
            context: &context,
            agent: AgentKind::Claude,
            mode: "interactive",
            coordination_mode: cli::CoordinationMode::Advisory,
            title: None,
            title_state: None,
            explicit_id: Some("detached-survivor"),
            cwd: tmp.path(),
            prompt: None,
            log_file_name: None,
            provider_resume: None,
            agent_args: Vec::new(),
            agent_bin: None,
        })
        .unwrap()
        .record;
        (tmp, context, record)
    }

    #[test]
    fn delete_text_describes_verified_stop_instead_of_a_kill_command() {
        let result = DeleteResult {
            children: None,
            termination_scope: "recorded-boundaries",
            process_inventory: super::ProcessInventory {
                complete: true,
                ..Default::default()
            },
            id: "stopped-session".to_string(),
            tmux_session: "hs-codex-stopped-session".to_string(),
            killed: true,
            deleted: true,
            session_dir: "/state/sessions/stopped-session".to_string(),
            cleanup_pending: false,
            registry_fence: SessionRegistryFence {
                session_id: "stopped-session".to_string(),
                tmux_session: "hs-codex-stopped-session".to_string(),
                runtime_launch_id: None,
                runtime_generation: None,
            },
        };

        assert_eq!(
            render_delete_text(&result),
            "deleted stopped-session (tmux stopped: yes)\n"
        );
    }

    #[test]
    fn delete_incomplete_inventory_success_is_explicit_in_json_and_text() {
        let (_tmp, context, record) = session_fixture();
        let directory = session_dir(&context, &record.id);
        let mut scan = ProcessScan {
            complete: true,
            inspected: 3,
            ..Default::default()
        };
        scan.read_failed(&io::Error::from(io::ErrorKind::PermissionDenied));
        scan.incomplete("argv-only-or-sip-redacted", 1);
        let result = finish_session_delete_with_scan(
            &context,
            record.clone(),
            directory.clone(),
            SessionRegistryFence::from_record(&record),
            board::CloseReason::Deleted,
            scan,
        )
        .unwrap();
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["termination_scope"], "recorded-boundaries");
        assert_eq!(value["process_inventory"]["complete"], false);
        assert_eq!(value["process_inventory"]["inspected_processes"], 3);
        assert_eq!(value["process_inventory"]["uninspected_processes"], 2);
        assert_eq!(
            value["process_inventory"]["reasons"]["permission-denied"],
            1
        );
        assert_eq!(
            value["process_inventory"]["reasons"]["argv-only-or-sip-redacted"],
            1
        );
        let text = render_delete_text(&result);
        assert!(text.contains("process inventory incomplete"));
        assert!(text.contains("permission-denied: 1"));
        assert!(text.contains("additional processes may remain"));
        assert!(!directory.join("session.json").exists());
    }

    #[test]
    fn delete_incomplete_inventory_preserves_retry_for_unobserved_failures() {
        let (_tmp, _context, record) = session_fixture();
        let mut scan = ProcessScan::default();
        scan.read_failed(&io::Error::from(io::ErrorKind::PermissionDenied));
        let error = termination_error_with_scan(
            &record,
            SessionTerminationFailure::VerificationFailed,
            SessionTerminationOperation::Delete,
            scan,
        );
        let details = error.0.details.unwrap();
        assert_eq!(details["retryable"], true);
        assert_eq!(details["action"], "retry-delete");
        assert_eq!(details["process_inventory"]["complete"], false);
        assert_eq!(details["process_inventory"]["uninspected_processes"], 1);
        assert_eq!(
            details["process_inventory"]["reasons"]["permission-denied"],
            1
        );
        assert!(error.0.hint.unwrap().contains("permission-denied: 1"));
    }

    #[test]
    fn ordinary_delete_retains_state_after_pane_stop_when_detached_markers_survive() {
        struct Survivor(std::process::Child);
        impl Drop for Survivor {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let (_tmp, context, record) = session_fixture();
        let directory = session_dir(&context, &record.id);
        let fence = SessionRegistryFence::from_record(&record);
        #[cfg(not(target_os = "macos"))]
        let mut command = {
            let mut command = ProcessCommand::new("sleep");
            command.arg("30");
            command
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = ProcessCommand::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "delete_recovery::tests::macos_marker_child"])
                .env("NILS_TEST_MARKER_CHILD", "1")
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            command
        };
        command.env("AGENT_SESSION_ID", &record.id).env(
            "AGENT_SESSION_RUNTIME_ID",
            &record.runtime.as_ref().unwrap().launch_id,
        );
        // Detach from the caller's POSIX session before exec. The fixture owns
        // the child handle and always reaps it, including on assertion failure.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let mut survivor = Survivor(command.spawn().unwrap());
        let error = finish_session_delete(
            &context,
            record,
            directory.clone(),
            fence,
            board::CloseReason::Deleted,
        )
        .unwrap_err();
        assert_eq!(error.code(), "session-termination-failed");
        let details = error.0.details.unwrap();
        assert_eq!(details["reason"], "process-still-running");
        assert_eq!(details["retryable"], false);
        assert!(
            details["blocking_processes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|process| process["pid"] == survivor.0.id()
                    && process["command"]
                        .as_str()
                        .is_some_and(|name| !name.is_empty()))
        );
        assert!(directory.join("session.json").exists());
        assert!(survivor.0.try_wait().unwrap().is_none());
    }
}

#[derive(Debug, Serialize)]
pub struct DeleteResult {
    /// The children check of a user delete or archive
    /// (`session-lineage-work-v1`); absent on internal cleanup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<lineage::ChildrenCheck>,
    pub(super) id: String,
    pub(super) tmux_session: String,
    pub(super) killed: bool,
    pub deleted: bool,
    pub termination_scope: &'static str,
    pub process_inventory: ProcessInventory,
    pub(super) session_dir: String,
    #[serde(skip)]
    pub cleanup_pending: bool,
    #[serde(skip)]
    pub(super) registry_fence: SessionRegistryFence,
}

pub(super) fn render_delete_text(result: &DeleteResult) -> String {
    let mut text = format!(
        "deleted {} (tmux stopped: {})\n",
        result.id,
        if result.killed { "yes" } else { "no" }
    );
    for child in result
        .children
        .iter()
        .flat_map(|children| &children.orphaned)
    {
        text.push_str(&format!("orphaned child: {}\n", child.session_id));
    }
    text.push_str(&shared_daemons::render_retained(&result.process_inventory));
    text.push_str(&inventory_warning(&result.process_inventory));
    text
}
