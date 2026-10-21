use super::*;

#[test]
fn adoption_reuses_one_deadline_across_snapshot_barriers() {
    let (checkout, state_root, reason_path, snapshot, _challenge_path) =
        adoption_transaction_fixture();
    let checkout_root =
        fs::canonicalize(checkout.path()).expect("canonicalize checkout fixture root");
    let state_root_path =
        fs::canonicalize(state_root.path()).expect("canonicalize state fixture root");
    let checkout_dir = state_root_path
        .join(&snapshot.repository_key)
        .join(&snapshot.checkout_key);
    let transaction_deadline = deadline_after(SNAPSHOT_TIMEOUT).expect("transaction deadline");
    let mut observed_deadlines = Vec::new();

    adopt_dirty_with_snapshot_clock_and_deadline(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
        |_, barrier_deadline| {
            observed_deadlines.push(barrier_deadline);
            Ok(snapshot.clone())
        },
        unix_time,
        transaction_deadline,
    )
    .expect("stable snapshot barriers must complete before the deadline");

    assert_eq!(
        observed_deadlines.len(),
        4,
        "adoption must execute every snapshot barrier"
    );
    assert!(
        observed_deadlines
            .iter()
            .all(|deadline| *deadline == transaction_deadline),
        "every snapshot barrier must receive the original transaction deadline"
    );
    assert!(
        checkout_dir.join("lease.json").exists(),
        "deadline observation must complete a real adoption transition"
    );
}

#[test]
fn lease_lock_waits_for_active_transaction_until_shared_deadline() {
    let legacy_lock_wait = Duration::from_secs(2);
    let state_dir = tempfile::TempDir::new().expect("lease-lock state directory");
    let held_lock = LeaseLock::acquire_until(
        state_dir.path(),
        deadline_after(TEST_WAIT).expect("initial lock deadline"),
    )
    .expect("acquire initial lease lock");
    let worker_directory = state_dir.path().to_path_buf();
    let (waiting_sender, waiting_receiver) = std::sync::mpsc::sync_channel(1);

    let worker = std::thread::spawn(move || {
        let mut notified = false;
        LeaseLock::acquire_until_with(
            &worker_directory,
            deadline_after(legacy_lock_wait + TEST_WAIT).expect("contending lock deadline"),
            || {
                if !notified {
                    waiting_sender
                        .send(())
                        .expect("report real lease-lock contention");
                    notified = true;
                }
            },
        )
    });

    waiting_receiver
        .recv_timeout(TEST_WAIT)
        .expect("contending worker must reach the real lock boundary");
    std::thread::sleep(legacy_lock_wait + LOCK_POLL);
    drop(held_lock);

    let result = worker.join().expect("contending lease-lock worker");
    if let Err(error) = result {
        panic!(
            "an active transaction must be allowed to finish within the shared deadline: {error}"
        );
    }
}

#[test]
fn lease_lock_times_out_at_shared_transaction_deadline() {
    let state_dir = tempfile::TempDir::new().expect("lease-lock state directory");
    let held_lock = LeaseLock::acquire_until(
        state_dir.path(),
        deadline_after(TEST_WAIT).expect("initial lock deadline"),
    )
    .expect("acquire initial lease lock");

    let error = match LeaseLock::acquire_until(
        state_dir.path(),
        deadline_after(Duration::from_millis(100)).expect("contending lock deadline"),
    ) {
        Ok(_) => panic!("contending lock acquisition must honor the shared deadline"),
        Err(error) => error,
    };

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed lease-lock timeout")
            .kind(),
        DirtyCheckoutErrorKind::Timeout
    );
    drop(held_lock);
}

#[test]
fn adoption_boundary_rechecks_expiry_and_git_operation_markers() {
    let root = tempfile::TempDir::new().expect("boundary root");
    let checkout = root.path().join("checkout");
    let git_dir = root.path().join("git-dir");
    fs::create_dir(&checkout).expect("create checkout");
    fs::create_dir(&git_dir).expect("create git dir");
    let identity = CheckoutIdentity {
        root: checkout,
        git_dir: git_dir.clone(),
        common_dir: git_dir.clone(),
        repository_key: "a".repeat(64),
        checkout_key: "b".repeat(64),
        checkout_instance: "c".repeat(32),
    };
    let challenge = ChallengeRecord {
        schema: CHALLENGE_SCHEMA.to_string(),
        token_digest: "d".repeat(64),
        session_key: "e".repeat(64),
        repository_key: identity.repository_key.clone(),
        checkout_key: identity.checkout_key.clone(),
        checkout_instance: identity.checkout_instance.clone(),
        snapshot_id: "f".repeat(64),
        head_oid: "1".repeat(40),
        branch_ref_digest: "2".repeat(64),
        authorization_turn_digest: "3".repeat(64),
        issued_at: 100,
        expires_at: 101,
    };

    validate_adoption_boundary(&challenge, &identity, 100)
        .expect("challenge starts valid at the final boundary");
    let expired = validate_adoption_boundary(&challenge, &identity, 101)
        .expect_err("challenge must be rechecked at transition time");
    assert_eq!(
        expired
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed expiry")
            .kind(),
        DirtyCheckoutErrorKind::ChallengeExpired
    );

    let marker = validate_adoption_precommit_with(
        &challenge,
        &identity,
        || {
            fs::write(git_dir.join("index.lock"), b"")
                .context("write operation marker at precommit barrier")?;
            Ok(())
        },
        || Ok(100),
    )
    .expect_err("operation marker introduced at the barrier must fail closed");
    assert_eq!(
        marker
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed operation error")
            .kind(),
        DirtyCheckoutErrorKind::UnsupportedGitState
    );
    fs::remove_file(git_dir.join("index.lock")).expect("remove operation marker");

    let expired_at_barrier =
        validate_adoption_precommit_with(&challenge, &identity, || Ok(()), || Ok(101))
            .expect_err("challenge expiring at the precommit barrier must fail closed");
    assert_eq!(
        expired_at_barrier
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed barrier expiry")
            .kind(),
        DirtyCheckoutErrorKind::ChallengeExpired
    );
}
