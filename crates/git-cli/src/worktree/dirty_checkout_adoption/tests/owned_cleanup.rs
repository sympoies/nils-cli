#[cfg(target_os = "linux")]
use super::process_supervisor::spawn_authenticated_supervisor_output_fixture;
#[cfg(target_os = "linux")]
use super::supervisor_lifecycle::assert_process_absent;
use super::*;

#[test]
fn reaped_supervisor_disables_unpinned_pid_and_process_group_signals() {
    assert!(!should_use_unpinned_cleanup_signals(true, true));
    assert!(should_use_unpinned_cleanup_signals(true, false));
    assert!(should_use_unpinned_cleanup_signals(false, true));
}

#[test]
fn supervised_timeout_cleanup_signals_term_before_any_fallback_stop() {
    let mut signals = Vec::new();
    signal_cleanup_leader_with(42, true, |target, signal| {
        signals.push((target, signal));
        Ok(())
    })
    .expect("initial supervisor cleanup signal succeeds");

    assert_eq!(
        signals,
        vec![(42, libc::SIGTERM)],
        "the supervisor must retain execution so its ownership tracker can clean descendants"
    );
}

#[test]
fn owned_cleanup_continues_until_a_quiescent_generation_pass() {
    struct WaveOwner {
        next_generation: u128,
        last_generation: u128,
    }

    impl OwnedProcessTracker for WaveOwner {
        fn identities(&mut self, _deadline: Instant) -> Result<Vec<ProcessIdentity>> {
            if self.next_generation > self.last_generation {
                return Ok(Vec::new());
            }
            let identity = ProcessIdentity {
                pid: 40_000 + self.next_generation as libc::pid_t,
                generation: self.next_generation,
            };
            self.next_generation += 1;
            Ok(vec![identity])
        }

        fn known_identities(&self) -> Vec<ProcessIdentity> {
            Vec::new()
        }
    }

    struct RecordingControl(Vec<(ProcessIdentity, libc::c_int)>);

    impl OwnedProcessControl for RecordingControl {
        type Pinned = ProcessIdentity;

        fn pin(&mut self, identity: ProcessIdentity) -> Result<Option<Self::Pinned>> {
            Ok(Some(identity))
        }

        fn signal(&mut self, process: &Self::Pinned, signal: libc::c_int) -> Result<bool> {
            self.0.push((*process, signal));
            Ok(true)
        }
    }

    let mut owner = WaveOwner {
        next_generation: 1,
        last_generation: 5,
    };
    let mut control = RecordingControl(Vec::new());
    let mut child = Command::new(true_executable())
        .spawn()
        .expect("spawn completed cleanup child");

    terminate_owned_processes_until_with(
        &mut owner,
        &mut child,
        Instant::now() + Duration::from_secs(1),
        &mut control,
    )
    .expect("all escaping generations are cleaned");

    let killed: Vec<_> = control
        .0
        .iter()
        .filter_map(|(identity, signal)| (*signal == libc::SIGKILL).then_some(*identity))
        .collect();
    assert_eq!(
        killed.len(),
        5,
        "cleanup must not treat four discovery passes as a containment boundary"
    );
}

#[test]
fn owned_cleanup_rejects_cumulative_generation_limit_plus_one() {
    struct ChurningOwner {
        next_generation: usize,
    }

    impl OwnedProcessTracker for ChurningOwner {
        fn identities(&mut self, _deadline: Instant) -> Result<Vec<ProcessIdentity>> {
            if self.next_generation > PROCESS_SCAN_LIMITS.descendants + 1 {
                return Ok(Vec::new());
            }
            let identity = ProcessIdentity {
                pid: 42_424,
                generation: self.next_generation as u128,
            };
            self.next_generation += 1;
            Ok(vec![identity])
        }

        fn known_identities(&self) -> Vec<ProcessIdentity> {
            Vec::new()
        }
    }

    struct CountingControl {
        pinned: usize,
    }

    impl OwnedProcessControl for CountingControl {
        type Pinned = ProcessIdentity;

        fn pin(&mut self, identity: ProcessIdentity) -> Result<Option<Self::Pinned>> {
            self.pinned += 1;
            Ok(Some(identity))
        }

        fn signal(&mut self, _process: &Self::Pinned, _signal: libc::c_int) -> Result<bool> {
            Ok(true)
        }
    }

    let mut owner = ChurningOwner { next_generation: 1 };
    let mut control = CountingControl { pinned: 0 };
    let mut child = Command::new(true_executable())
        .spawn()
        .expect("spawn completed cleanup child");

    let error = terminate_owned_processes_until_with(
        &mut owner,
        &mut child,
        Instant::now() + Duration::from_secs(5),
        &mut control,
    )
    .expect_err("cumulative generation churn must fail closed");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed generation-limit error")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
    assert_eq!(control.pinned, PROCESS_SCAN_LIMITS.descendants);
}

#[test]
fn failed_kill_resumes_a_stopped_pinned_process_before_cleanup_returns_error() {
    struct SingleOwner(ProcessIdentity);

    impl OwnedProcessTracker for SingleOwner {
        fn identities(&mut self, _deadline: Instant) -> Result<Vec<ProcessIdentity>> {
            Ok(vec![self.0])
        }

        fn known_identities(&self) -> Vec<ProcessIdentity> {
            vec![self.0]
        }
    }

    struct KillFailingControl {
        signals: Vec<libc::c_int>,
    }

    impl OwnedProcessControl for KillFailingControl {
        type Pinned = ProcessIdentity;

        fn pin(&mut self, identity: ProcessIdentity) -> Result<Option<Self::Pinned>> {
            Ok(Some(identity))
        }

        fn signal(&mut self, _process: &Self::Pinned, signal: libc::c_int) -> Result<bool> {
            self.signals.push(signal);
            if signal == libc::SIGKILL {
                Err(process_scan_resource_error())
            } else {
                Ok(true)
            }
        }
    }

    let identity = ProcessIdentity {
        pid: 42_424,
        generation: 11,
    };
    let mut owner = SingleOwner(identity);
    let mut control = KillFailingControl {
        signals: Vec::new(),
    };
    let mut child = Command::new(true_executable())
        .spawn()
        .expect("spawn completed cleanup child");

    terminate_owned_processes_until_with(
        &mut owner,
        &mut child,
        Instant::now() + Duration::from_secs(1),
        &mut control,
    )
    .expect_err("failed termination must fail containment");

    assert_eq!(
        control.signals,
        vec![libc::SIGSTOP, libc::SIGKILL, libc::SIGCONT],
        "cleanup must resume a process that it stopped but could not terminate"
    );
}

#[test]
fn successful_kill_waits_for_pinned_process_termination_before_cleanup_returns() {
    struct TerminationControl {
        signals: Vec<libc::c_int>,
        waited: bool,
    }

    impl OwnedProcessControl for TerminationControl {
        type Pinned = ProcessIdentity;

        fn pin(&mut self, identity: ProcessIdentity) -> Result<Option<Self::Pinned>> {
            Ok(Some(identity))
        }

        fn signal(&mut self, _process: &Self::Pinned, signal: libc::c_int) -> Result<bool> {
            self.signals.push(signal);
            Ok(true)
        }

        fn wait_for_exit(&mut self, _process: &Self::Pinned, _deadline: Instant) -> Result<()> {
            self.waited = true;
            Ok(())
        }
    }

    let identity = ProcessIdentity {
        pid: 42_424,
        generation: 11,
    };
    let mut control = TerminationControl {
        signals: Vec::new(),
        waited: false,
    };

    terminate_owned_identity_until_with(
        &mut control,
        identity,
        Instant::now() + Duration::from_secs(1),
    )
    .expect("stable termination is observed");

    assert_eq!(control.signals, vec![libc::SIGSTOP, libc::SIGKILL]);
    assert!(
        control.waited,
        "cleanup completion must wait until the pinned generation terminates"
    );
}

#[test]
fn cleanup_deadline_resumes_a_process_stopped_mid_phase_without_late_kill() {
    struct SingleOwner(ProcessIdentity);

    impl OwnedProcessTracker for SingleOwner {
        fn identities(&mut self, _deadline: Instant) -> Result<Vec<ProcessIdentity>> {
            Ok(vec![self.0])
        }

        fn known_identities(&self) -> Vec<ProcessIdentity> {
            vec![self.0]
        }
    }

    struct DeadlineCrossingControl {
        signals: Vec<libc::c_int>,
    }

    impl OwnedProcessControl for DeadlineCrossingControl {
        type Pinned = ProcessIdentity;

        fn pin(&mut self, identity: ProcessIdentity) -> Result<Option<Self::Pinned>> {
            Ok(Some(identity))
        }

        fn signal(&mut self, _process: &Self::Pinned, signal: libc::c_int) -> Result<bool> {
            self.signals.push(signal);
            if signal == libc::SIGSTOP {
                std::thread::sleep(Duration::from_millis(75));
            }
            Ok(true)
        }
    }

    let identity = ProcessIdentity {
        pid: 42_424,
        generation: 11,
    };
    let mut owner = SingleOwner(identity);
    let mut control = DeadlineCrossingControl {
        signals: Vec::new(),
    };
    let mut child = Command::new(true_executable())
        .spawn()
        .expect("spawn completed cleanup child");

    terminate_owned_processes_until_with(
        &mut owner,
        &mut child,
        Instant::now() + Duration::from_millis(50),
        &mut control,
    )
    .expect_err("crossing the shared cleanup deadline must fail containment");

    assert_eq!(
        control.signals,
        vec![libc::SIGSTOP, libc::SIGCONT],
        "deadline expiry must resume the stopped process instead of signaling after expiry"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_process_pinning_propagates_post_open_identity_read_failures() {
    let identity = ProcessIdentity {
        pid: 42_424,
        generation: 11,
    };

    let error = pin_process_identity_result_with::<(), _, _>(
        identity,
        |_| Ok(()),
        |_| Err(process_scan_resource_error()),
    )
    .expect_err("post-pidfd identity read failures must fail containment");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed identity-read containment error")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_process_pinning_treats_only_esrch_as_vanished() {
    let identity = ProcessIdentity {
        pid: 42_424,
        generation: 11,
    };
    let vanished = pin_process_identity_result_with::<(), _, _>(
        identity,
        |_| Err(io::Error::from_raw_os_error(libc::ESRCH)),
        |_| Ok(Some(identity)),
    )
    .expect("ESRCH means the discovered generation has exited");
    assert!(vanished.is_none());

    for error_code in [libc::ENOSYS, libc::EMFILE, libc::ENFILE, libc::EPERM] {
        let error = pin_process_identity_result_with::<(), _, _>(
            identity,
            |_| Err(io::Error::from_raw_os_error(error_code)),
            |_| Ok(Some(identity)),
        )
        .expect_err("pidfd resource and policy failures must fail containment");
        assert_eq!(
            error
                .downcast_ref::<DirtyCheckoutError>()
                .expect("typed pidfd containment error")
                .kind(),
            DirtyCheckoutErrorKind::ResourceUnavailable
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_pidfd_signaling_treats_only_esrch_as_vanished() {
    assert!(signal_pinned_process_result_with(|| Ok(())).expect("successful pidfd signal"));
    assert!(
        !signal_pinned_process_result_with(|| Err(io::Error::from_raw_os_error(libc::ESRCH)))
            .expect("ESRCH means the pinned generation has exited")
    );

    for error_code in [libc::ENOSYS, libc::EBADF, libc::EPERM, libc::EINVAL] {
        let error =
            signal_pinned_process_result_with(|| Err(io::Error::from_raw_os_error(error_code)))
                .expect_err("pidfd signal failures other than ESRCH must fail containment");
        assert_eq!(
            error
                .downcast_ref::<DirtyCheckoutError>()
                .expect("typed pidfd signal containment error")
                .kind(),
            DirtyCheckoutErrorKind::ResourceUnavailable
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn cleanup_failure_terminates_known_processes_and_cannot_report_success() {
    struct FailingOwner {
        known: Vec<ProcessIdentity>,
    }

    impl OwnedProcessTracker for FailingOwner {
        fn identities(&mut self, _deadline: Instant) -> Result<Vec<ProcessIdentity>> {
            Err(process_scan_resource_error())
        }

        fn known_identities(&self) -> Vec<ProcessIdentity> {
            self.known.clone()
        }
    }

    let mut child = Command::new("/bin/sh")
        .args(["-c", "while :; do :; done"])
        .spawn()
        .expect("spawn cleanup failure fixture");
    let pid = child.id() as libc::pid_t;
    let identity = process_identity(pid)
        .expect("read cleanup fixture identity")
        .expect("cleanup fixture process exists");
    let mut owner = FailingOwner {
        known: vec![identity],
    };

    let error = terminate_owned_processes(&mut owner, &mut child)
        .expect_err("incomplete discovery must fail containment after cleanup");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed cleanup failure")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
    assert_process_absent(pid, "known process after incomplete discovery");
}

// Hosted macOS coverage races kqueue ProcessOwner setup against the exiting group leader; see #1290.
#[cfg(target_os = "linux")]
#[test]
fn signal_terminated_target_cannot_be_authenticated_as_git_status_one() {
    let root = tempfile::TempDir::new().expect("signal target producer root");
    let spawned = spawn_authenticated_supervisor_output_fixture(
        "target-signal",
        &root.path().join("unused.pid"),
    );

    let error = collect_output_child_until(
        spawned,
        Instant::now() + Duration::from_secs(5),
        1024,
        1024,
        1024,
    )
    .expect_err("signal termination must remain outside the Git exit-status domain");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed abnormal-target result")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
}

#[test]
fn successful_child_with_cleanup_failure_cannot_cross_supervisor_success_boundary() {
    struct CleanupFailingOwner;

    impl OwnedProcessTracker for CleanupFailingOwner {
        fn identities(&mut self, _deadline: Instant) -> Result<Vec<ProcessIdentity>> {
            Err(process_scan_resource_error())
        }

        fn known_identities(&self) -> Vec<ProcessIdentity> {
            Vec::new()
        }
    }

    let error = supervise_process_with_owner(
        OsString::from(true_executable()),
        &[],
        &mut CleanupFailingOwner,
        None,
        Instant::now() + Duration::from_secs(1),
        || false,
    )
    .expect_err("cleanup failure must replace a successful child result");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed supervisor cleanup failure")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
}
