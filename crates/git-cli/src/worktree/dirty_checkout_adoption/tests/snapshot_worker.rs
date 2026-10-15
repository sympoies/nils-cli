use super::*;

#[test]
fn snapshot_worker_success_rejects_semantically_invalid_identity_and_budgets() {
    let valid = serde_json::json!({
        "schema": SNAPSHOT_SCHEMA,
        "repository_key": "a".repeat(64),
        "checkout_key": "b".repeat(64),
        "checkout_instance": "c".repeat(32),
        "snapshot_id": "d".repeat(64),
        "head_oid": "e".repeat(40),
        "branch_ref_digest": "f".repeat(64),
        "tracked_entries": 1,
        "untracked_entries": 1,
        "hashed_bytes": 2,
    });
    let mut cases = Vec::new();
    for (label, field, value) in [
        ("malformed repository key", "repository_key", "a".repeat(63)),
        ("uppercase checkout key", "checkout_key", "B".repeat(64)),
        (
            "malformed checkout instance",
            "checkout_instance",
            "not-an-instance".to_string(),
        ),
        ("uppercase snapshot digest", "snapshot_id", "D".repeat(64)),
        (
            "malformed branch digest",
            "branch_ref_digest",
            "not-a-digest".to_string(),
        ),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = serde_json::json!(value);
        cases.push((label, invalid));
    }
    let mut invalid_head = valid.clone();
    invalid_head["head_oid"] = serde_json::json!("not-a-head");
    cases.push(("malformed HEAD", invalid_head));
    let mut invalid_entries = valid.clone();
    invalid_entries["tracked_entries"] = serde_json::json!(MAX_ENTRY_COUNT);
    invalid_entries["untracked_entries"] = serde_json::json!(1);
    cases.push(("combined entry budget", invalid_entries));
    let mut invalid_bytes = valid;
    invalid_bytes["hashed_bytes"] = serde_json::json!(MAX_TOTAL_BYTES + 1);
    cases.push(("hashed byte budget", invalid_bytes));

    for (label, snapshot) in cases {
        let error = decode_snapshot_worker_output(successful_worker_output(snapshot))
            .expect_err("semantically invalid worker success must be rejected");
        assert_eq!(
            error
                .downcast_ref::<DirtyCheckoutError>()
                .expect("typed invalid worker response")
                .kind(),
            DirtyCheckoutErrorKind::SnapshotFailed,
            "{label}"
        );
    }
}

#[test]
fn snapshot_worker_executable_rejects_group_and_world_writable_files() {
    let root = tempfile::TempDir::new().expect("worker executable root");
    let executable = root.path().join("git-cli");
    fs::write(&executable, b"worker").expect("write worker fixture");

    for (mode, trusted) in [(0o700, true), (0o755, true), (0o775, false), (0o702, false)] {
        fs::set_permissions(&executable, fs::Permissions::from_mode(mode))
            .expect("set worker fixture mode");
        let metadata = fs::metadata(&executable).expect("worker fixture metadata");
        assert_eq!(
            validate_worker_file_metadata(&metadata).is_ok(),
            trusted,
            "unexpected worker trust result for mode {mode:o}"
        );
    }
}

/// Removes a file on drop so a shared-fixture copy is cleaned up even when
/// an assertion panics mid-test.
#[cfg(any(target_os = "linux", target_os = "macos"))]
struct RemoveOnDrop(PathBuf);

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// A group-writable, effective-user-owned source binary that forces the
/// hardened private-copy path regardless of the ambient umask.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn writable_worker_source(payload: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::TempDir::new().expect("worker source root");
    let source = root.path().join("git-cli");
    fs::write(&source, payload).expect("write worker source");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o770))
        .expect("make worker source group writable");
    (root, source)
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn private_test_worker_executable_reuses_a_single_stable_copy() {
    let payload = b"fake git-cli worker payload";
    let (_root, source) = writable_worker_source(payload);

    let first =
        private_test_worker_executable(source.clone()).expect("first private worker install");
    let _cleanup = RemoveOnDrop(first.clone());
    let installed_ino = fs::metadata(&first).expect("installed copy metadata").ino();

    // Reuse must not re-copy the ~17 MB binary: the path stays stable AND the
    // on-disk inode never changes across calls (a re-install-every-call
    // regression — the exact #1323 cost — would change the inode).
    let second =
        private_test_worker_executable(source.clone()).expect("second private worker install");
    let third =
        private_test_worker_executable(source.clone()).expect("third private worker install");
    assert_eq!(first, second, "the private worker copy path must be stable");
    assert_eq!(first, third, "reuse must survive across repeated calls");
    assert_eq!(
        fs::metadata(&second).expect("reused copy metadata").ino(),
        installed_ino,
        "reuse must not re-install the copy on every call"
    );

    // The copy is a faithful, hardened stand-in for the source binary.
    let metadata = fs::metadata(&first).expect("private worker copy metadata");
    assert_eq!(
        metadata.mode() & 0o777,
        0o500,
        "the private copy must be hardened to 0o500"
    );
    assert_eq!(
        metadata.uid(),
        unsafe { libc::geteuid() },
        "the private copy must be owned by the effective user"
    );
    assert_eq!(
        fs::read(&first).expect("read private worker copy"),
        payload,
        "the private copy content must match the source binary"
    );

    // No accumulation: exactly one copy exists for this source binary.
    let name = private_test_worker_name(&source);
    let directory = private_test_worker_dir().expect("private worker directory");
    assert_eq!(
        first,
        directory.join(&name),
        "the copy must live at the derived stable path"
    );
    let matching = fs::read_dir(&directory)
        .expect("read private worker directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy() == name)
        .count();
    assert_eq!(
        matching, 1,
        "only one private copy may exist per source binary"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn private_test_worker_executable_adopts_an_existing_current_copy() {
    // Cross-process/cross-run reuse depends on the slow-path *adopt* branch,
    // not the in-memory fast path. Install a copy, evict the single-slot
    // process cache with a second source, then re-request the first source so
    // it misses the cache and must adopt the still-current on-disk copy.
    let payload = b"adoptable git-cli worker payload";
    let (_root, source) = writable_worker_source(payload);
    let first =
        private_test_worker_executable(source.clone()).expect("initial private worker install");
    let _cleanup = RemoveOnDrop(first.clone());
    let installed_ino = fs::metadata(&first).expect("installed copy metadata").ino();

    let (_evict_root, evict_source) = writable_worker_source(b"cache-evicting source");
    let evicted = private_test_worker_executable(evict_source.clone())
        .expect("evicting private worker install");
    let _evict_cleanup = RemoveOnDrop(evicted);

    let adopted =
        private_test_worker_executable(source.clone()).expect("adopt existing private worker copy");
    assert_eq!(
        first, adopted,
        "adoption must resolve to the same stable path"
    );
    assert_eq!(
        fs::metadata(&adopted).expect("adopted copy metadata").ino(),
        installed_ino,
        "a current on-disk copy must be adopted, not re-installed"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn private_test_worker_executable_reinstalls_a_stale_copy() {
    let payload = b"authoritative git-cli worker payload";
    let (_root, source) = writable_worker_source(payload);
    let name = private_test_worker_name(&source);
    let directory = private_test_worker_dir().expect("private worker directory");
    let destination = directory.join(&name);
    let _cleanup = RemoveOnDrop(destination.clone());

    // Pre-seed a stale copy (different content/length) at the derived path.
    fs::write(&destination, b"stale").expect("write stale worker copy");
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o500))
        .expect("harden stale worker copy");
    let stale_ino = fs::metadata(&destination)
        .expect("stale copy metadata")
        .ino();

    let resolved = private_test_worker_executable(source.clone())
        .expect("reinstall stale private worker copy");
    assert_eq!(
        resolved, destination,
        "reinstall must reuse the derived path"
    );
    assert_eq!(
        fs::read(&destination).expect("read reinstalled copy"),
        payload,
        "a stale copy must be reinstalled with the current binary's content"
    );
    let reinstalled = fs::metadata(&destination).expect("reinstalled copy metadata");
    assert_eq!(
        reinstalled.mode() & 0o777,
        0o500,
        "the reinstalled copy must be hardened to 0o500"
    );
    assert_ne!(
        reinstalled.ino(),
        stale_ino,
        "reinstall must atomically replace the stale copy"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn prune_stale_private_test_worker_staging_removes_only_aged_staging_files() {
    use std::fs::FileTimes;

    let directory = private_test_worker_dir().expect("private worker directory");
    // Distinctive names so this never collides with concurrent installs
    // (whose staging files use random `tempfile` names) or their copies.
    let fresh = directory.join(".git-cli.prune-fresh-fixture.tmp");
    let aged = directory.join(".git-cli.prune-aged-fixture.tmp");
    let published = directory.join("git-cli.prune-published-fixture");
    let _fresh_cleanup = RemoveOnDrop(fresh.clone());
    let _aged_cleanup = RemoveOnDrop(aged.clone());
    let _published_cleanup = RemoveOnDrop(published.clone());

    fs::write(&fresh, b"fresh").expect("write fresh staging fixture");
    fs::write(&published, b"published").expect("write published fixture");
    fs::write(&aged, b"aged").expect("write aged staging fixture");
    let backdated = SystemTime::now() - PRIVATE_TEST_WORKER_STAGING_TTL - Duration::from_secs(60);
    OpenOptions::new()
        .write(true)
        .open(&aged)
        .expect("open aged staging fixture")
        .set_times(FileTimes::new().set_modified(backdated))
        .expect("backdate aged staging fixture");

    prune_stale_private_test_worker_staging(&directory);

    assert!(
        fresh.exists(),
        "a recent staging file must be left in place"
    );
    assert!(
        !aged.exists(),
        "a staging file older than the TTL must be removed"
    );
    assert!(
        published.exists(),
        "a published `git-cli.<hash>` copy must never be pruned"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn private_test_worker_executable_uses_a_trusted_source_in_place() {
    let root = tempfile::TempDir::new().expect("worker source root");
    let source = root.path().join("git-cli");
    fs::write(&source, b"trusted worker").expect("write worker source");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755))
        .expect("set non-writable worker mode");

    let resolved = private_test_worker_executable(source.clone())
        .expect("a non-writable source needs no private copy");
    assert_eq!(
        resolved, source,
        "a source that is neither group- nor other-writable is used in place"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn descriptor_backed_worker(path: &Path) -> SnapshotWorkerExecutable {
    let file = File::open(path).expect("open worker fixture descriptor");
    let metadata = file.metadata().expect("read worker fixture metadata");
    let digest = worker_file_digest(&file, metadata.len()).expect("hash worker fixture");
    let command_path = if cfg!(target_os = "linux") {
        PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
    } else {
        PathBuf::from(format!("/dev/fd/{}", file.as_raw_fd()))
    };
    SnapshotWorkerExecutable {
        source_path: path.to_path_buf(),
        command_path,
        file,
        metadata,
        digest,
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn descriptor_backed_worker_accepts_owner_controlled_group_writable_ancestry() {
    let root = tempfile::Builder::new()
        .prefix("worker-ancestor-")
        .tempdir_in(env::current_dir().expect("current test directory"))
        .expect("worker ancestor root");
    let executable = root.path().join("git-cli");
    fs::write(&executable, b"worker").expect("write worker fixture");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("set worker mode");
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o775))
        .expect("make owner-controlled ancestor group writable");

    descriptor_backed_worker(&executable)
        .revalidate()
        .expect("descriptor-backed launch must not depend on mutable ancestor path modes");
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn descriptor_backed_worker_remains_bound_after_path_replacement() {
    let root = tempfile::TempDir::new().expect("worker replacement root");
    let executable = root.path().join("git-cli");
    let displaced = root.path().join("git-cli.original");
    fs::write(&executable, b"trusted worker").expect("write worker fixture");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).expect("set worker mode");
    let worker = descriptor_backed_worker(&executable);

    fs::rename(&executable, &displaced).expect("displace worker path");
    fs::write(&executable, b"replacement worker").expect("replace worker path");
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
        .expect("set replacement mode");

    worker
        .revalidate()
        .expect("held descriptor must remain bound to the originally validated worker inode");
    let held_metadata = worker.file.metadata().expect("held worker metadata");
    assert!(same_worker_descriptor_metadata(
        &worker.metadata,
        &held_metadata
    ));
    assert_eq!(
        worker_file_digest(&worker.file, held_metadata.len()).expect("held worker content digest"),
        worker.digest
    );
    assert!(!same_metadata(
        &worker.metadata,
        &fs::metadata(&executable).expect("replacement path metadata")
    ));
}
