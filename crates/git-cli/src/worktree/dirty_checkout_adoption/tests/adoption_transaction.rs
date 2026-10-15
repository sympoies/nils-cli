use super::*;

#[test]
fn adoption_rechecks_the_complete_snapshot_after_durable_preparation() {
    let (checkout, state_root, reason_path, snapshot, challenge_path) =
        adoption_transaction_fixture();
    let checkout_root =
        fs::canonicalize(checkout.path()).expect("canonicalize checkout fixture root");
    let state_root_path =
        fs::canonicalize(state_root.path()).expect("canonicalize state fixture root");
    let checkout_dir = checkout_state_dir(
        &state_root_path,
        &resolve_checkout(
            &checkout_root,
            false,
            deadline_after(SNAPSHOT_TIMEOUT).expect("identity deadline"),
        )
        .expect("resolve identity"),
    )
    .expect("resolve checkout state");
    let mut calls = 0;

    let error = adopt_dirty_with_snapshot(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
        |path| {
            calls += 1;
            if calls == 3 {
                fs::write(path.join("dirty.txt"), b"drift before acceptance\n")
                    .expect("introduce pre-acceptance drift");
            }
            dirty_snapshot(path)
        },
    )
    .expect_err("pre-acceptance snapshot drift must reject adoption");

    assert_eq!(calls, 3);
    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed pre-acceptance drift")
            .kind(),
        DirtyCheckoutErrorKind::ChallengeDrift
    );
    assert!(challenge_path.exists(), "challenge must remain reusable");
    assert!(!checkout_dir.join("lease.json").exists());
    assert!(
        fs::read_dir(checkout_dir.join("receipts"))
            .expect("list receipt directory")
            .next()
            .is_none(),
        "failed preparation must roll back transition artifacts"
    );
    assert_eq!(snapshot.snapshot_id.len(), 64);
}

#[test]
fn adoption_rolls_back_when_the_post_install_snapshot_drifts() {
    let (checkout, state_root, reason_path, snapshot, challenge_path) =
        adoption_transaction_fixture();
    let checkout_root =
        fs::canonicalize(checkout.path()).expect("canonicalize checkout fixture root");
    let state_root_path =
        fs::canonicalize(state_root.path()).expect("canonicalize state fixture root");
    let checkout_dir = state_root_path
        .join(&snapshot.repository_key)
        .join(&snapshot.checkout_key);
    let mut calls = 0;

    let error = adopt_dirty_with_snapshot(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
        |path| {
            calls += 1;
            if calls == 4 {
                fs::write(path.join("dirty.txt"), b"drift after lease install\n")
                    .expect("introduce post-install drift");
            }
            dirty_snapshot(path)
        },
    )
    .expect_err("post-install snapshot drift must roll back adoption");

    assert_eq!(calls, 4);
    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed post-install drift")
            .kind(),
        DirtyCheckoutErrorKind::ChallengeDrift
    );
    assert!(
        !challenge_path.exists(),
        "consumed challenge must stay spent"
    );
    assert!(!checkout_dir.join("lease.json").exists());
    let tombstones = fs::read_dir(&checkout_dir)
        .expect("list checkout state")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().as_bytes().starts_with(b".revoked-"))
        .count();
    assert_eq!(tombstones, 1, "rollback must retain one tombstone");
    assert!(
        fs::read_dir(checkout_dir.join("receipts"))
            .expect("list receipt directory")
            .next()
            .is_none(),
        "revoked recovery must remove receipt and spent challenge"
    );

    let replay = adopt_dirty(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
    )
    .expect_err("rolled-back challenge must not be replayable");
    assert_eq!(
        replay
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed challenge replay")
            .kind(),
        DirtyCheckoutErrorKind::ChallengeReused
    );
}

#[test]
fn post_install_rollback_prunes_revocation_tombstones() {
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
    let mut receipt_ids = Vec::new();
    for index in 0..MAX_REVOCATION_TOMBSTONES {
        let receipt_id = format!("{index:064x}");
        let adopted_at = index as u64 + 1;
        let lease = LeaseRecord::V2(Box::new(LeaseV2Wire {
            schema: LEASE_V2_SCHEMA.to_string(),
            session_key: "d".repeat(64),
            checkout_instance: identity.checkout_instance.clone(),
            checkout_root: identity.root.to_string_lossy().into_owned(),
            checkout_git_dir: identity.git_dir.to_string_lossy().into_owned(),
            checkout_root_bytes: hex_bytes(identity.root.as_os_str().as_bytes()),
            checkout_git_dir_bytes: hex_bytes(identity.git_dir.as_os_str().as_bytes()),
            acquired_at: adopted_at,
            refreshed_at: adopted_at,
            expires_at: adopted_at + 100,
            adoption: AdoptionRecord {
                schema: ADOPTION_SCHEMA.to_string(),
                receipt_schema: RECEIPT_SCHEMA.to_string(),
                receipt_id: receipt_id.clone(),
                snapshot_id: "e".repeat(64),
                authorization_turn_digest: "f".repeat(64),
                reason_digest: "1".repeat(64),
                adopted_at,
                challenge_issued_at: adopted_at - 1,
                challenge_digest: "2".repeat(64),
            },
        }));
        write_json_atomic(
            &checkout_dir.join(format!(".revoked-{receipt_id}.json")),
            &lease,
            false,
        )
        .expect("write rollback tombstone fixture");
        receipt_ids.push(receipt_id);
    }
    let mut calls = 0;

    adopt_dirty_with_snapshot(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
        |path| {
            calls += 1;
            if calls == 4 {
                fs::write(path.join("dirty.txt"), b"drift for tombstone pruning\n")
                    .expect("introduce post-stage drift");
            }
            dirty_snapshot(path)
        },
    )
    .expect_err("post-stage drift must roll back adoption");

    let remaining = fs::read_dir(&checkout_dir)
        .expect("list rollback tombstones")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().as_bytes().starts_with(b".revoked-"))
        .count();
    assert_eq!(remaining, MAX_REVOCATION_TOMBSTONES);
    assert!(
        !checkout_dir
            .join(format!(".revoked-{}.json", receipt_ids[0]))
            .exists(),
        "oldest unprotected tombstone must be pruned"
    );
}

#[test]
fn adoption_expiry_is_checked_at_the_durable_authorization_boundary() {
    let (checkout, state_root, reason_path, snapshot, challenge_path) =
        adoption_transaction_fixture();
    let checkout_root =
        fs::canonicalize(checkout.path()).expect("canonicalize checkout fixture root");
    let state_root_path =
        fs::canonicalize(state_root.path()).expect("canonicalize state fixture root");
    let challenge: ChallengeRecord =
        serde_json::from_slice(&fs::read(&challenge_path).expect("read expiring challenge"))
            .expect("parse expiring challenge");
    let checkout_dir = state_root_path
        .join(&snapshot.repository_key)
        .join(&snapshot.checkout_key);
    let valid_at = challenge.issued_at;
    let expired_at = challenge.expires_at;
    let fourth_barrier_seen = std::cell::Cell::new(false);
    let authoritative_lease_visible = std::cell::Cell::new(false);
    let mut snapshot_calls = 0;

    let error = adopt_dirty_with_snapshot_and_clock(
        &checkout_root,
        &state_root_path,
        &"a".repeat(64),
        &reason_path,
        |path| {
            snapshot_calls += 1;
            if snapshot_calls == 4 {
                authoritative_lease_visible.set(checkout_dir.join("lease.json").exists());
                fourth_barrier_seen.set(true);
            }
            dirty_snapshot(path)
        },
        || {
            Ok(if fourth_barrier_seen.get() {
                expired_at
            } else {
                valid_at
            })
        },
    )
    .expect_err("authorization expiring before durable acceptance must be rejected");

    assert_eq!(snapshot_calls, 4);
    assert!(
        !authoritative_lease_visible.get(),
        "authoritative lease must remain unpublished through the fourth barrier"
    );
    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed durable-boundary expiry")
            .kind(),
        DirtyCheckoutErrorKind::ChallengeExpired
    );
    assert!(
        !checkout_dir.join("lease.json").exists(),
        "expired authorization must not leave an accepted lease"
    );
}
