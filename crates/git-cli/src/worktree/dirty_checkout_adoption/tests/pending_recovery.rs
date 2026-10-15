use super::*;

#[test]
fn pending_adoption_recovery_covers_preinstall_commit_and_rollback_faults() {
    let root = tempfile::TempDir::new().expect("pending recovery root");
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
        fs::create_dir(directory).expect("create recovery directory");
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .expect("make recovery directory private");
    }
    let identity = CheckoutIdentity {
        root: checkout,
        git_dir: git_dir.clone(),
        common_dir: git_dir,
        repository_key: "a".repeat(64),
        checkout_key: "b".repeat(64),
        checkout_instance: "c".repeat(32),
    };
    let challenge_digest = "d".repeat(64);
    let receipt_id = "e".repeat(64);
    let predecessor_receipt_id = "f".repeat(64);
    let challenge = ChallengeRecord {
        schema: CHALLENGE_SCHEMA.to_string(),
        token_digest: challenge_digest.clone(),
        session_key: "1".repeat(64),
        repository_key: identity.repository_key.clone(),
        checkout_key: identity.checkout_key.clone(),
        checkout_instance: identity.checkout_instance.clone(),
        snapshot_id: "2".repeat(64),
        head_oid: "3".repeat(40),
        branch_ref_digest: "4".repeat(64),
        authorization_turn_digest: "5".repeat(64),
        issued_at: 10,
        expires_at: 20,
    };
    let challenge_artifact_digest = {
        let mut bytes = serde_json::to_vec(&challenge).expect("serialize recovery challenge");
        bytes.push(b'\n');
        sha256_hex(&bytes)
    };
    let receipt = ReceiptRecord {
        schema: RECEIPT_SCHEMA.to_string(),
        receipt_id: receipt_id.clone(),
        session_key: challenge.session_key.clone(),
        repository_key: identity.repository_key.clone(),
        checkout_key: identity.checkout_key.clone(),
        checkout_instance: identity.checkout_instance.clone(),
        snapshot_id: challenge.snapshot_id.clone(),
        authorization_turn_digest: challenge.authorization_turn_digest.clone(),
        reason_digest: "6".repeat(64),
        challenge_digest: challenge_artifact_digest.clone(),
        adopted_at: 15,
    };
    let pending_path = pending_adoption_path(&checkout_dir, &challenge_digest);
    let challenge_path = challenge_dir.join(format!("{challenge_digest}.json"));
    let receipt_path = receipts_dir.join(format!("{receipt_id}.json"));
    let spent_path = receipts_dir.join(format!(".challenge-{receipt_id}.json"));
    let pending = PendingAdoptionRecord {
        schema: PENDING_ADOPTION_SCHEMA.to_string(),
        receipt_id: receipt_id.clone(),
        token_digest: challenge_digest.clone(),
        challenge_digest: challenge_artifact_digest.clone(),
        session_key: challenge.session_key.clone(),
        checkout_instance: identity.checkout_instance.clone(),
        snapshot_id: challenge.snapshot_id.clone(),
        predecessor_receipt_id: None,
        predecessor_receipt_digest: None,
        predecessor_spent_challenge_digest: None,
        predecessor_lease_digest: None,
        predecessor_lease_bytes: None,
    };

    write_json_atomic(&challenge_path, &challenge, false).expect("write challenge");
    write_json_atomic(&receipt_path, &receipt, false).expect("write prepared receipt");
    write_json_atomic(&pending_path, &pending, false).expect("write pending marker");
    fs::rename(&challenge_path, &spent_path).expect("simulate consumed challenge");
    assert_eq!(
        recover_pending_adoption(&checkout_dir, &challenge_dir, &identity, &challenge_digest,)
            .expect("roll back preinstall transition"),
        PendingRecovery::RolledBack
    );
    assert!(challenge_path.exists());
    assert!(!spent_path.exists());
    assert!(!receipt_path.exists());
    assert!(!pending_path.exists());

    write_json_atomic(&receipt_path, &receipt, false).expect("rewrite committed receipt");
    write_json_atomic(&pending_path, &pending, false).expect("rewrite pending marker");
    fs::rename(&challenge_path, &spent_path).expect("consume committed challenge");
    let lease = LeaseRecord::V2(Box::new(LeaseV2Wire {
        schema: LEASE_V2_SCHEMA.to_string(),
        session_key: challenge.session_key.clone(),
        checkout_instance: identity.checkout_instance.clone(),
        checkout_root: identity.root.to_str().expect("UTF-8 root").to_string(),
        checkout_git_dir: identity
            .git_dir
            .to_str()
            .expect("UTF-8 Git dir")
            .to_string(),
        checkout_root_bytes: hex_bytes(identity.root.as_os_str().as_bytes()),
        checkout_git_dir_bytes: hex_bytes(identity.git_dir.as_os_str().as_bytes()),
        acquired_at: 15,
        refreshed_at: 15,
        expires_at: 30,
        adoption: AdoptionRecord {
            schema: ADOPTION_SCHEMA.to_string(),
            receipt_schema: RECEIPT_SCHEMA.to_string(),
            receipt_id: receipt_id.clone(),
            snapshot_id: challenge.snapshot_id.clone(),
            authorization_turn_digest: challenge.authorization_turn_digest.clone(),
            reason_digest: receipt.reason_digest.clone(),
            adopted_at: 15,
            challenge_issued_at: challenge.issued_at,
            challenge_digest: challenge_artifact_digest.clone(),
        },
    }));
    write_json_atomic(&checkout_dir.join("lease.json"), &lease, false)
        .expect("write committed lease");
    assert_eq!(
        recover_pending_adoption(&checkout_dir, &challenge_dir, &identity, &challenge_digest,)
            .expect("finish committed transition"),
        PendingRecovery::Committed
    );
    assert!(!pending_path.exists());
    assert!(receipt_path.exists());
    assert!(spent_path.exists());

    write_json_atomic(&pending_path, &pending, false)
        .expect("write missing-receipt recovery marker");
    fs::remove_file(&receipt_path).expect("remove current committed receipt");
    let missing_receipt =
        recover_pending_adoption(&checkout_dir, &challenge_dir, &identity, &challenge_digest)
            .expect_err("committed recovery must require the current receipt");
    assert_eq!(
        missing_receipt
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed missing receipt error")
            .kind(),
        DirtyCheckoutErrorKind::MalformedState
    );
    assert!(pending_path.exists());

    write_json_atomic(&receipt_path, &receipt, false).expect("restore current receipt");
    fs::remove_file(&spent_path).expect("remove current spent challenge");
    let missing_spent =
        recover_pending_adoption(&checkout_dir, &challenge_dir, &identity, &challenge_digest)
            .expect_err("committed recovery must require the exact spent challenge");
    assert_eq!(
        missing_spent
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed missing spent challenge error")
            .kind(),
        DirtyCheckoutErrorKind::MalformedState
    );
    assert!(pending_path.exists());
    write_json_atomic(&spent_path, &challenge, false).expect("restore spent challenge");
    assert_eq!(
        recover_pending_adoption(&checkout_dir, &challenge_dir, &identity, &challenge_digest)
            .expect("finish restored committed transition"),
        PendingRecovery::Committed
    );

    let committed_pending = PendingAdoptionRecord {
        predecessor_receipt_id: Some(predecessor_receipt_id.clone()),
        predecessor_receipt_digest: Some("7".repeat(64)),
        predecessor_spent_challenge_digest: Some("8".repeat(64)),
        ..pending
    };
    let predecessor_receipt_path = receipts_dir.join(format!("{predecessor_receipt_id}.json"));
    let predecessor_spent_path =
        receipts_dir.join(format!(".challenge-{predecessor_receipt_id}.json"));
    fs::write(&predecessor_receipt_path, b"unbound predecessor receipt")
        .expect("write unbound predecessor receipt");
    fs::write(&predecessor_spent_path, b"unbound predecessor challenge")
        .expect("write unbound predecessor challenge");
    write_json_atomic(&pending_path, &committed_pending, false)
        .expect("write unbound predecessor marker");

    let unbound =
        recover_pending_adoption(&checkout_dir, &challenge_dir, &identity, &challenge_digest)
            .expect_err("unbound predecessor artifacts must fail closed");
    assert_eq!(
        unbound
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed unbound predecessor error")
            .kind(),
        DirtyCheckoutErrorKind::MalformedState
    );
    assert!(pending_path.exists());
    assert!(predecessor_receipt_path.exists());
    assert!(predecessor_spent_path.exists());
    fs::remove_file(&pending_path).expect("remove unbound predecessor marker");
    fs::remove_file(&predecessor_receipt_path).expect("remove unbound predecessor receipt");
    fs::remove_file(&predecessor_spent_path).expect("remove unbound predecessor challenge");

    let predecessor_receipt_bytes = b"bound predecessor receipt";
    let predecessor_spent_bytes = b"bound predecessor challenge";
    let bound_pending = PendingAdoptionRecord {
        predecessor_receipt_digest: Some(sha256_hex(predecessor_receipt_bytes)),
        predecessor_spent_challenge_digest: Some(sha256_hex(predecessor_spent_bytes)),
        ..committed_pending.clone()
    };
    for missing_path in [&predecessor_receipt_path, &predecessor_spent_path] {
        fs::write(&predecessor_receipt_path, predecessor_receipt_bytes)
            .expect("write bound predecessor receipt");
        fs::write(&predecessor_spent_path, predecessor_spent_bytes)
            .expect("write bound predecessor challenge");
        for path in [&predecessor_receipt_path, &predecessor_spent_path] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("make bound predecessor artifact private");
        }
        write_json_atomic(&pending_path, &bound_pending, false)
            .expect("write bound predecessor marker");
        fs::remove_file(missing_path).expect("simulate partial predecessor cleanup");

        assert_eq!(
            recover_pending_adoption(&checkout_dir, &challenge_dir, &identity, &challenge_digest,)
                .expect("resume partial predecessor cleanup"),
            PendingRecovery::Committed
        );
        assert!(!pending_path.exists());
        assert!(!predecessor_receipt_path.exists());
        assert!(!predecessor_spent_path.exists());
    }

    fs::remove_file(checkout_dir.join("lease.json")).expect("remove committed lease");
    write_json_atomic(&pending_path, &committed_pending, false)
        .expect("write rollback-failure marker");
    write_json_atomic(&challenge_path, &challenge, false)
        .expect("create duplicate current challenge");
    let error =
        recover_pending_adoption(&checkout_dir, &challenge_dir, &identity, &challenge_digest)
            .expect_err("duplicate challenge state must retain recovery marker");
    assert_eq!(
        error
            .downcast_ref::<DirtyCheckoutError>()
            .expect("typed rollback failure")
            .kind(),
        DirtyCheckoutErrorKind::MalformedState
    );
    assert!(pending_path.exists());
}

#[test]
fn lease_install_fault_classification_compares_complete_validated_state() {
    let root = tempfile::TempDir::new().expect("lease state root");
    let checkout = root.path().join("checkout");
    let git_dir = root.path().join("git-dir");
    fs::create_dir(&checkout).expect("create checkout");
    fs::create_dir(&git_dir).expect("create git dir");
    let identity = CheckoutIdentity {
        root: checkout,
        git_dir: git_dir.clone(),
        common_dir: git_dir,
        repository_key: "a".repeat(64),
        checkout_key: "b".repeat(64),
        checkout_instance: "c".repeat(32),
    };
    let expected = LeaseRecord::V2(Box::new(LeaseV2Wire {
        schema: LEASE_V2_SCHEMA.to_string(),
        session_key: "d".repeat(64),
        checkout_instance: identity.checkout_instance.clone(),
        checkout_root: identity.root.to_str().expect("UTF-8 root").to_string(),
        checkout_git_dir: identity
            .git_dir
            .to_str()
            .expect("UTF-8 Git dir")
            .to_string(),
        checkout_root_bytes: hex_bytes(identity.root.as_os_str().as_bytes()),
        checkout_git_dir_bytes: hex_bytes(identity.git_dir.as_os_str().as_bytes()),
        acquired_at: 10,
        refreshed_at: 20,
        expires_at: 30,
        adoption: AdoptionRecord {
            schema: ADOPTION_SCHEMA.to_string(),
            receipt_schema: RECEIPT_SCHEMA.to_string(),
            receipt_id: "e".repeat(64),
            snapshot_id: "f".repeat(64),
            authorization_turn_digest: "1".repeat(64),
            reason_digest: "2".repeat(64),
            adopted_at: 20,
            challenge_issued_at: 10,
            challenge_digest: "3".repeat(64),
        },
    }));
    let lease_path = root.path().join("lease.json");

    assert!(matches!(
        install_lease_with(&lease_path, &expected, None, &identity, || Err(
            anyhow::anyhow!("injected pre-install failure")
        )),
        LeaseInstallOutcome::NotInstalled(_)
    ));

    assert!(matches!(
        install_lease_with(&lease_path, &expected, None, &identity, || {
            write_json_atomic(&lease_path, &expected, true)?;
            Err(anyhow::anyhow!("injected post-install durability failure"))
        }),
        LeaseInstallOutcome::Installed
    ));

    let mut different = expected.clone();
    let LeaseRecord::V2(different) = &mut different else {
        panic!("expected v2 lease");
    };
    different.adoption.reason_digest = "4".repeat(64);
    assert!(matches!(
        install_lease_with(&lease_path, &expected, None, &identity, || {
            write_json_atomic(&lease_path, &different, true)?;
            Err(anyhow::anyhow!("injected ambiguous durability failure"))
        }),
        LeaseInstallOutcome::Ambiguous(_)
    ));
}
