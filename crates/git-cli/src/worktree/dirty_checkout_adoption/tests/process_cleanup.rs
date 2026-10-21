use super::process_supervisor::{pin_published_process_with, signal_published_process_with};
use super::*;

#[cfg(target_os = "linux")]
#[test]
fn process_identity_replacement_before_pinning_receives_no_signal() {
    let generation_a = ProcessIdentity {
        pid: 42_424,
        generation: 11,
    };
    let generation_b = ProcessIdentity {
        pid: generation_a.pid,
        generation: 12,
    };
    let current = std::cell::Cell::new(generation_a);
    let mut signals = Vec::new();

    let pinned = pin_process_identity_result_with(
        generation_a,
        |_| {
            current.set(generation_b);
            Ok(generation_a.pid)
        },
        |_| Ok(Some(current.get())),
    )
    .expect("post-open generation validation succeeds");
    if let Some(handle) = pinned {
        signals.push((handle, libc::SIGKILL));
    }

    assert!(pinned.is_none(), "generation B must not be pinned as A");
    assert!(signals.is_empty(), "generation B must receive zero signals");
}

#[test]
fn delayed_cleanup_signals_use_only_the_successfully_pinned_generation() {
    struct SingleOwner(ProcessIdentity);

    impl OwnedProcessTracker for SingleOwner {
        fn identities(&mut self, _deadline: Instant) -> Result<Vec<ProcessIdentity>> {
            Ok(vec![self.0])
        }

        fn known_identities(&self) -> Vec<ProcessIdentity> {
            vec![self.0]
        }
    }

    struct ReplacingControl {
        replacement: ProcessIdentity,
        current: ProcessIdentity,
        signals: Vec<(ProcessIdentity, libc::c_int)>,
    }

    impl OwnedProcessControl for ReplacingControl {
        type Pinned = ProcessIdentity;

        fn pin(&mut self, identity: ProcessIdentity) -> Result<Option<Self::Pinned>> {
            self.current = self.replacement;
            Ok(Some(identity))
        }

        fn signal(&mut self, process: &Self::Pinned, signal: libc::c_int) -> Result<bool> {
            self.signals.push((*process, signal));
            Ok(true)
        }
    }

    let generation_a = ProcessIdentity {
        pid: 42_424,
        generation: 11,
    };
    let generation_b = ProcessIdentity {
        pid: generation_a.pid,
        generation: 12,
    };
    let mut owner = SingleOwner(generation_a);
    let mut control = ReplacingControl {
        replacement: generation_b,
        current: generation_a,
        signals: Vec::new(),
    };
    let mut child = Command::new(true_executable())
        .spawn()
        .expect("spawn completed cleanup child");

    terminate_owned_processes_until_with(
        &mut owner,
        &mut child,
        Instant::now() + TEST_WAIT,
        &mut control,
    )
    .expect("opaque-handle cleanup succeeds");

    assert_eq!(control.current, generation_b);
    assert_eq!(
        control.signals,
        vec![(generation_a, libc::SIGSTOP), (generation_a, libc::SIGKILL)],
        "cleanup must signal only the opaque handle acquired for generation A"
    );
    assert!(
        control
            .signals
            .iter()
            .all(|(handle, _)| *handle != generation_b),
        "the replacement generation must receive no cleanup signal"
    );
}

#[test]
fn published_pid_cleanup_pins_the_original_generation_before_delayed_signal() {
    struct ReplacingControl {
        current: ProcessIdentity,
        replacement: ProcessIdentity,
        signals: Vec<(ProcessIdentity, libc::c_int)>,
    }

    impl OwnedProcessControl for ReplacingControl {
        type Pinned = ProcessIdentity;

        fn pin(&mut self, identity: ProcessIdentity) -> Result<Option<Self::Pinned>> {
            self.current = self.replacement;
            Ok(Some(identity))
        }

        fn signal(&mut self, process: &Self::Pinned, signal: libc::c_int) -> Result<bool> {
            self.signals.push((*process, signal));
            Ok(true)
        }
    }

    let generation_a = ProcessIdentity {
        pid: 54_321,
        generation: 7,
    };
    let generation_b = ProcessIdentity {
        pid: generation_a.pid,
        generation: 8,
    };
    let mut control = ReplacingControl {
        current: generation_a,
        replacement: generation_b,
        signals: Vec::new(),
    };

    let pinned = pin_published_process_with(&mut control, generation_a)
        .expect("pin published generation")
        .expect("published generation exists");
    signal_published_process_with(&mut control, &pinned).expect("clean published generation");

    assert_eq!(control.current, generation_b);
    assert_eq!(control.signals, vec![(generation_a, libc::SIGKILL)]);
    assert!(
        control
            .signals
            .iter()
            .all(|(identity, _)| *identity != generation_b),
        "delayed fixture cleanup must never signal the reused PID generation"
    );
}

#[test]
fn supervisor_cleanup_completion_requires_authenticated_proof() {
    let mut valid = Vec::new();
    write_process_supervisor_completion(
        &mut valid,
        ProcessSupervisorCompletion {
            kind: ProcessSupervisorCompletionKind::Terminated,
            cleanup_complete: true,
            exit_code: exit::RUNTIME,
        },
    )
    .expect("serialize authenticated cleanup completion");
    let mut overlong = valid.clone();
    overlong.push(0);
    assert!(read_supervisor_cleanup_proof_with(&mut io::Cursor::new(
        valid
    )));
    assert!(!read_supervisor_cleanup_proof_with(&mut io::Cursor::new(
        overlong
    )));
    assert!(!read_supervisor_cleanup_proof_with(&mut io::Cursor::new(
        Vec::<u8>::new()
    )));
    assert!(!read_supervisor_cleanup_proof_with(&mut io::Cursor::new(
        b"wrong-proof".to_vec()
    )));
}
