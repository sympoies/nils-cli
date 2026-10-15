#[cfg(target_os = "linux")]
use super::supervisor_lifecycle::assert_process_absent;
use super::*;

#[test]
fn process_cleanup_budget_allows_bounded_slow_discovery() {
    const SLOW_DISCOVERY: Duration = Duration::from_millis(350);

    struct SlowOwner;

    impl OwnedProcessTracker for SlowOwner {
        fn refresh(&mut self, deadline: Instant) -> Result<()> {
            std::thread::sleep(SLOW_DISCOVERY);
            ensure_deadline(deadline)
        }

        fn identities(&mut self, _deadline: Instant) -> Result<Vec<ProcessIdentity>> {
            Ok(Vec::new())
        }

        fn known_identities(&self) -> Vec<ProcessIdentity> {
            Vec::new()
        }
    }

    let started = Instant::now();
    let mut child = Command::new(true_executable())
        .spawn()
        .expect("spawn bounded slow-discovery child");

    terminate_owned_processes(&mut SlowOwner, &mut child)
        .expect("bounded process discovery must retain enough cleanup headroom");

    assert!(
        started.elapsed() < Duration::from_secs(2),
        "bounded cleanup discovery must not turn into an unbounded wait"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn process_cleanup_deadline_bounds_every_cleanup_phase() {
    struct EmptyOwner;

    impl OwnedProcessTracker for EmptyOwner {
        fn identities(&mut self, _deadline: Instant) -> Result<Vec<ProcessIdentity>> {
            Ok(Vec::new())
        }

        fn known_identities(&self) -> Vec<ProcessIdentity> {
            Vec::new()
        }
    }

    let mut child = Command::new("/bin/sh")
        .args(["-c", "while :; do :; done"])
        .spawn()
        .expect("spawn expired cleanup fixture");
    let pid = child.id() as libc::pid_t;
    let mut owner = EmptyOwner;
    let error = terminate_owned_processes_until(
        &mut owner,
        &mut child,
        Instant::now() - Duration::from_millis(1),
    )
    .expect_err("an expired cleanup deadline must fail closed");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed cleanup timeout")
            .kind(),
        DirtyCheckoutErrorKind::Timeout
    );
    let status = child.wait().expect("reap expired-deadline direct child");
    assert!(
        !status.success(),
        "the direct child must still be terminated"
    );
    assert_process_absent(pid, "expired-deadline direct child");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_process_scan_is_linear_bounded_and_deadline_first() {
    let limits = ProcessScanLimits {
        process_entries: 6,
        metadata_bytes: 600,
        descendants: 4,
    };
    let records = vec![
        ProcessRecord::new(10, 1, 10, 100),
        ProcessRecord::new(11, 10, 11, 100),
        ProcessRecord::new(12, 10, 12, 100),
        ProcessRecord::new(13, 11, 13, 100),
        ProcessRecord::new(14, 12, 14, 100),
        ProcessRecord::new(99, 1, 99, 100),
    ];
    let scan = collect_descendant_processes_with(
        10,
        Instant::now() + Duration::from_secs(1),
        limits,
        |_| Ok(records.clone()),
    )
    .expect("exact process, byte, and descendant caps must pass");
    assert_eq!(scan.identities.len(), 4);
    assert_eq!(scan.edge_visits, 4, "each descendant edge is visited once");

    let assert_resource = |result: Result<ProcessScan>| {
        assert_eq!(
            result
                .expect_err("cap plus one must fail")
                .downcast_ref::<DirtyCheckoutError>()
                .expect("typed process scan error")
                .kind(),
            DirtyCheckoutErrorKind::ResourceUnavailable
        );
    };
    assert_resource(collect_descendant_processes_with(
        10,
        Instant::now() + Duration::from_secs(1),
        ProcessScanLimits {
            process_entries: 5,
            ..limits
        },
        |_| Ok(records.clone()),
    ));
    assert_resource(collect_descendant_processes_with(
        10,
        Instant::now() + Duration::from_secs(1),
        ProcessScanLimits {
            metadata_bytes: 599,
            ..limits
        },
        |_| Ok(records.clone()),
    ));
    assert_resource(collect_descendant_processes_with(
        10,
        Instant::now() + Duration::from_secs(1),
        ProcessScanLimits {
            descendants: 3,
            ..limits
        },
        |_| Ok(records),
    ));

    let mut reads = 0;
    let expired = collect_descendant_processes_with(
        10,
        Instant::now() - Duration::from_millis(1),
        limits,
        |_| {
            reads += 1;
            Ok(Vec::new())
        },
    )
    .expect_err("an expired cleanup deadline must fail");
    assert_eq!(
        expired
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed cleanup timeout")
            .kind(),
        DirtyCheckoutErrorKind::Timeout
    );
    assert_eq!(reads, 0, "no process metadata may be read after expiry");
}

#[cfg(target_os = "linux")]
#[test]
fn linux_targeted_reader_scans_only_reachable_processes_and_retains_a_bounded_prefix() {
    fn write_process(root: &Path, pid: libc::pid_t, parent: libc::pid_t, children: &str) {
        let task = root
            .join(pid.to_string())
            .join("task")
            .join(pid.to_string());
        fs::create_dir_all(&task).expect("create synthetic process task");
        fs::write(task.join("children"), children).expect("write synthetic children");
        let mut stat = format!("{pid} (process-{pid}) S {parent}").into_bytes();
        for _ in 0..17 {
            stat.extend_from_slice(b" 0");
        }
        stat.extend_from_slice(format!(" {pid}00\n").as_bytes());
        fs::write(root.join(pid.to_string()).join("stat"), stat).expect("write synthetic stat");
    }

    let proc_root = tempfile::TempDir::new().expect("synthetic proc root");
    write_process(proc_root.path(), 10, 1, "11\n");
    write_process(proc_root.path(), 11, 10, "12\n");
    write_process(proc_root.path(), 12, 11, "");
    write_process(proc_root.path(), 99, 1, "100\n");
    write_process(proc_root.path(), 100, 99, "");
    let limits = ProcessScanLimits {
        process_entries: 8,
        metadata_bytes: 4 * 1024,
        descendants: 2,
    };
    let mut observed = Vec::new();

    let scan = descendant_processes_at(
        proc_root.path(),
        10,
        Instant::now() + Duration::from_secs(1),
        limits,
        |identity| observed.push(identity),
    )
    .expect("reachable synthetic descendants must be discovered");

    assert_eq!(
        scan.identities
            .iter()
            .map(|identity| identity.pid)
            .collect::<Vec<_>>(),
        vec![11, 12]
    );
    assert_eq!(scan.edge_visits, 2);
    assert_eq!(observed, scan.identities);
    assert!(scan.identities.iter().all(|identity| identity.pid != 99));

    let mut bounded_prefix = Vec::new();
    let error = descendant_processes_at(
        proc_root.path(),
        10,
        Instant::now() + Duration::from_secs(1),
        ProcessScanLimits {
            descendants: 1,
            ..limits
        },
        |identity| bounded_prefix.push(identity),
    )
    .expect_err("descendant cap plus one must fail closed");
    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed targeted scan error")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
    assert_eq!(
        bounded_prefix
            .iter()
            .map(|identity| identity.pid)
            .collect::<Vec<_>>(),
        vec![11],
        "already discovered identities must remain available for cleanup"
    );

    let late_task = proc_root.path().join("10/task/20");
    fs::create_dir_all(&late_task).expect("create late synthetic task");
    fs::write(late_task.join("children"), b"11\n").expect("write initial late-task metadata");
    let task_children = [
        proc_root.path().join("10/task/10/children"),
        late_task.join("children"),
    ];
    let mut streamed_prefix = Vec::new();
    descendant_processes_at(
        proc_root.path(),
        10,
        Instant::now() + Duration::from_secs(1),
        ProcessScanLimits {
            metadata_bytes: MAX_PROC_CHILDREN_BYTES * 2,
            ..limits
        },
        |identity| {
            streamed_prefix.push(identity);
            for path in &task_children {
                fs::write(path, vec![b' '; MAX_PROC_CHILDREN_BYTES + 1])
                    .expect("inject oversized remaining-task metadata");
            }
        },
    )
    .expect_err("a later task metadata overflow must fail closed");
    assert_eq!(
        streamed_prefix
            .iter()
            .map(|identity| identity.pid)
            .collect::<Vec<_>>(),
        vec![11],
        "a later task failure must preserve the earlier task's discovered identity"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_targeted_reader_rejects_recycled_child_pid_with_unrelated_parent() {
    fn write_process(root: &Path, pid: libc::pid_t, parent: libc::pid_t, children: &str) {
        let task = root
            .join(pid.to_string())
            .join("task")
            .join(pid.to_string());
        fs::create_dir_all(&task).expect("create synthetic process task");
        fs::write(task.join("children"), children).expect("write synthetic children");
        let mut stat = format!("{pid} (process-{pid}) S {parent}").into_bytes();
        for _ in 0..17 {
            stat.extend_from_slice(b" 0");
        }
        stat.extend_from_slice(format!(" {pid}00\n").as_bytes());
        fs::write(root.join(pid.to_string()).join("stat"), stat).expect("write synthetic stat");
    }

    let proc_root = tempfile::TempDir::new().expect("synthetic proc root");
    write_process(proc_root.path(), 10, 1, "11\n");
    write_process(proc_root.path(), 11, 99, "");

    let scan = descendant_processes_at(
        proc_root.path(),
        10,
        Instant::now() + Duration::from_secs(1),
        ProcessScanLimits {
            process_entries: 4,
            metadata_bytes: 1024,
            descendants: 2,
        },
        |_| {},
    )
    .expect("an unrelated PID generation must be skipped");

    assert!(
        scan.identities.is_empty(),
        "a recycled children entry whose current occupant has another parent is not owned"
    );
}
