use super::*;

#[test]
fn crash_after_lease_staging_rechecks_authorization_before_commit_recovery() {
    let (checkout, state_root, reason_path, snapshot, challenge_path) =
        adoption_transaction_fixture();
    let checkout_root =
        fs::canonicalize(checkout.path()).expect("canonicalize checkout fixture root");
    let state_root_path =
        fs::canonicalize(state_root.path()).expect("canonicalize state fixture root");
    let challenge: ChallengeRecord =
        serde_json::from_slice(&fs::read(&challenge_path).expect("read crash challenge"))
            .expect("parse crash challenge");
    let checkout_dir = state_root_path
        .join(&snapshot.repository_key)
        .join(&snapshot.checkout_key);
    let mut snapshot_calls = 0;

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = adopt_dirty_with_snapshot_and_clock(
            &checkout_root,
            &state_root_path,
            &"a".repeat(64),
            &reason_path,
            |path| {
                snapshot_calls += 1;
                if snapshot_calls == 4 {
                    panic!("injected crash after durable lease staging");
                }
                dirty_snapshot(path)
            },
            || Ok(challenge.issued_at),
        );
    }));
    assert!(crashed.is_err(), "fault injection must interrupt adoption");
    assert!(
        !checkout_dir.join("lease.json").exists(),
        "crash before barrier four must not publish authoritative authority"
    );
    assert_eq!(
        fs::read_dir(&checkout_dir)
            .expect("list staged crash state")
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().as_bytes().starts_with(b".pending-lease-"))
            .count(),
        1,
        "crash must retain one private staged lease"
    );
    assert!(
        fs::read_dir(&checkout_dir)
            .expect("list pending crash state")
            .filter_map(|entry| entry.ok())
            .any(|entry| entry
                .file_name()
                .as_bytes()
                .starts_with(b".pending-adoption-")),
        "crash must retain provisional pending state"
    );

    let error = adopt_dirty_with_snapshot_and_clock(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
        dirty_snapshot,
        || Ok(challenge.expires_at),
    )
    .expect_err("expired provisional authorization must not recover as committed");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed crash-recovery expiry")
            .kind(),
        DirtyCheckoutErrorKind::ChallengeExpired
    );
    assert!(
        !checkout_dir.join("lease.json").exists(),
        "unverified provisional lease must be revoked"
    );
}

#[test]
fn staged_retry_preserves_a_newer_authoritative_foreign_lease() {
    let (checkout, state_root, reason_path, snapshot, challenge_path) =
        adoption_transaction_fixture();
    let checkout_root =
        fs::canonicalize(checkout.path()).expect("canonicalize checkout fixture root");
    let state_root_path =
        fs::canonicalize(state_root.path()).expect("canonicalize state fixture root");
    let identity = resolve_checkout(
        &checkout_root,
        false,
        deadline_after(SNAPSHOT_TIMEOUT).expect("identity deadline"),
    )
    .expect("resolve checkout identity");
    let challenge: ChallengeRecord =
        serde_json::from_slice(&fs::read(&challenge_path).expect("read crash challenge"))
            .expect("parse crash challenge");
    let checkout_dir = state_root_path
        .join(&snapshot.repository_key)
        .join(&snapshot.checkout_key);
    let lease_path = checkout_dir.join("lease.json");
    let mut snapshot_calls = 0;

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = adopt_dirty_with_snapshot_and_clock(
            &checkout_root,
            &state_root_path,
            &"a".repeat(64),
            &reason_path,
            |path| {
                snapshot_calls += 1;
                if snapshot_calls == 4 {
                    panic!("injected crash after durable lease staging");
                }
                dirty_snapshot(path)
            },
            || Ok(challenge.issued_at),
        );
    }));
    assert!(crashed.is_err(), "fault injection must interrupt adoption");

    let foreign = LeaseRecord::V1(LeaseV1Wire {
        schema: LEASE_V1_SCHEMA.to_string(),
        session_key: "b".repeat(64),
        checkout_instance: identity.checkout_instance.clone(),
        checkout_root: identity.root.to_string_lossy().into_owned(),
        checkout_git_dir: identity.git_dir.to_string_lossy().into_owned(),
        acquired_at: challenge.issued_at,
        refreshed_at: challenge.issued_at,
        expires_at: challenge.expires_at.saturating_add(3_600),
    });
    let mut foreign_bytes =
        serde_json::to_vec_pretty(&foreign).expect("serialize newer foreign lease");
    foreign_bytes.push(b'\n');
    fs::write(&lease_path, &foreign_bytes).expect("write newer foreign lease");
    fs::set_permissions(&lease_path, fs::Permissions::from_mode(0o600))
        .expect("make newer foreign lease private");
    sync_directory(&checkout_dir).expect("make newer foreign lease durable");

    let error = adopt_dirty_with_snapshot_and_clock(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
        dirty_snapshot,
        || Ok(challenge.issued_at),
    )
    .expect_err("staged retry must not overwrite a newer foreign lease");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed foreign staged-retry conflict")
            .kind(),
        DirtyCheckoutErrorKind::ForeignLease
    );
    assert_eq!(
        fs::read(&lease_path).expect("read preserved foreign lease"),
        foreign_bytes,
        "staged retry must preserve the exact newer foreign lease bytes"
    );
    assert!(
        fs::read_dir(&checkout_dir)
            .expect("list retained staged state")
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.file_name().as_bytes().starts_with(b".pending-lease-")),
        "foreign conflict must retain staged recovery evidence"
    );
}

#[test]
fn committed_retry_requires_lease_to_remain_active_through_snapshot() {
    let (checkout, state_root, reason_path, snapshot, _challenge_path) =
        adoption_transaction_fixture();
    let checkout_root =
        fs::canonicalize(checkout.path()).expect("canonicalize checkout fixture root");
    let state_root_path =
        fs::canonicalize(state_root.path()).expect("canonicalize state fixture root");
    let checkout_dir = state_root_path
        .join(&snapshot.repository_key)
        .join(&snapshot.checkout_key);

    adopt_dirty_with_snapshot_and_clock(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
        dirty_snapshot,
        unix_time,
    )
    .expect("create committed adoption");
    let lease = load_lease(&checkout_dir.join("lease.json"))
        .expect("load committed lease")
        .expect("committed lease");
    let before_expiry = lease.expires_at() - 1;
    let at_expiry = lease.expires_at();
    let clock_calls = std::cell::Cell::new(0usize);

    let error = adopt_dirty_with_snapshot_and_clock(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
        dirty_snapshot,
        || {
            let call = clock_calls.get();
            clock_calls.set(call + 1);
            Ok(if call == 0 { before_expiry } else { at_expiry })
        },
    )
    .expect_err("retry must fail when the lease expires during snapshot validation");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed committed retry expiry")
            .kind(),
        DirtyCheckoutErrorKind::ChallengeReused
    );
    assert_eq!(clock_calls.get(), 2, "retry must check lease expiry twice");
}

#[test]
fn pending_rollback_binds_expired_authoritative_lease_bytes() {
    let now = 1_000;
    let v1 = LeaseRecord::V1(LeaseV1Wire {
        schema: LEASE_V1_SCHEMA.to_string(),
        session_key: "a".repeat(64),
        checkout_instance: "b".repeat(64),
        checkout_root: "/checkout".to_string(),
        checkout_git_dir: "/checkout/.git".to_string(),
        acquired_at: 1,
        refreshed_at: 2,
        expires_at: now - 1,
    });
    let v2 = LeaseRecord::V2(Box::new(LeaseV2Wire {
        schema: LEASE_V2_SCHEMA.to_string(),
        session_key: "a".repeat(64),
        checkout_instance: "b".repeat(64),
        checkout_root: "/checkout".to_string(),
        checkout_git_dir: "/checkout/.git".to_string(),
        checkout_root_bytes: hex_bytes(OsStr::new("/checkout").as_bytes()),
        checkout_git_dir_bytes: hex_bytes(OsStr::new("/checkout/.git").as_bytes()),
        acquired_at: 1,
        refreshed_at: 2,
        expires_at: now - 1,
        adoption: AdoptionRecord {
            schema: ADOPTION_SCHEMA.to_string(),
            receipt_schema: RECEIPT_SCHEMA.to_string(),
            receipt_id: "c".repeat(64),
            snapshot_id: "d".repeat(64),
            authorization_turn_digest: "e".repeat(64),
            reason_digest: "f".repeat(64),
            adopted_at: 2,
            challenge_issued_at: 1,
            challenge_digest: "1".repeat(64),
        },
    }));

    for lease in [&v1, &v2] {
        let bytes = serde_json::to_vec(lease).expect("serialize expired predecessor");
        assert_eq!(
            preserved_predecessor_lease_bytes(Some(lease), Some(bytes.clone())),
            Some(bytes),
            "pre-publication rollback must retain the untouched expired predecessor"
        );
    }
}

#[test]
fn adoption_rollback_restores_exact_max_size_active_same_session_v1_lease() {
    let (checkout, state_root, reason_path, snapshot, _challenge_path) =
        adoption_transaction_fixture();
    let checkout_root =
        fs::canonicalize(checkout.path()).expect("canonicalize checkout fixture root");
    let state_root_path =
        fs::canonicalize(state_root.path()).expect("canonicalize state fixture root");
    let identity = resolve_checkout(
        &checkout_root,
        false,
        deadline_after(SNAPSHOT_TIMEOUT).expect("identity deadline"),
    )
    .expect("resolve checkout identity");
    let checkout_dir = state_root_path
        .join(&snapshot.repository_key)
        .join(&snapshot.checkout_key);
    let lease_path = checkout_dir.join("lease.json");
    let now = unix_time().expect("predecessor lease time");
    let predecessor = LeaseRecord::V1(LeaseV1Wire {
        schema: LEASE_V1_SCHEMA.to_string(),
        session_key: "b".repeat(64),
        checkout_instance: identity.checkout_instance.clone(),
        checkout_root: identity.root.to_string_lossy().into_owned(),
        checkout_git_dir: identity.git_dir.to_string_lossy().into_owned(),
        acquired_at: now - 10,
        refreshed_at: now,
        expires_at: now + 3_600,
    });
    let mut predecessor_bytes =
        serde_json::to_vec_pretty(&predecessor).expect("serialize exact predecessor");
    predecessor_bytes.push(b'\n');
    predecessor_bytes.resize(MAX_STATE_FILE_BYTES, b' ');
    fs::write(&lease_path, &predecessor_bytes).expect("write exact predecessor lease");
    fs::set_permissions(&lease_path, fs::Permissions::from_mode(0o600))
        .expect("make predecessor lease private");
    sync_directory(&checkout_dir).expect("make predecessor durable");
    let mut snapshot_calls = 0;

    let error = adopt_dirty_with_snapshot(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
        |path| {
            snapshot_calls += 1;
            if snapshot_calls == 4 {
                fs::write(path.join("dirty.txt"), b"rollback exact predecessor\n")
                    .expect("introduce post-install drift");
            }
            dirty_snapshot(path)
        },
    )
    .expect_err("post-install drift must roll back over the v1 predecessor");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed predecessor rollback")
            .kind(),
        DirtyCheckoutErrorKind::ChallengeDrift
    );
    assert_eq!(
        fs::read(&lease_path).expect("read restored predecessor"),
        predecessor_bytes,
        "rollback must restore the exact predecessor bytes"
    );
}

#[test]
fn adoption_recovery_scan_rejects_the_entry_cap_plus_one_before_mutation() {
    const RECOVERY_ENTRY_CAP: usize = 1_024;
    let (checkout, state_root, reason_path, snapshot, challenge_path) =
        adoption_transaction_fixture();
    let checkout_root =
        fs::canonicalize(checkout.path()).expect("canonicalize checkout fixture root");
    let state_root_path =
        fs::canonicalize(state_root.path()).expect("canonicalize state fixture root");
    let checkout_dir = state_root_path
        .join(&snapshot.repository_key)
        .join(&snapshot.checkout_key);
    for index in 0..RECOVERY_ENTRY_CAP {
        fs::write(checkout_dir.join(format!("inert-{index:04}")), b"")
            .expect("write inert recovery scan entry");
    }
    fs::write(checkout_root.join("dirty.txt"), b"stale challenge\n")
        .expect("drift challenge fixture");

    let error = adopt_dirty(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
    )
    .expect_err("oversized recovery scan must fail closed");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed recovery scan limit")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
    assert!(
        challenge_path.exists(),
        "scan refusal must not consume challenge"
    );
    assert!(
        !checkout_dir.join("receipts").exists(),
        "scan refusal must precede transition artifacts"
    );
}

#[test]
fn adoption_recovers_checkout_wide_pending_predecessor_before_another_token() {
    let (checkout, state_root, reason_path, snapshot, challenge_path) =
        adoption_transaction_fixture();
    let checkout_root =
        fs::canonicalize(checkout.path()).expect("canonicalize checkout fixture root");
    let state_root_path =
        fs::canonicalize(state_root.path()).expect("canonicalize state fixture root");
    let identity = resolve_checkout(
        &checkout_root,
        false,
        deadline_after(SNAPSHOT_TIMEOUT).expect("identity deadline"),
    )
    .expect("resolve checkout identity");
    let checkout_dir = state_root_path
        .join(&snapshot.repository_key)
        .join(&snapshot.checkout_key);
    let challenge_dir = checkout_dir.join("challenges");
    let lease_path = checkout_dir.join("lease.json");
    let now = unix_time().expect("predecessor lease time");
    let predecessor = LeaseRecord::V1(LeaseV1Wire {
        schema: LEASE_V1_SCHEMA.to_string(),
        session_key: "b".repeat(64),
        checkout_instance: identity.checkout_instance.clone(),
        checkout_root: identity.root.to_string_lossy().into_owned(),
        checkout_git_dir: identity.git_dir.to_string_lossy().into_owned(),
        acquired_at: now - 10,
        refreshed_at: now,
        expires_at: now + 3_600,
    });
    let mut predecessor_bytes =
        serde_json::to_vec_pretty(&predecessor).expect("serialize predecessor");
    predecessor_bytes.push(b'\n');
    fs::write(&lease_path, &predecessor_bytes).expect("write predecessor lease");
    fs::set_permissions(&lease_path, fs::Permissions::from_mode(0o600))
        .expect("make predecessor private");
    sync_directory(&checkout_dir).expect("make predecessor durable");
    let original_challenge: ChallengeRecord =
        serde_json::from_slice(&fs::read(&challenge_path).expect("read first challenge"))
            .expect("parse first challenge");
    let first_token_digest = sha256_hex("a".repeat(64).as_bytes());
    let mut snapshot_calls = 0;

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = adopt_dirty_with_snapshot(
            &checkout_root,
            &state_root_path,
            &"a".repeat(64),
            &reason_path,
            |path| {
                snapshot_calls += 1;
                if snapshot_calls == 4 {
                    panic!("injected crash after first-token lease staging");
                }
                dirty_snapshot(path)
            },
        );
    }));
    assert!(crashed.is_err(), "fault injection must interrupt adoption");
    let first_pending_path = pending_adoption_path(&checkout_dir, &first_token_digest);
    let pending_bytes = fs::read(&first_pending_path).expect("read first pending marker");
    let pending: PendingAdoptionRecord =
        serde_json::from_slice(&pending_bytes).expect("parse first pending marker");
    fs::rename(
        staged_lease_path(&checkout_dir, &pending.receipt_id),
        checkout_dir.join(format!(".revoked-{}.json", pending.receipt_id)),
    )
    .expect("simulate durable first-token rollback tombstone");
    sync_directory(&checkout_dir).expect("make first-token tombstone durable");

    let second_token = "d".repeat(64);
    let second_token_digest = sha256_hex(second_token.as_bytes());
    let mut second_challenge = original_challenge;
    second_challenge.token_digest = second_token_digest.clone();
    write_json_atomic(
        &challenge_dir.join(format!("{second_token_digest}.json")),
        &second_challenge,
        false,
    )
    .expect("write second challenge");
    fs::write(checkout_root.join("dirty.txt"), b"stale second challenge\n")
        .expect("drift second-token snapshot");

    let error = adopt_dirty(
        &checkout_root,
        &state_root_path,
        &second_token,
        &reason_path,
    )
    .expect_err("stale second challenge must be rejected");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed second-token drift")
            .kind(),
        DirtyCheckoutErrorKind::ChallengeDrift
    );
    assert_eq!(
        fs::read(&lease_path).expect("read recovered predecessor"),
        predecessor_bytes,
        "checkout-wide recovery must preserve the exact predecessor"
    );
    assert!(
        !first_pending_path.exists(),
        "first-token recovery must complete before second-token validation"
    );
}
