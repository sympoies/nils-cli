//! Work-context admission regressions on an ordinary enforced session.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nils_test_support::bin;
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::json;

use super::coordination::{
    candidate, capability, data, digest, init_checkout, load_coordination_registry,
    rewrite_registry, run, seed_activity_state, seed_brokers_at, seed_live_runtime_identity,
    write_private_json,
};

fn init_enforced_admission_session() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    String,
) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let checkout = tmp.path().join("checkout");
    fs::create_dir(&state_dir).expect("state");
    init_checkout(&checkout, "https://example.invalid/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[(
            "admission",
            "admission-incarnation",
            "admission-private-capability-material",
            checkout.as_path(),
            Some("enforce"),
        )],
    );
    let capability_file = capability(&state_dir, "admission");
    (tmp, state_dir, checkout, capability_file)
}

fn claim_admission_scope(
    tmp: &tempfile::TempDir,
    state_dir: &Path,
    checkout: &Path,
    capability_file: &str,
) -> serde_json::Value {
    let candidate_file = tmp.path().join("admission-candidate.json");
    candidate(&candidate_file, "src/owned/", "admission context");
    let claimed = run(
        checkout,
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "work-context",
            "claim",
            "--session",
            "admission",
            "--file",
            candidate_file.to_str().expect("candidate"),
            "--capability-file",
            capability_file,
            "--idempotency-key",
            "claim-admission-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(
        claimed.code,
        0,
        "stdout={} stderr={}",
        claimed.stdout_text(),
        claimed.stderr_text()
    );
    let claim_id = data(&claimed)["context"]["claim_id"].clone();
    load_coordination_registry(state_dir)["claims"]
        .as_array()
        .expect("claims")
        .iter()
        .find(|claim| claim["claim_id"] == claim_id && claim["state"] == "active")
        .expect("active claim")
        .clone()
}

fn wait_for_claim_barrier(barrier: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !barrier.join("ready").is_file() {
        assert!(
            Instant::now() < deadline,
            "admission did not reach its claim barrier"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// An admitted lease whose PostToolUse never arrived reaches its safety TTL
/// and fails closed to `completing`. Without its execution token nothing made
/// it terminal, so every later admission and every wake of the same session
/// was refused forever (sympoies/nils-cli#1881). Once the TTL has expired and
/// the controller proves the lease's own turn superseded with no live
/// descendant under an unchanged runtime, registry maintenance reclaims it;
/// before any of those proofs admission still refuses.
#[test]
fn expired_superseded_lease_is_reclaimed_only_with_inactivity_proof() {
    let (tmp, state_dir, checkout, capability_file) = init_enforced_admission_session();
    let set_turn = |turn: &str| {
        seed_activity_state(
            &state_dir,
            "admission",
            "admission-incarnation",
            "working",
            json!({
                "provider_turn_id": turn,
                "started_at": "2030-01-01T00:00:01Z"
            }),
            serde_json::Value::Null,
        );
    };
    set_turn("turn-orphaned-lease");
    let _runtime =
        seed_live_runtime_identity(&state_dir, "admission", "admission-incarnation", 196);
    let active_claim = claim_admission_scope(&tmp, &state_dir, &checkout, &capability_file);
    let claim_id = active_claim["claim_id"]
        .as_str()
        .expect("claim id")
        .to_string();
    let claim_revision = active_claim["revision"]
        .as_u64()
        .expect("claim revision")
        .to_string();
    let targets = tmp.path().join("orphaned-lease-targets.json");
    write_private_json(
        &targets,
        &json!({
            "schema_version": "agent-session.operation-targets.v1",
            "targets": [{
                "kind": "path-exact",
                "repository": "example/repository",
                "value": "src/owned/lib.rs"
            }]
        }),
    );
    let state_arg = state_dir.to_string_lossy().into_owned();
    let admit = |token: &str, key: &str| {
        let token_path = tmp.path().join(format!("{key}-execution-token"));
        fs::write(&token_path, token).expect("execution token");
        fs::set_permissions(&token_path, fs::Permissions::from_mode(0o600))
            .expect("execution token mode");
        run(
            &checkout,
            &[
                "--state-dir",
                state_arg.as_str(),
                "work-context",
                "admit",
                "--session",
                "admission",
                "--claim",
                claim_id.as_str(),
                "--if-revision",
                claim_revision.as_str(),
                "--targets-file",
                targets.to_str().expect("targets"),
                "--operation",
                "edit",
                "--execution-token-file",
                token_path.to_str().expect("execution token"),
                "--capability-file",
                capability_file.as_str(),
                "--idempotency-key",
                key,
                "--format",
                "json",
            ],
        )
    };
    let orphaned = admit(
        "orphaned-lease-execution-token",
        "admit-orphaned-lease-0001",
    );
    assert_eq!(orphaned.code, 0, "{}", orphaned.stdout_text());
    let orphaned_lease = data(&orphaned)["lease_id"]
        .as_str()
        .expect("lease id")
        .to_string();
    let lease_state = || {
        load_coordination_registry(&state_dir)["operations"]
            .as_array()
            .expect("operations")
            .iter()
            .find(|lease| lease["lease_id"] == orphaned_lease.as_str())
            .expect("orphaned lease")
            .clone()
    };
    let expire = || {
        rewrite_registry(&state_dir, |registry| {
            let lease = registry["operations"]
                .as_array_mut()
                .expect("operations")
                .iter_mut()
                .find(|lease| lease["lease_id"] == orphaned_lease.as_str())
                .expect("orphaned lease");
            lease["state"] = json!("completing");
            lease["expires_at"] = json!("2001-01-01T00:00:00Z");
            lease["expires_at_epoch"] = json!(978_307_200_i64);
        });
    };

    // A later turn alone does not reclaim a lease inside its safety TTL, even
    // one already `completing`.
    set_turn("turn-after-orphaned-lease");
    let unexpired = admit("unexpired-execution-token", "admit-unexpired-0001");
    assert_ne!(unexpired.code, 0, "{}", unexpired.stdout_text());
    assert_eq!(
        unexpired.stdout_json()["error"]["code"],
        "coordination-unavailable"
    );
    assert_eq!(lease_state()["state"], "active");
    rewrite_registry(&state_dir, |registry| {
        let lease = registry["operations"]
            .as_array_mut()
            .expect("operations")
            .iter_mut()
            .find(|lease| lease["lease_id"] == orphaned_lease.as_str())
            .expect("orphaned lease");
        lease["state"] = json!("completing");
    });
    let completing = admit("completing-execution-token", "admit-completing-0001");
    assert_ne!(completing.code, 0, "{}", completing.stdout_text());
    assert_eq!(
        completing.stdout_json()["error"]["code"],
        "coordination-unavailable"
    );
    assert_eq!(lease_state()["state"], "completing");

    // An expired, superseded lease admitted by a different runtime identity is
    // not proven inactive by this runtime's evidence.
    expire();
    let original_runtime_digest = lease_state()["runtime_identity_digest"].clone();
    rewrite_registry(&state_dir, |registry| {
        let lease = registry["operations"]
            .as_array_mut()
            .expect("operations")
            .iter_mut()
            .find(|lease| lease["lease_id"] == orphaned_lease.as_str())
            .expect("orphaned lease");
        lease["runtime_identity_digest"] = json!(digest("another-runtime"));
    });
    let other_runtime = admit("other-runtime-execution-token", "admit-other-runtime-0001");
    assert_ne!(other_runtime.code, 0, "{}", other_runtime.stdout_text());
    assert_eq!(
        other_runtime.stdout_json()["error"]["code"],
        "coordination-unavailable"
    );
    assert_ne!(lease_state()["state"], "abandoned");
    rewrite_registry(&state_dir, |registry| {
        let lease = registry["operations"]
            .as_array_mut()
            .expect("operations")
            .iter_mut()
            .find(|lease| lease["lease_id"] == orphaned_lease.as_str())
            .expect("orphaned lease");
        lease["runtime_identity_digest"] = original_runtime_digest.clone();
    });

    // An expired lease whose own turn is still current is not proven inactive.
    set_turn("turn-orphaned-lease");
    expire();
    let same_turn = admit("same-turn-execution-token", "admit-same-turn-0001");
    assert_ne!(same_turn.code, 0, "{}", same_turn.stdout_text());
    assert_eq!(
        same_turn.stdout_json()["error"]["code"],
        "coordination-unavailable"
    );
    assert_ne!(lease_state()["state"], "abandoned");

    // Expired and superseded by an idle composer: ordinary registry
    // maintenance reclaims it without any admission, so an idle session that
    // will never mutate again is not blocked from its next wake.
    seed_activity_state(
        &state_dir,
        "admission",
        "admission-incarnation",
        "waiting",
        serde_json::Value::Null,
        json!({
            "provider_turn_id": "turn-orphaned-lease",
            "started_at": "2030-01-01T00:00:01Z",
            "completed_at": "2030-01-01T00:00:02Z",
            "outcome": "completed"
        }),
    );
    expire();
    let shown = run(
        &checkout,
        &[
            "--state-dir",
            state_arg.as_str(),
            "work-context",
            "show",
            "--session",
            "admission",
            "--capability-file",
            capability_file.as_str(),
            "--format",
            "json",
        ],
    );
    assert_eq!(shown.code, 0, "{}", shown.stdout_text());
    let abandoned = lease_state();
    assert_eq!(abandoned["state"], "abandoned");
    assert_eq!(abandoned["outcome"], "ttl-expired-inactive");

    // The session's next admission then proceeds.
    set_turn("turn-after-orphaned-lease");
    let reclaimed = admit("reclaimed-execution-token", "admit-reclaimed-0001");
    assert_eq!(
        reclaimed.code,
        0,
        "stdout={} stderr={}",
        reclaimed.stdout_text(),
        reclaimed.stderr_text()
    );
    assert_ne!(data(&reclaimed)["lease_id"], orphaned_lease.as_str());
    assert_eq!(data(&reclaimed)["state"], "active");
}

#[test]
fn work_context_admit_revalidates_the_authenticated_capability_after_preparation() {
    let (tmp, state_dir, checkout, capability_file) = init_enforced_admission_session();
    seed_activity_state(
        &state_dir,
        "admission",
        "admission-incarnation",
        "working",
        json!({
            "provider_turn_id": "turn-admit-capability-rotation",
            "started_at": "2030-01-01T00:00:01Z"
        }),
        serde_json::Value::Null,
    );
    let _runtime =
        seed_live_runtime_identity(&state_dir, "admission", "admission-incarnation", 197);
    let active_claim = claim_admission_scope(&tmp, &state_dir, &checkout, &capability_file);
    let targets = tmp.path().join("capability-rotation-targets.json");
    write_private_json(
        &targets,
        &json!({
            "schema_version": "agent-session.operation-targets.v1",
            "targets": [{
                "kind": "path-exact",
                "repository": "example/repository",
                "value": "src/owned/lib.rs"
            }]
        }),
    );
    let execution_token = tmp.path().join("capability-rotation-execution-token");
    fs::write(&execution_token, "capability-rotation-execution-token").expect("execution token");
    fs::set_permissions(&execution_token, fs::Permissions::from_mode(0o600))
        .expect("execution token mode");
    let barrier = tmp.path().join("admit-capability-rotation-barrier");
    fs::create_dir(&barrier).expect("admit barrier");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let claim_revision = active_claim["revision"]
        .as_u64()
        .expect("claim revision")
        .to_string();
    let mut admitting = Command::new(bin::resolve("agent-session"));
    admitting
        .current_dir(&checkout)
        .args([
            "--state-dir",
            state_arg.as_str(),
            "work-context",
            "admit",
            "--session",
            "admission",
            "--claim",
            active_claim["claim_id"].as_str().expect("claim id"),
            "--if-revision",
            claim_revision.as_str(),
            "--targets-file",
            targets.to_str().expect("targets"),
            "--operation",
            "edit",
            "--execution-token-file",
            execution_token.to_str().expect("execution token"),
            "--capability-file",
            capability_file.as_str(),
            "--idempotency-key",
            "admit-capability-rotation-0001",
            "--format",
            "json",
        ])
        .env(
            "NILS_AGENT_SESSION_TEST_CLAIM_BARRIER_STAGE",
            "before_final_authority_lock",
        )
        .env("NILS_AGENT_SESSION_TEST_CLAIM_BARRIER_DIR", &barrier)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let admitting = admitting.spawn().expect("spawn admission");
    wait_for_claim_barrier(&barrier);

    let rotated_capability = "rotated-capability-material-after-admission-preparation";
    fs::write(&capability_file, rotated_capability).expect("rotate capability file");
    fs::set_permissions(&capability_file, fs::Permissions::from_mode(0o600))
        .expect("rotated capability mode");
    rewrite_registry(&state_dir, |registry| {
        registry["brokers"]["admission"]["capability_digest"] = json!(digest(rotated_capability));
    });
    let authorized = run(
        &checkout,
        &[
            "--state-dir",
            state_arg.as_str(),
            "work-context",
            "admit",
            "--session",
            "admission",
            "--claim",
            active_claim["claim_id"].as_str().expect("claim id"),
            "--if-revision",
            claim_revision.as_str(),
            "--targets-file",
            targets.to_str().expect("targets"),
            "--operation",
            "edit",
            "--execution-token-file",
            execution_token.to_str().expect("execution token"),
            "--capability-file",
            capability_file.as_str(),
            "--idempotency-key",
            "admit-capability-rotation-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(
        authorized.code,
        0,
        "stdout={} stderr={}",
        authorized.stdout_text(),
        authorized.stderr_text()
    );
    fs::write(barrier.join("release"), b"release").expect("release admit barrier");

    let output = admitting.wait_with_output().expect("wait admission");
    assert!(!output.status.success());
    let output: serde_json::Value = serde_json::from_slice(&output.stdout).expect("admission json");
    assert_eq!(output["error"]["code"], "coordination-unauthorized");
    assert_eq!(
        load_coordination_registry(&state_dir)["operations"]
            .as_array()
            .expect("operations")
            .len(),
        1
    );
}

#[test]
fn work_context_admit_renews_live_claim_at_final_commit_after_preparation() {
    let (tmp, state_dir, checkout, capability_file) = init_enforced_admission_session();
    seed_activity_state(
        &state_dir,
        "admission",
        "admission-incarnation",
        "working",
        json!({
            "provider_turn_id": "turn-admit-claim-expiry",
            "started_at": "2030-01-01T00:00:01Z"
        }),
        serde_json::Value::Null,
    );
    let _runtime =
        seed_live_runtime_identity(&state_dir, "admission", "admission-incarnation", 198);
    let active_claim = claim_admission_scope(&tmp, &state_dir, &checkout, &capability_file);
    let targets = tmp.path().join("claim-expiry-targets.json");
    write_private_json(
        &targets,
        &json!({
            "schema_version": "agent-session.operation-targets.v1",
            "targets": [{
                "kind": "path-exact",
                "repository": "example/repository",
                "value": "src/owned/lib.rs"
            }]
        }),
    );
    let execution_token = tmp.path().join("claim-expiry-execution-token");
    fs::write(&execution_token, "claim-expiry-execution-token").expect("execution token");
    fs::set_permissions(&execution_token, fs::Permissions::from_mode(0o600))
        .expect("execution token mode");
    let barrier = tmp.path().join("admit-claim-expiry-barrier");
    fs::create_dir(&barrier).expect("admit barrier");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let claim_revision = active_claim["revision"]
        .as_u64()
        .expect("claim revision")
        .to_string();
    let mut admitting = Command::new(bin::resolve("agent-session"));
    admitting
        .current_dir(&checkout)
        .args([
            "--state-dir",
            state_arg.as_str(),
            "work-context",
            "admit",
            "--session",
            "admission",
            "--claim",
            active_claim["claim_id"].as_str().expect("claim id"),
            "--if-revision",
            claim_revision.as_str(),
            "--targets-file",
            targets.to_str().expect("targets"),
            "--operation",
            "edit",
            "--execution-token-file",
            execution_token.to_str().expect("execution token"),
            "--capability-file",
            capability_file.as_str(),
            "--idempotency-key",
            "admit-claim-expiry-0001",
            "--format",
            "json",
        ])
        .env(
            "NILS_AGENT_SESSION_TEST_CLAIM_BARRIER_STAGE",
            "before_final_authority_lock",
        )
        .env("NILS_AGENT_SESSION_TEST_CLAIM_BARRIER_DIR", &barrier)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let admitting = admitting.spawn().expect("spawn admission");
    wait_for_claim_barrier(&barrier);
    let expires_at_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64
        + 1;
    rewrite_registry(&state_dir, |registry| {
        let claim = registry["claims"]
            .as_array_mut()
            .expect("claims")
            .iter_mut()
            .find(|claim| claim["session_id"] == "admission" && claim["state"] == "active")
            .expect("active claim");
        claim["expires_at_epoch"] = json!(expires_at_epoch);
        claim["expires_at"] = json!("1970-01-01T00:00:01Z");
    });
    std::thread::sleep(Duration::from_secs(2));
    fs::write(barrier.join("release"), b"release").expect("release admit barrier");

    let output = admitting.wait_with_output().expect("wait admission");
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let registry = load_coordination_registry(&state_dir);
    assert_eq!(
        registry["operations"].as_array().expect("operations").len(),
        1
    );
    let renewed_claim = registry["claims"]
        .as_array()
        .expect("claims")
        .iter()
        .find(|claim| claim["claim_id"] == active_claim["claim_id"])
        .expect("renewed claim");
    assert_eq!(renewed_claim["state"], "active");
    assert_eq!(renewed_claim["revision"], active_claim["revision"]);
    assert_ne!(renewed_claim["expires_at"], "1970-01-01T00:00:01Z");
    assert!(
        renewed_claim["expires_at_epoch"]
            .as_i64()
            .expect("renewed expiry")
            > expires_at_epoch
    );
}
