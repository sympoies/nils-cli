use super::*;

#[test]
fn identity_bound_child_visiting_retains_safe_prefix_and_rejects_parent_swaps() {
    let parent = ProcessIdentity {
        pid: 10,
        generation: 1_000,
    };
    let child = ProcessIdentity {
        pid: 11,
        generation: 1_100,
    };
    let replacement = ProcessIdentity {
        pid: parent.pid,
        generation: 9_999,
    };
    let mut observed = Vec::new();
    let error = visit_identity_bound_children_with(
        parent,
        [Ok(11), Ok(12)],
        |pid| {
            if pid == parent.pid {
                Ok(Some(ProcessRecord::new(pid, 1, parent.generation, 0)))
            } else if pid == child.pid {
                Ok(Some(ProcessRecord::new(
                    pid,
                    parent.pid,
                    child.generation,
                    0,
                )))
            } else {
                Err(process_scan_resource_error())
            }
        },
        |identity| {
            observed.push(identity);
            Ok(())
        },
    )
    .expect_err("a later child metadata failure must fail containment");
    assert!(error.downcast_ref::<DirtyCheckoutError>().is_some());
    assert_eq!(
        observed,
        vec![child],
        "a child validated before a later failure remains available to cleanup"
    );

    let validations = std::cell::Cell::new(0_u8);
    observed.clear();
    visit_identity_bound_children_with(
        parent,
        [Ok(11)],
        |pid| {
            if pid == parent.pid {
                let validation = validations.get();
                validations.set(validation + 1);
                let generation = if validation == 0 {
                    parent.generation
                } else {
                    replacement.generation
                };
                Ok(Some(ProcessRecord::new(pid, 1, generation, 0)))
            } else {
                Ok(Some(ProcessRecord::new(
                    pid,
                    parent.pid,
                    child.generation,
                    0,
                )))
            }
        },
        |identity| {
            observed.push(identity);
            Ok(())
        },
    )
    .expect("a parent replacement is an absent edge, not a scan failure");
    assert!(
        observed.is_empty(),
        "a child edge must not publish across a parent generation change"
    );
}

#[test]
fn macos_fallback_rejects_reused_root_generation_before_descendant_scan() {
    let root = ProcessIdentity {
        pid: 10,
        generation: 1_000,
    };
    let replacement = ProcessIdentity {
        pid: root.pid,
        generation: 9_999,
    };
    let unrelated = ProcessIdentity {
        pid: 11,
        generation: 11_000,
    };
    let scans = std::cell::Cell::new(0_usize);

    let scan = macos_fallback_descendants_with(
        root,
        Instant::now() + Duration::from_secs(1),
        |pid| {
            assert_eq!(pid, root.pid);
            Ok(Some(replacement))
        },
        |identity, _deadline| {
            assert_eq!(identity, root);
            scans.set(scans.get() + 1);
            Ok(ProcessScan {
                identities: vec![unrelated],
                #[cfg(all(test, target_os = "linux"))]
                edge_visits: 1,
            })
        },
    )
    .expect("a reused root is an absent edge, not a scan failure");

    assert!(
        scan.identities.is_empty(),
        "fallback discovery must not adopt descendants through a reused root PID"
    );
    assert_eq!(
        scans.get(),
        0,
        "a stale root identity must suppress descendant discovery"
    );
}

#[test]
fn macos_fallback_rejects_root_reuse_between_precheck_and_scan() {
    let root = ProcessIdentity {
        pid: 20,
        generation: 20_000,
    };
    let replacement = ProcessIdentity {
        pid: root.pid,
        generation: 29_999,
    };
    let unrelated = ProcessIdentity {
        pid: 21,
        generation: 21_000,
    };
    let child_lists = std::cell::Cell::new(0_usize);
    let observed = std::cell::RefCell::new(Vec::new());

    let scan = macos_fallback_descendants_with(
        root,
        Instant::now() + Duration::from_secs(1),
        |pid| {
            assert_eq!(pid, root.pid);
            Ok(Some(root))
        },
        |expected_root, deadline| {
            macos_descendant_processes_from_identity_with(
                expected_root,
                deadline,
                |pid| match pid {
                    pid if pid == root.pid => Ok(Some(ProcessRecord::new(
                        replacement.pid,
                        1,
                        replacement.generation,
                        1,
                    ))),
                    pid if pid == unrelated.pid => Ok(Some(ProcessRecord::new(
                        unrelated.pid,
                        root.pid,
                        unrelated.generation,
                        1,
                    ))),
                    _ => Ok(None),
                },
                |pid, children| {
                    assert_eq!(pid, root.pid);
                    child_lists.set(child_lists.get() + 1);
                    children[0] = unrelated.pid;
                    Ok(1)
                },
                |identity| observed.borrow_mut().push(identity),
            )
        },
    )
    .expect("root reuse during fallback scanning is a vanished ownership edge");

    assert!(
        scan.identities.is_empty(),
        "fallback scanning must remain bound to the prevalidated root generation"
    );
    assert_eq!(child_lists.get(), 0);
    assert!(observed.borrow().is_empty());
}

#[test]
fn macos_fallback_fail_closed_boundary_matrix() {
    let root = ProcessIdentity {
        pid: 30,
        generation: 30_000,
    };
    let child = ProcessIdentity {
        pid: 31,
        generation: 31_000,
    };

    let vanished_scans = std::cell::Cell::new(0_usize);
    let vanished = macos_fallback_descendants_with(
        root,
        Instant::now() + Duration::from_secs(1),
        |_| Ok(None),
        |_, _| {
            vanished_scans.set(vanished_scans.get() + 1);
            Ok(ProcessScan {
                identities: vec![child],
                #[cfg(all(test, target_os = "linux"))]
                edge_visits: 1,
            })
        },
    )
    .expect("a vanished root is an absent ownership edge");
    assert!(vanished.identities.is_empty());
    assert_eq!(vanished_scans.get(), 0);

    let failed_scans = std::cell::Cell::new(0_usize);
    let identity_error = macos_fallback_descendants_with(
        root,
        Instant::now() + Duration::from_secs(1),
        |_| Err(process_scan_resource_error()),
        |_, _| {
            failed_scans.set(failed_scans.get() + 1);
            Ok(ProcessScan {
                identities: vec![child],
                #[cfg(all(test, target_os = "linux"))]
                edge_visits: 1,
            })
        },
    )
    .expect_err("identity read failures must remain containment failures");
    assert_eq!(
        identity_error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed identity failure")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
    assert_eq!(failed_scans.get(), 0);

    let expired_identity_reads = std::cell::Cell::new(0_usize);
    let expired_scans = std::cell::Cell::new(0_usize);
    let expired = macos_fallback_descendants_with(
        root,
        Instant::now() - Duration::from_secs(1),
        |_| {
            expired_identity_reads.set(expired_identity_reads.get() + 1);
            Ok(Some(root))
        },
        |_, _| {
            expired_scans.set(expired_scans.get() + 1);
            Ok(ProcessScan {
                identities: vec![child],
                #[cfg(all(test, target_os = "linux"))]
                edge_visits: 1,
            })
        },
    )
    .expect_err("an expired fallback deadline must fail before discovery");
    assert_eq!(
        expired
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed fallback timeout")
            .kind(),
        DirtyCheckoutErrorKind::Timeout
    );
    assert_eq!(expired_identity_reads.get(), 0);
    assert_eq!(expired_scans.get(), 0);

    let delegated_scans = std::cell::Cell::new(0_usize);
    let delegated = macos_fallback_descendants_with(
        root,
        Instant::now() + Duration::from_secs(1),
        |_| Ok(Some(root)),
        |_, _| {
            delegated_scans.set(delegated_scans.get() + 1);
            Ok(ProcessScan {
                identities: vec![child],
                #[cfg(all(test, target_os = "linux"))]
                edge_visits: 1,
            })
        },
    )
    .expect("an exact live root delegates discovery");
    assert_eq!(delegated.identities, vec![child]);
    assert_eq!(delegated_scans.get(), 1);

    let scanner_error = macos_fallback_descendants_with(
        root,
        Instant::now() + Duration::from_secs(1),
        |_| Ok(Some(root)),
        |_, _| Err(process_scan_resource_error()),
    )
    .expect_err("scanner containment failures must propagate");
    assert_eq!(
        scanner_error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed scanner failure")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
}

#[test]
fn macos_event_reducer_retains_reparented_child_after_parent_reap() {
    let root = ProcessIdentity {
        pid: 10,
        generation: 1_000,
    };
    let parent = ProcessIdentity {
        pid: 11,
        generation: 1_100,
    };
    let leaf = ProcessIdentity {
        pid: 12,
        generation: 1_200,
    };
    let mut tracked = std::collections::HashMap::from([(parent.pid, parent)]);
    let mut pending = std::collections::HashMap::new();

    reduce_macos_process_event_with(
        root,
        &mut tracked,
        &mut pending,
        MacosProcessEvent::Exit { pid: parent.pid },
        |_| Ok(None),
    )
    .expect("parent exit reduction succeeds");
    reduce_macos_process_event_with(
        root,
        &mut tracked,
        &mut pending,
        MacosProcessEvent::Child { pid: leaf.pid },
        |pid| Ok(Some(ProcessRecord::new(pid, 1, leaf.generation, 0))),
    )
    .expect("reparented child event reduction succeeds");
    assert_eq!(pending.get(&leaf.pid), Some(&leaf));
    assert!(!tracked.contains_key(&leaf.pid));

    reduce_macos_process_event_with(
        root,
        &mut tracked,
        &mut pending,
        MacosProcessEvent::Drained,
        |pid| Ok(Some(ProcessRecord::new(pid, 1, leaf.generation, 0))),
    )
    .expect("drained child generation revalidation succeeds");
    assert_eq!(
        tracked.get(&leaf.pid),
        Some(&leaf),
        "NOTE_TRACK ancestry must survive rapid parent reap and child reparenting"
    );
    assert!(pending.is_empty());
}

#[test]
fn macos_event_reducer_rejects_reused_child_before_drain() {
    let root = ProcessIdentity {
        pid: 10,
        generation: 1_000,
    };
    let child = ProcessIdentity {
        pid: 11,
        generation: 1_100,
    };
    let replacement = ProcessIdentity {
        pid: child.pid,
        generation: 9_999,
    };
    let mut tracked = std::collections::HashMap::new();
    let mut pending = std::collections::HashMap::new();

    reduce_macos_process_event_with(
        root,
        &mut tracked,
        &mut pending,
        MacosProcessEvent::Child { pid: child.pid },
        |pid| Ok(Some(ProcessRecord::new(pid, root.pid, child.generation, 0))),
    )
    .expect("child event stages an exact generation");
    reduce_macos_process_event_with(
        root,
        &mut tracked,
        &mut pending,
        MacosProcessEvent::Exit { pid: child.pid },
        |pid| Ok(Some(ProcessRecord::new(pid, 1, replacement.generation, 0))),
    )
    .expect("exit cancels the pending generation");
    reduce_macos_process_event_with(
        root,
        &mut tracked,
        &mut pending,
        MacosProcessEvent::Drained,
        |pid| Ok(Some(ProcessRecord::new(pid, 1, replacement.generation, 0))),
    )
    .expect("drain after replacement succeeds");

    assert!(tracked.is_empty(), "the replacement PID is never published");
    assert!(pending.is_empty());
}

#[test]
fn macos_event_reducer_discards_generation_change_before_drain() {
    let root = ProcessIdentity {
        pid: 10,
        generation: 1_000,
    };
    let child = ProcessIdentity {
        pid: 11,
        generation: 1_100,
    };
    let replacement = ProcessIdentity {
        pid: child.pid,
        generation: 9_999,
    };
    let mut tracked = std::collections::HashMap::new();
    let mut pending = std::collections::HashMap::new();

    reduce_macos_process_event_with(
        root,
        &mut tracked,
        &mut pending,
        MacosProcessEvent::Child { pid: child.pid },
        |pid| Ok(Some(ProcessRecord::new(pid, root.pid, child.generation, 0))),
    )
    .expect("child event stages generation A");
    reduce_macos_process_event_with(
        root,
        &mut tracked,
        &mut pending,
        MacosProcessEvent::Drained,
        |pid| Ok(Some(ProcessRecord::new(pid, 1, replacement.generation, 0))),
    )
    .expect("drain generation comparison succeeds");

    assert!(tracked.is_empty(), "neither generation is published");
    assert!(pending.is_empty());
}

#[test]
fn macos_event_reducer_stale_exit_preserves_tracked_replacement() {
    let root = ProcessIdentity {
        pid: 10,
        generation: 1_000,
    };
    let replacement = ProcessIdentity {
        pid: 11,
        generation: 9_999,
    };
    let mut tracked = std::collections::HashMap::from([(replacement.pid, replacement)]);
    let mut pending = std::collections::HashMap::new();

    reduce_macos_process_event_with(
        root,
        &mut tracked,
        &mut pending,
        MacosProcessEvent::Exit {
            pid: replacement.pid,
        },
        |pid| {
            Ok(Some(ProcessRecord::new(
                pid,
                root.pid,
                replacement.generation,
                0,
            )))
        },
    )
    .expect("stale exit event reduction succeeds");
    assert_eq!(
        tracked.get(&replacement.pid),
        Some(&replacement),
        "an old exit event must not erase the current tracked generation"
    );
}

#[test]
fn macos_child_resolution_discards_edges_when_parent_generation_changes() {
    let parent = ProcessIdentity {
        pid: 10,
        generation: 1_000,
    };
    let replacement = ProcessIdentity {
        pid: parent.pid,
        generation: 9_999,
    };
    let validations = std::cell::Cell::new(0_u8);
    let mut observed = Vec::new();

    visit_identity_bound_children_with(
        parent,
        [Ok(11)],
        |pid| {
            if pid == parent.pid {
                let validation = validations.get();
                validations.set(validation + 1);
                let generation = if validation == 0 {
                    parent.generation
                } else {
                    replacement.generation
                };
                Ok(Some(ProcessRecord::new(pid, 1, generation, 0)))
            } else {
                Ok(Some(ProcessRecord::new(pid, parent.pid, 1_100, 0)))
            }
        },
        |identity| {
            observed.push(identity);
            Ok(())
        },
    )
    .expect("generation validation must remain available");

    assert!(
        observed.is_empty(),
        "children resolved across a parent generation change must be discarded"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_targeted_reader_revalidates_parent_generation_before_following_its_children() {
    fn write_process(
        root: &Path,
        pid: libc::pid_t,
        parent: libc::pid_t,
        generation: u128,
        children: &str,
    ) {
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
        stat.extend_from_slice(format!(" {generation}\n").as_bytes());
        fs::write(root.join(pid.to_string()).join("stat"), stat).expect("write synthetic stat");
    }

    let proc_root = tempfile::TempDir::new().expect("synthetic proc root");
    write_process(proc_root.path(), 10, 1, 1_000, "11\n");
    write_process(proc_root.path(), 11, 10, 1_100, "12\n");
    write_process(proc_root.path(), 12, 11, 1_200, "");
    let mut observed = Vec::new();

    let scan = descendant_processes_at(
        proc_root.path(),
        10,
        Instant::now() + Duration::from_secs(1),
        ProcessScanLimits {
            process_entries: 8,
            metadata_bytes: 8 * 1024,
            descendants: 4,
        },
        |identity| {
            observed.push(identity);
            if identity.pid == 11 {
                write_process(proc_root.path(), 11, 99, 9_999, "12\n");
            }
        },
    )
    .expect("a replaced frontier parent must be discarded rather than followed");

    assert_eq!(
        scan.identities
            .iter()
            .map(|identity| identity.pid)
            .collect::<Vec<_>>(),
        vec![11],
        "the replacement generation's child edge must never become owned"
    );
    assert_eq!(observed, scan.identities);
}
