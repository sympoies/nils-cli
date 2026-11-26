use super::*;

const SIGNAL_GRACE: Duration = Duration::from_millis(250);
const KILL_WAIT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub(super) struct AuditToken([u32; 8]);

unsafe extern "C" {
    static mach_task_self_: libc::mach_port_t;
    fn task_name_for_pid(
        caller: libc::mach_port_t,
        pid: libc::pid_t,
        name: *mut libc::mach_port_t,
    ) -> libc::kern_return_t;
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
}

type SignalProcess = unsafe extern "C" fn(*mut AuditToken, libc::c_int) -> libc::c_int;

pub(super) fn terminate(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    // The audit-token signal API was added during macOS 14. Older systems
    // report attributed survivors rather than racing a signal to a bare PID.
    let symbol =
        unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"proc_signal_with_audittoken".as_ptr()) };
    let signal = (!symbol.is_null())
        .then(|| unsafe { std::mem::transmute::<*mut libc::c_void, SignalProcess>(symbol) });
    terminate_with(context, record, |mut token, sig| {
        if let Some(signal) = signal {
            // The kernel checks both the PID and its exec generation.
            unsafe { signal(&mut token, sig) };
        }
    })
}

pub(super) fn terminate_with(
    context: &CliContext,
    record: &SessionRecord,
    signal: impl Fn(AuditToken, libc::c_int),
) -> Result<(), CliError> {
    let Some(runtime) = record
        .runtime
        .as_ref()
        .filter(|runtime| !runtime.launch_id.is_empty())
    else {
        return Ok(());
    };
    if recorded_runtime_is_from_prior_boot(record) {
        return Ok(());
    }
    let markers = (&*record.id, &*runtime.launch_id);
    let policy = daemon_policy(record, context);
    let mut scan = scan_processes(markers.0, markers.1, policy.as_ref());
    if scan.orphan_pids.is_empty() {
        return Ok(());
    }

    let Some(argument_limit) = macos_argument_limit() else {
        scan.processes
            .retain(|process| scan.orphan_pids.contains(&process.pid));
        return Err(refusal(record, scan));
    };
    let mut targets = Vec::new();
    let mut unknown = Vec::new();
    let orphan_pids = std::mem::take(&mut scan.orphan_pids);
    for process in std::mem::take(&mut scan.processes) {
        if !orphan_pids.contains(&process.pid) {
            continue;
        }
        match authorize(process.pid, argument_limit, markers, policy.as_ref()) {
            Ok(Some(token)) => {
                signal(token, libc::SIGTERM);
                targets.push((process, token));
            }
            Ok(None) => {}
            Err(_) => unknown.push(process),
        }
    }

    wait_for_exit(&mut targets, SIGNAL_GRACE);
    for (_, token) in &targets {
        signal(*token, libc::SIGKILL);
    }
    wait_for_exit(&mut targets, KILL_WAIT);

    unknown.retain(|process| !matches!(process_info(process.pid), Ok(None)));
    unknown.extend(targets.into_iter().map(|(process, _)| process));
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_by_key(|process| process.pid);
    scan.processes = unknown;
    Err(refusal(record, scan))
}

pub(super) fn refusal(record: &SessionRecord, scan: ProcessScan) -> CliError {
    let mut error = termination_error_with_scan(
        record,
        SessionTerminationFailure::ProcessStillRunning,
        SessionTerminationOperation::Delete,
        scan,
    )
    .with_hint(format!(
        "To close while leaving the listed processes running, run `agent-session delete --leave-processes -- {}`.",
        record.id
    ));
    if let Some(details) = error.0.details.as_mut() {
        details["action"] = json!("delete-with-leave-processes");
    }
    error
}

fn authorize(
    pid: libc::pid_t,
    argument_limit: usize,
    (id, launch): (&str, &str),
    policy: Option<&shared_daemons::DaemonPaths>,
) -> io::Result<Option<AuditToken>> {
    let token = AuditToken::read(pid);
    let Some(info) = process_info(pid)? else {
        return Ok(None);
    };
    if info.pbi_ppid != 1 || info.pbi_uid != unsafe { libc::geteuid() } {
        return Ok(None);
    }
    let token = token.ok_or_else(|| io::Error::other("process audit token unavailable"))?;
    let bytes = macos_process_arguments(pid, argument_limit)?;
    let environment = macos_environment(&bytes)
        .ok_or_else(|| io::Error::other("process environment unavailable"))?;
    if !matches_runtime(environment, id, launch)
        || macos_shared_daemon(&bytes, environment, policy).is_some()
    {
        return Ok(None);
    }
    // Bind the marker read to the same exec generation that will be signaled.
    if AuditToken::read(pid) != Some(token) {
        return Err(io::Error::other("process audit token changed"));
    }
    Ok(Some(token))
}

fn wait_for_exit(targets: &mut Vec<(ProcessSummary, AuditToken)>, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        targets.retain(|(process, token)| match process_info(process.pid) {
            Ok(None) => false,
            Ok(Some(_)) => AuditToken::read(process.pid).is_none_or(|current| current == *token),
            Err(_) => true,
        });
        let remaining = deadline.saturating_duration_since(Instant::now());
        if targets.is_empty() || remaining.is_zero() {
            return;
        }
        thread::sleep(Duration::from_millis(10).min(remaining));
    }
}

fn process_info(pid: libc::pid_t) -> io::Result<Option<libc::proc_bsdinfo>> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read == size {
        let info = unsafe { info.assume_init() };
        return Ok((info.pbi_status != libc::SZOMB).then_some(info));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(None)
    } else {
        Err(error)
    }
}

impl AuditToken {
    fn read(pid: libc::pid_t) -> Option<Self> {
        // TASK_AUDIT_TOKEN returns eight uint32_t values through a task-name port.
        const TASK_AUDIT_TOKEN: libc::task_flavor_t = 15;
        let mut name = 0;
        let caller = unsafe { mach_task_self_ };
        if unsafe { task_name_for_pid(caller, pid, &mut name) } != libc::KERN_SUCCESS {
            return None;
        }
        let mut token = Self([0; 8]);
        let mut count = token.0.len() as libc::mach_msg_type_number_t;
        let status = unsafe {
            let result = libc::task_info(
                name,
                TASK_AUDIT_TOKEN,
                token.0.as_mut_ptr().cast(),
                &mut count,
            );
            mach_port_deallocate(caller, name);
            result
        };
        (status == libc::KERN_SUCCESS
            && count as usize == token.0.len()
            && token.0[5] == pid as u32)
            .then_some(token)
    }
}
