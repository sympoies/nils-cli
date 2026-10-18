use super::*;

pub(super) fn capture_tmux_runtime_identity(
    context: &CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
    timeout: Duration,
) -> Result<TmuxRuntimeProbe, SessionTerminationFailure> {
    capture_tmux_identity(context, record, tmux_bin, timeout, true)
}

fn capture_tmux_identity(
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
    complete: bool,
}

pub(super) fn run_delete(context: &CliContext, args: cli::DeleteArgs) -> i32 {
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
                if !result.survivor_scan_complete {
                    text.push_str(
                        "process inventory incomplete; additional processes may remain\n",
                    );
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

#[derive(Serialize)]
struct RecoveryDeleteResult {
    #[serde(flatten)]
    deleted: DeleteResult,
    termination_scope: &'static str,
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
    let fence = SessionRegistryFence::from_record(&record);
    let mut deleted = commit_session_delete(
        context,
        record,
        resolved.session_dir,
        fence,
        board::CloseReason::Deleted,
    )?;
    // Compatibility field: ordinary deletion uses true only after verifying
    // every retained boundary. Explicit recovery cannot make that claim.
    deleted.killed = false;
    deleted.children = Some(children);
    Ok(RecoveryDeleteResult {
        deleted,
        termination_scope: "tmux-only",
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
    if !scan.processes.is_empty() {
        return Err(termination_error_with_scan(
            &record,
            SessionTerminationFailure::ProcessStillRunning,
            SessionTerminationOperation::Delete,
            scan,
        ));
    }
    commit_session_delete(context, record, session_dir, registry_fence, close_reason)
}

fn commit_session_delete(
    context: &CliContext,
    record: SessionRecord,
    session_dir: PathBuf,
    registry_fence: SessionRegistryFence,
    close_reason: board::CloseReason,
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
    if operation == SessionTerminationOperation::Delete && !scan.processes.is_empty() {
        details["blocking_processes"] = json!(scan.processes);
        details["process_scan_complete"] = json!(scan.complete);
        // Ownership/identity errors still require their original remedy.
        if matches!(
            reason,
            SessionTerminationFailure::VerificationFailed
                | SessionTerminationFailure::ProcessStillRunning
                | SessionTerminationFailure::ProcessIdentityUnavailable
                | SessionTerminationFailure::RuntimeIdentityUnavailable
                | SessionTerminationFailure::ProcessStatusUnavailable
        ) {
            details["retryable"] = json!(false);
            details["action"] = json!("inspect-processes-or-delete-with-leave-processes");
        }
    }
    CliError::runtime(
        "session-termination-failed",
        format!("{}; session metadata was retained", reason.message()),
        Some(details),
    )
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
    let mut scan = ProcessScan {
        complete: true,
        ..Default::default()
    };
    let Ok(entries) = fs::read_dir("/proc") else {
        return ProcessScan::default();
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    for entry in entries {
        if Instant::now() >= deadline || scan.processes.len() >= 128 {
            scan.complete = false;
            break;
        }
        let Ok(entry) = entry else {
            scan.complete = false;
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
        let before = match read_linux_process_identity(pid) {
            Ok(Some(identity)) if !identity.zombie => identity,
            Ok(_) => continue,
            Err(_) => {
                scan.complete = false;
                continue;
            }
        };
        let bytes = match fs::read(entry.path().join("environ")) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                scan.complete = false;
                continue;
            }
        };
        if !matches_runtime(&bytes, id, runtime) {
            continue;
        }
        let command =
            fs::read_to_string(entry.path().join("comm")).unwrap_or_else(|_| "unknown".into());
        // Recheck incarnation after reading diagnostics; reused PIDs must not
        // acquire a former process's runtime marker or command.
        if matches!(read_linux_process_identity(pid), Ok(Some(after)) if after.start_time == before.start_time && !after.zombie)
        {
            scan.processes.push(ProcessSummary {
                pid,
                command: command_name(&command),
            });
        }
    }
    scan.processes.sort_by_key(|process| process.pid);
    scan
}

#[cfg(target_os = "macos")]
fn scan_processes(id: &str, runtime: &str) -> ProcessScan {
    let mut command = ProcessCommand::new("/bin/ps");
    command.args(["-axo", "pid=,uid=,comm="]);
    let Ok(output) = run_output_with_timeout(command, Duration::from_secs(1)) else {
        return ProcessScan::default();
    };
    if !output.status.success() {
        return ProcessScan::default();
    }
    let mut scan = ProcessScan {
        complete: true,
        ..Default::default()
    };
    let deadline = Instant::now() + Duration::from_secs(1);
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if Instant::now() >= deadline {
            scan.complete = false;
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
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        let mut bytes = vec![0u8; 1024 * 1024];
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
            scan.complete = false;
            continue;
        }
        bytes.truncate(length);
        if let Some(environment) = macos_environment(&bytes)
            && matches_runtime(environment, id, runtime)
        {
            scan.processes.push(ProcessSummary {
                pid,
                command: command_name(&fields.collect::<Vec<_>>().join(" ")),
            });
        }
        if scan.processes.len() >= 128 {
            scan.complete = false;
            break;
        }
    }
    scan.processes.sort_by_key(|process| process.pid);
    scan
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
    bytes.get(offset..)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn scan_processes(_: &str, _: &str) -> ProcessScan {
    ProcessScan::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

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
        let directory = session_dir(&context, &record.id);
        let fence = SessionRegistryFence::from_record(&record);
        let mut command = ProcessCommand::new("sleep");
        command.arg("30").env("AGENT_SESSION_ID", &record.id).env(
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
                .any(|process| process["pid"] == survivor.0.id() && process["command"] == "sleep")
        );
        assert!(directory.join("session.json").exists());
        assert!(survivor.0.try_wait().unwrap().is_none());
    }
}
