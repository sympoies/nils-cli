use super::*;

#[test]
fn strict_wire_round_trips_and_rejects_duplicate_or_unknown_fields() {
    let root = tempfile::TempDir::new().expect("wire root");
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
    let adoption = AdoptionRecord {
        schema: ADOPTION_SCHEMA.to_string(),
        receipt_schema: RECEIPT_SCHEMA.to_string(),
        receipt_id: "d".repeat(64),
        snapshot_id: "e".repeat(64),
        authorization_turn_digest: "f".repeat(64),
        reason_digest: "1".repeat(64),
        adopted_at: 20,
        challenge_issued_at: 10,
        challenge_digest: "2".repeat(64),
    };
    let v2 = LeaseRecord::V2(Box::new(LeaseV2Wire {
        schema: LEASE_V2_SCHEMA.to_string(),
        session_key: "3".repeat(64),
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
        adoption: adoption.clone(),
    }));
    let v2_path = root.path().join("v2.json");
    write_json_atomic(&v2_path, &v2, false).expect("write canonical v2");
    assert_eq!(load_lease(&v2_path).expect("load v2"), Some(v2));

    let v1 = LeaseRecord::V1(LeaseV1Wire {
        schema: LEASE_V1_SCHEMA.to_string(),
        session_key: "4".repeat(64),
        checkout_instance: identity.checkout_instance.clone(),
        checkout_root: identity.root.to_str().expect("UTF-8 root").to_string(),
        checkout_git_dir: identity
            .git_dir
            .to_str()
            .expect("UTF-8 Git dir")
            .to_string(),
        acquired_at: 10,
        refreshed_at: 20,
        expires_at: 30,
    });
    let v1_path = root.path().join("v1.json");
    write_json_atomic(&v1_path, &v1, false).expect("write canonical v1");
    assert_eq!(load_lease(&v1_path).expect("load v1"), Some(v1));

    let duplicate_path = root.path().join("duplicate.json");
    let duplicate = format!(
        "{{\"schema\":\"{LEASE_V1_SCHEMA}\",\"session_key\":\"{}\",\"session_key\":\"{}\",\"checkout_instance\":\"{}\",\"checkout_root\":{},\"checkout_git_dir\":{},\"acquired_at\":10,\"refreshed_at\":20,\"expires_at\":30}}\n",
        "4".repeat(64),
        "5".repeat(64),
        identity.checkout_instance,
        serde_json::to_string(identity.root.to_str().expect("UTF-8 root")).expect("root JSON"),
        serde_json::to_string(identity.git_dir.to_str().expect("UTF-8 Git dir"))
            .expect("Git-dir JSON")
    );
    fs::write(&duplicate_path, duplicate).expect("write duplicate-key lease");
    fs::set_permissions(&duplicate_path, fs::Permissions::from_mode(0o600))
        .expect("make duplicate-key lease private");
    load_lease(&duplicate_path).expect_err("duplicate known fields must be rejected");

    let mut unknown_adoption = serde_json::to_value(&adoption).expect("serialize adoption");
    unknown_adoption["unexpected"] = serde_json::json!(true);
    serde_json::from_value::<AdoptionRecord>(unknown_adoption)
        .expect_err("unknown adoption field must be rejected");
    let mut missing_receipt_schema = serde_json::to_value(&adoption).expect("serialize adoption");
    missing_receipt_schema
        .as_object_mut()
        .expect("adoption object")
        .remove("receipt_schema");
    serde_json::from_value::<AdoptionRecord>(missing_receipt_schema)
        .expect_err("receipt_schema must be required");

    let receipt = ReceiptRecord {
        schema: RECEIPT_SCHEMA.to_string(),
        receipt_id: adoption.receipt_id,
        session_key: "3".repeat(64),
        repository_key: identity.repository_key,
        checkout_key: identity.checkout_key,
        checkout_instance: identity.checkout_instance,
        snapshot_id: adoption.snapshot_id,
        authorization_turn_digest: adoption.authorization_turn_digest,
        reason_digest: adoption.reason_digest,
        challenge_digest: adoption.challenge_digest,
        adopted_at: adoption.adopted_at,
    };
    let encoded_receipt = serde_json::to_vec(&receipt).expect("serialize receipt");
    let decoded_receipt: ReceiptRecord =
        serde_json::from_slice(&encoded_receipt).expect("round-trip receipt");
    assert_eq!(decoded_receipt.receipt_id, receipt.receipt_id);
    let mut unknown_receipt = serde_json::to_value(&receipt).expect("serialize receipt");
    unknown_receipt["unexpected"] = serde_json::json!(true);
    serde_json::from_value::<ReceiptRecord>(unknown_receipt)
        .expect_err("unknown receipt field must be rejected");
}

#[test]
fn canonical_json_fixtures_match_producers_and_cross_file_contracts() {
    const ROOT: &str = "/workspace/agent-runtime-kit";
    const GIT_DIR: &str = "/workspace/agent-runtime-kit/.git/worktrees/dirty-adoption";
    const ROOT_BYTES: &str = "2f776f726b73706163652f6167656e742d72756e74696d652d6b6974";
    const GIT_DIR_BYTES: &str = "2f776f726b73706163652f6167656e742d72756e74696d652d6b69742f2e6769742f776f726b74726565732f64697274792d61646f7074696f6e";

    let snapshot_fixture = include_str!(
        "../../../../tests/fixtures/dirty-checkout-adoption/dirty-checkout-snapshot-v1.json"
    );
    let challenge_fixture = include_str!(
        "../../../../tests/fixtures/dirty-checkout-adoption/dirty-checkout-challenge-v1.json"
    );
    let receipt_fixture = include_str!(
        "../../../../tests/fixtures/dirty-checkout-adoption/dirty-checkout-receipt-v1.json"
    );
    let lease_v1_fixture =
        include_str!("../../../../tests/fixtures/dirty-checkout-adoption/checkout-lease-v1.json");
    let lease_v2_fixture =
        include_str!("../../../../tests/fixtures/dirty-checkout-adoption/checkout-lease-v2.json");
    let adoption_fixture = include_str!(
        "../../../../tests/fixtures/dirty-checkout-adoption/dirty-checkout-adoption-v1.json"
    );
    let fixture_value = |source: &str| {
        serde_json::from_str::<serde_json::Value>(source).expect("parse canonical fixture")
    };

    let expected_snapshot = DirtySnapshot {
        schema: SNAPSHOT_SCHEMA,
        repository_key: "a".repeat(64),
        checkout_key: "b".repeat(64),
        checkout_instance: "c".repeat(32),
        snapshot_id: "d".repeat(64),
        head_oid: "e".repeat(40),
        branch_ref_digest: "f".repeat(64),
        tracked_entries: 2,
        untracked_entries: 1,
        hashed_bytes: 4096,
    };
    assert_eq!(
        serde_json::to_value(&expected_snapshot).expect("serialize snapshot producer"),
        fixture_value(snapshot_fixture)
    );
    let snapshot: SnapshotWorkerSnapshot =
        serde_json::from_str(snapshot_fixture).expect("strict snapshot fixture");

    let expected_challenge = ChallengeRecord {
        schema: CHALLENGE_SCHEMA.to_string(),
        token_digest: "1".repeat(64),
        session_key: "2".repeat(64),
        repository_key: "a".repeat(64),
        checkout_key: "b".repeat(64),
        checkout_instance: "c".repeat(32),
        snapshot_id: "d".repeat(64),
        head_oid: "e".repeat(40),
        branch_ref_digest: "f".repeat(64),
        authorization_turn_digest: "3".repeat(64),
        issued_at: 1_700_000_000,
        expires_at: 1_700_000_300,
    };
    assert_eq!(
        serde_json::to_value(&expected_challenge).expect("serialize challenge producer"),
        fixture_value(challenge_fixture)
    );
    let challenge: ChallengeRecord =
        serde_json::from_str(challenge_fixture).expect("strict challenge fixture");
    let challenge_artifact_digest = sha256_hex(challenge_fixture.as_bytes());

    let expected_receipt = ReceiptRecord {
        schema: RECEIPT_SCHEMA.to_string(),
        receipt_id: "4".repeat(64),
        session_key: "2".repeat(64),
        repository_key: "a".repeat(64),
        checkout_key: "b".repeat(64),
        checkout_instance: "c".repeat(32),
        snapshot_id: "d".repeat(64),
        authorization_turn_digest: "3".repeat(64),
        reason_digest: "5".repeat(64),
        challenge_digest: challenge_artifact_digest.clone(),
        adopted_at: 1_700_000_100,
    };
    assert_eq!(
        serde_json::to_value(&expected_receipt).expect("serialize receipt producer"),
        fixture_value(receipt_fixture)
    );
    let receipt: ReceiptRecord =
        serde_json::from_str(receipt_fixture).expect("strict receipt fixture");

    let expected_adoption = AdoptionRecord {
        schema: ADOPTION_SCHEMA.to_string(),
        receipt_schema: RECEIPT_SCHEMA.to_string(),
        receipt_id: "4".repeat(64),
        snapshot_id: "d".repeat(64),
        authorization_turn_digest: "3".repeat(64),
        reason_digest: "5".repeat(64),
        adopted_at: 1_700_000_100,
        challenge_issued_at: 1_700_000_000,
        challenge_digest: challenge_artifact_digest.clone(),
    };
    assert_eq!(
        serde_json::to_value(&expected_adoption).expect("serialize adoption producer"),
        fixture_value(adoption_fixture)
    );
    let adoption: AdoptionRecord =
        serde_json::from_str(adoption_fixture).expect("strict adoption fixture");

    let expected_v1 = LeaseRecord::V1(LeaseV1Wire {
        schema: LEASE_V1_SCHEMA.to_string(),
        session_key: "2".repeat(64),
        checkout_instance: "c".repeat(32),
        checkout_root: ROOT.to_string(),
        checkout_git_dir: GIT_DIR.to_string(),
        acquired_at: 1_699_999_000,
        refreshed_at: 1_700_000_000,
        expires_at: 1_700_003_600,
    });
    assert_eq!(
        serde_json::to_value(&expected_v1).expect("serialize v1 lease producer"),
        fixture_value(lease_v1_fixture)
    );
    let lease_v1 = parse_lease(lease_v1_fixture.as_bytes()).expect("strict v1 lease fixture");
    assert_eq!(lease_v1, expected_v1);

    let expected_v2 = LeaseRecord::V2(Box::new(LeaseV2Wire {
        schema: LEASE_V2_SCHEMA.to_string(),
        session_key: "2".repeat(64),
        checkout_instance: "c".repeat(32),
        checkout_root: ROOT.to_string(),
        checkout_git_dir: GIT_DIR.to_string(),
        checkout_root_bytes: ROOT_BYTES.to_string(),
        checkout_git_dir_bytes: GIT_DIR_BYTES.to_string(),
        acquired_at: 1_700_000_100,
        refreshed_at: 1_700_000_100,
        expires_at: 1_700_003_700,
        adoption: expected_adoption,
    }));
    assert_eq!(
        serde_json::to_value(&expected_v2).expect("serialize v2 lease producer"),
        fixture_value(lease_v2_fixture)
    );
    let lease_v2 = parse_lease(lease_v2_fixture.as_bytes()).expect("strict v2 lease fixture");
    assert_eq!(lease_v2, expected_v2);

    let identity = CheckoutIdentity {
        root: PathBuf::from(ROOT),
        git_dir: PathBuf::from(GIT_DIR),
        common_dir: PathBuf::from(GIT_DIR),
        repository_key: snapshot.repository_key.clone(),
        checkout_key: snapshot.checkout_key.clone(),
        checkout_instance: snapshot.checkout_instance.clone(),
    };
    validate_challenge_identity(&challenge, &identity, &challenge.token_digest)
        .expect("validate challenge fixture");
    validate_receipt(&receipt, &identity, &receipt.receipt_id).expect("validate receipt fixture");
    validate_lease(&lease_v1, &identity).expect("validate v1 lease fixture");
    validate_lease(&lease_v2, &identity).expect("validate v2 lease fixture");

    assert_eq!(challenge.repository_key, snapshot.repository_key);
    assert_eq!(challenge.checkout_key, snapshot.checkout_key);
    assert_eq!(challenge.checkout_instance, snapshot.checkout_instance);
    assert_eq!(challenge.snapshot_id, snapshot.snapshot_id);
    assert_eq!(challenge.head_oid, snapshot.head_oid);
    assert_eq!(challenge.branch_ref_digest, snapshot.branch_ref_digest);
    assert_eq!(receipt.session_key, challenge.session_key);
    assert_eq!(receipt.challenge_digest, challenge_artifact_digest);
    assert_ne!(receipt.challenge_digest, challenge.token_digest);
    assert_eq!(receipt.repository_key, challenge.repository_key);
    assert_eq!(receipt.checkout_key, challenge.checkout_key);
    assert_eq!(receipt.checkout_instance, challenge.checkout_instance);
    assert_eq!(receipt.snapshot_id, challenge.snapshot_id);
    assert_eq!(
        receipt.authorization_turn_digest,
        challenge.authorization_turn_digest
    );
    assert_eq!(adoption.receipt_id, receipt.receipt_id);
    assert_eq!(adoption.snapshot_id, receipt.snapshot_id);
    assert_eq!(
        adoption.authorization_turn_digest,
        receipt.authorization_turn_digest
    );
    assert_eq!(adoption.reason_digest, receipt.reason_digest);
    assert_eq!(adoption.challenge_digest, receipt.challenge_digest);
    assert_eq!(adoption.adopted_at, receipt.adopted_at);
    assert_eq!(adoption.challenge_issued_at, challenge.issued_at);
    assert_eq!(lease_v2.adoption(), Some(&adoption));
    assert_eq!(lease_v2.session_key(), receipt.session_key);
    assert_eq!(lease_v2.checkout_instance(), receipt.checkout_instance);
    assert_eq!(
        decode_lower_hex(ROOT_BYTES).as_deref(),
        Some(ROOT.as_bytes())
    );
    assert_eq!(
        decode_lower_hex(GIT_DIR_BYTES).as_deref(),
        Some(GIT_DIR.as_bytes())
    );
}
