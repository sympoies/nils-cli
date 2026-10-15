use super::*;

#[test]
fn persisted_adoption_requires_transition_before_challenge_expiry() {
    let identity = CheckoutIdentity {
        root: PathBuf::from("/checkout"),
        git_dir: PathBuf::from("/checkout/.git"),
        common_dir: PathBuf::from("/checkout/.git"),
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
        issued_at: 10,
        expires_at: 20,
    };
    let mut challenge_bytes = serde_json::to_vec(&challenge).expect("serialize challenge");
    challenge_bytes.push(b'\n');
    let challenge_digest = sha256_hex(&challenge_bytes);

    for (adopted_at, valid) in [(19, true), (20, false), (21, false)] {
        let lease = LeaseRecord::V2(Box::new(LeaseV2Wire {
            schema: LEASE_V2_SCHEMA.to_string(),
            session_key: challenge.session_key.clone(),
            checkout_instance: identity.checkout_instance.clone(),
            checkout_root: "/checkout".to_string(),
            checkout_git_dir: "/checkout/.git".to_string(),
            checkout_root_bytes: hex_bytes(identity.root.as_os_str().as_bytes()),
            checkout_git_dir_bytes: hex_bytes(identity.git_dir.as_os_str().as_bytes()),
            acquired_at: adopted_at,
            refreshed_at: adopted_at,
            expires_at: 100,
            adoption: AdoptionRecord {
                schema: ADOPTION_SCHEMA.to_string(),
                receipt_schema: RECEIPT_SCHEMA.to_string(),
                receipt_id: "4".repeat(64),
                snapshot_id: challenge.snapshot_id.clone(),
                authorization_turn_digest: challenge.authorization_turn_digest.clone(),
                reason_digest: "5".repeat(64),
                adopted_at,
                challenge_issued_at: challenge.issued_at,
                challenge_digest: challenge_digest.clone(),
            },
        }));
        assert_eq!(
                validate_spent_challenge_matches_lease(
                    &challenge_bytes,
                    &challenge,
                    &identity,
                    &lease,
                )
                .is_ok(),
                valid,
                "unexpected persisted authorization result at {adopted_at}"
            );
    }
}

#[test]
fn revocation_pruning_accepts_the_scan_cap_and_rejects_cap_plus_one() {
    fn fixture(entry_count: usize) -> (tempfile::TempDir, PathBuf, CheckoutIdentity) {
        let root = tempfile::TempDir::new().expect("revocation scan root");
        let checkout = root.path().join("checkout");
        let git_dir = root.path().join("git-dir");
        let checkout_dir = root.path().join("state");
        for directory in [&checkout, &git_dir, &checkout_dir] {
            fs::create_dir(directory).expect("create revocation scan directory");
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .expect("make revocation scan directory private");
        }
        for index in 0..entry_count {
            fs::write(checkout_dir.join(format!("inert-{index:04}")), b"")
                .expect("write revocation scan entry");
        }
        let identity = CheckoutIdentity {
            root: checkout,
            git_dir: git_dir.clone(),
            common_dir: git_dir,
            repository_key: "a".repeat(64),
            checkout_key: "b".repeat(64),
            checkout_instance: "c".repeat(32),
        };
        (root, checkout_dir, identity)
    }

    let (_exact_root, exact_dir, exact_identity) = fixture(MAX_RECOVERY_DIRECTORY_ENTRIES);
    prune_revocation_tombstones(
        &exact_dir,
        &exact_identity,
        &"d".repeat(64),
        deadline_after(SNAPSHOT_TIMEOUT).expect("exact scan deadline"),
    )
    .expect("the exact revocation scan entry cap must remain supported");

    let deadline_error =
        prune_revocation_tombstones(&exact_dir, &exact_identity, &"d".repeat(64), Instant::now())
            .expect_err("an expired revocation scan deadline must fail closed");
    assert_eq!(
        deadline_error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed revocation scan timeout")
            .kind(),
        DirtyCheckoutErrorKind::Timeout
    );

    let (_overflow_root, overflow_dir, overflow_identity) =
        fixture(MAX_RECOVERY_DIRECTORY_ENTRIES + 1);
    let error = prune_revocation_tombstones(
        &overflow_dir,
        &overflow_identity,
        &"d".repeat(64),
        deadline_after(SNAPSHOT_TIMEOUT).expect("overflow scan deadline"),
    )
    .expect_err("revocation scan entry cap plus one must fail closed");
    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed revocation scan limit")
            .kind(),
        DirtyCheckoutErrorKind::ResourceUnavailable
    );
}

#[test]
fn revocation_tombstones_are_pruned_oldest_first_with_current_protected() {
    let root = tempfile::TempDir::new().expect("tombstone retention root");
    let checkout = root.path().join("checkout");
    let git_dir = root.path().join("git-dir");
    let checkout_dir = root.path().join("state");
    for directory in [&checkout, &git_dir, &checkout_dir] {
        fs::create_dir(directory).expect("create tombstone retention directory");
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .expect("make tombstone retention directory private");
    }
    let identity = CheckoutIdentity {
        root: checkout,
        git_dir: git_dir.clone(),
        common_dir: git_dir,
        repository_key: "a".repeat(64),
        checkout_key: "b".repeat(64),
        checkout_instance: "c".repeat(32),
    };
    let mut receipt_ids = Vec::new();
    for index in 0..=MAX_REVOCATION_TOMBSTONES {
        let receipt_id = format!("{index:064x}");
        let adopted_at = 100 + index as u64;
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
        .expect("write tombstone fixture");
        receipt_ids.push(receipt_id);
    }
    let protected_token_digest = "9".repeat(64);
    let protected_pending = PendingAdoptionRecord {
        schema: PENDING_ADOPTION_SCHEMA.to_string(),
        receipt_id: receipt_ids[0].clone(),
        token_digest: protected_token_digest.clone(),
        challenge_digest: "2".repeat(64),
        session_key: "d".repeat(64),
        checkout_instance: identity.checkout_instance.clone(),
        snapshot_id: "e".repeat(64),
        predecessor_receipt_id: None,
        predecessor_receipt_digest: None,
        predecessor_spent_challenge_digest: None,
        predecessor_lease_digest: None,
        predecessor_lease_bytes: None,
    };
    write_json_atomic(
        &pending_adoption_path(&checkout_dir, &protected_token_digest),
        &protected_pending,
        false,
    )
    .expect("write pending tombstone reference");
    let current = receipt_ids.last().expect("current tombstone");

    prune_revocation_tombstones(
        &checkout_dir,
        &identity,
        current,
        deadline_after(SNAPSHOT_TIMEOUT).expect("pruning deadline"),
    )
    .expect("prune tombstone retention set");

    let remaining = fs::read_dir(&checkout_dir)
        .expect("list retained tombstones")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().as_bytes().starts_with(b".revoked-"))
        .count();
    assert_eq!(remaining, MAX_REVOCATION_TOMBSTONES);
    assert!(
        checkout_dir
            .join(format!(".revoked-{}.json", receipt_ids[0]))
            .exists(),
        "pending-referenced tombstone must remain durable"
    );
    assert!(
        !checkout_dir
            .join(format!(".revoked-{}.json", receipt_ids[1]))
            .exists(),
        "oldest unreferenced tombstone should be pruned first"
    );
    assert!(
        checkout_dir
            .join(format!(".revoked-{current}.json"))
            .exists()
    );
}

#[test]
fn failed_checkout_recovery_protects_every_prescanned_pending_tombstone() {
    let root = tempfile::TempDir::new().expect("pending tombstone recovery root");
    let checkout = root.path().join("checkout");
    let git_dir = root.path().join("git-dir");
    let checkout_dir = root.path().join("state");
    let challenge_dir = checkout_dir.join("challenges");
    let receipts_dir = checkout_dir.join("receipts");
    for directory in [
        checkout.as_path(),
        git_dir.as_path(),
        checkout_dir.as_path(),
        challenge_dir.as_path(),
        receipts_dir.as_path(),
    ] {
        fs::create_dir(directory).expect("create pending tombstone recovery directory");
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .expect("make pending tombstone recovery directory private");
    }
    let identity = CheckoutIdentity {
        root: checkout,
        git_dir: git_dir.clone(),
        common_dir: git_dir,
        repository_key: "a".repeat(64),
        checkout_key: "b".repeat(64),
        checkout_instance: "c".repeat(32),
    };
    let mut receipt_ids = Vec::new();
    for index in 0..=MAX_REVOCATION_TOMBSTONES {
        let receipt_id = format!("{index:064x}");
        let adopted_at = 100 + index as u64;
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
        .expect("write pending tombstone recovery fixture");
        receipt_ids.push(receipt_id);
    }
    for (token_digest, receipt_id) in [
        ("8".repeat(64), &receipt_ids[0]),
        ("9".repeat(64), &receipt_ids[1]),
    ] {
        let pending = PendingAdoptionRecord {
            schema: PENDING_ADOPTION_SCHEMA.to_string(),
            receipt_id: receipt_id.clone(),
            token_digest: token_digest.clone(),
            challenge_digest: "2".repeat(64),
            session_key: "7".repeat(64),
            checkout_instance: identity.checkout_instance.clone(),
            snapshot_id: "e".repeat(64),
            predecessor_receipt_id: None,
            predecessor_receipt_digest: None,
            predecessor_spent_challenge_digest: None,
            predecessor_lease_digest: None,
            predecessor_lease_bytes: None,
        };
        write_json_atomic(
            &pending_adoption_path(&checkout_dir, &token_digest),
            &pending,
            false,
        )
        .expect("write pending tombstone reference");
    }

    let error = recover_checkout_pending_adoptions(
        &checkout_dir,
        &challenge_dir,
        &identity,
        &"6".repeat(64),
        deadline_after(SNAPSHOT_TIMEOUT).expect("pending tombstone recovery deadline"),
    )
    .expect_err("one mismatched pending recovery must remain primary");

    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed pending tombstone recovery failure")
            .kind(),
        DirtyCheckoutErrorKind::MalformedState
    );
    for receipt_id in &receipt_ids[..2] {
        assert!(
            checkout_dir
                .join(format!(".revoked-{receipt_id}.json"))
                .exists(),
            "every prescanned pending marker must protect its tombstone"
        );
    }
}

#[test]
fn predecessor_cleanup_failure_is_reported_for_recovery() {
    let root = tempfile::TempDir::new().expect("predecessor cleanup root");
    let receipt_path = root.path().join("receipt.json");
    let spent_challenge_path = root.path().join("spent.json");
    fs::create_dir(&receipt_path).expect("create non-removable receipt fixture");
    fs::write(&spent_challenge_path, b"spent").expect("write spent fixture");
    let predecessor = ExpiredPredecessor {
        receipt_id: "a".repeat(64),
        receipt_path,
        spent_challenge_path,
        receipt_digest: Some("b".repeat(64)),
        spent_challenge_digest: Some("c".repeat(64)),
    };

    cleanup_expired_predecessor(&predecessor)
        .expect_err("predecessor cleanup failure must not be silent");
    assert!(
        predecessor.spent_challenge_path.exists(),
        "cleanup must stop without forgetting the remaining predecessor state"
    );
}
