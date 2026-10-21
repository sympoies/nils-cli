use super::*;

#[test]
fn snapshot_budget_counters_accept_exact_limits_and_reject_limit_plus_one() {
    let one_byte = std::process::Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: vec![0],
        stderr: Vec::new(),
    };
    let assert_resource = |error: anyhow::Error, label: &str| {
        assert_eq!(
            error
                .downcast_ref::<DirtyCheckoutError>()
                .unwrap_or_else(|| panic!("typed {label} budget error"))
                .kind(),
            DirtyCheckoutErrorKind::ResourceUnavailable,
            "unexpected {label} budget error: {error}"
        );
    };

    let mut capture = SnapshotBudget {
        capture_bytes: MAX_CAPTURE_BYTES - 1,
        ..SnapshotBudget::default()
    };
    capture
        .charge_output(&one_byte)
        .expect("exact capture-byte limit is accepted");
    assert_resource(
        capture
            .output_limit(1)
            .expect_err("capture limit plus one must be rejected"),
        "capture",
    );

    let mut paths = SnapshotBudget {
        path_bytes: MAX_PATH_BYTES - 1,
        ..SnapshotBudget::default()
    };
    paths
        .charge_path(1)
        .expect("exact path-byte limit is accepted");
    assert_resource(
        paths
            .charge_path(1)
            .expect_err("path-byte limit plus one must be rejected"),
        "path",
    );

    let mut traversal = SnapshotBudget {
        traversal_entries: MAX_ENTRY_COUNT - 1,
        ..SnapshotBudget::default()
    };
    traversal
        .charge_traversal_entry()
        .expect("exact traversal-entry limit is accepted");
    assert_resource(
        traversal
            .charge_traversal_entry()
            .expect_err("traversal-entry limit plus one must be rejected"),
        "traversal",
    );
}

#[test]
fn nul_path_parser_enforces_entry_cap_before_owned_allocation() {
    let mut raw = Vec::new();
    for index in 0..=MAX_ENTRY_COUNT {
        raw.extend_from_slice(format!("path-{index}").as_bytes());
        raw.push(0);
    }

    parse_nul_paths(&raw).expect_err("path parser must enforce the entry cap");
}

#[test]
fn aggregate_file_budget_is_checked_before_opening_the_file() {
    const CHILD_ENV: &str = "NILS_GIT_CLI_TEST_PREOPEN_BUDGET";
    if env::var_os(CHILD_ENV).is_none() {
        let status = Command::new(env::current_exe().expect("current test executable"))
            .arg("aggregate_file_budget_is_checked_before_opening_the_file")
            .arg("--nocapture")
            .env(CHILD_ENV, "1")
            .env("RUST_TEST_THREADS", "1")
            .status()
            .expect("run isolated descriptor-budget test");
        assert!(status.success(), "isolated descriptor-budget test failed");
        return;
    }

    let root = tempfile::TempDir::new().expect("snapshot root");
    let canonical_root = fs::canonicalize(root.path()).expect("canonical snapshot root");
    let path = canonical_root.join("unopenable");
    fs::write(&path, b"x").expect("write fixture");
    let mut hasher = FramedHasher::new();
    let mut budget = SnapshotBudget {
        file_bytes: MAX_TOTAL_BYTES,
        ..SnapshotBudget::default()
    };
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits) },
        0,
        "read descriptor limit"
    );
    let original = limits;
    limits.rlim_cur = 0;
    assert_eq!(
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limits) },
        0,
        "exhaust descriptor budget"
    );

    let result = hash_worktree_object(
        &mut hasher,
        &canonical_root,
        b"unopenable",
        false,
        None,
        &mut budget,
        Instant::now() + TEST_WAIT,
    );

    assert_eq!(
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &original) },
        0,
        "restore descriptor limit"
    );
    let error = result.expect_err("aggregate budget must be checked before file open");
    assert!(
        error.to_string().contains("total bytes"),
        "unexpected pre-open failure: {error}"
    );
}
