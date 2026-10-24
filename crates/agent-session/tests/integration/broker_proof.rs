//! Isolated authenticated CLI producer/consumer contract; no live session.
use super::coordination::{
    candidate, capability, data, init_checkout, load_coordination_registry, rewrite_registry, run,
    seed_activity_state, seed_brokers_at, seed_live_runtime_identity, write_private_json,
};
use nils_test_support::cmd::CmdOutput;
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture {
    root: tempfile::TempDir,
    state: PathBuf,
    checkout: PathBuf,
    cap: String,
    claim: Value,
    _runtime: super::cli::TestProcessGroup,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let checkout = root.path().join("checkout");
        fs::create_dir(&state).unwrap();
        init_checkout(&checkout, "https://example.invalid/example/repository.git");
        seed_brokers_at(
            &state,
            &[(
                "proof-session",
                "proof-incarnation",
                "proof-private-capability-material-0001",
                &checkout,
                Some("enforce"),
            )],
        );
        seed_activity_state(
            &state,
            "proof-session",
            "proof-incarnation",
            "working",
            json!({"provider_turn_id":"proof-turn", "started_at":"2030-01-01T00:00:01Z"}),
            Value::Null,
        );
        let runtime = seed_live_runtime_identity(&state, "proof-session", "proof-incarnation", 196);
        let cap = capability(&state, "proof-session");
        let file = root.path().join("claim.json");
        candidate(&file, "src/owned/", "proof fixture");
        let result = run(
            &checkout,
            &[
                "--state-dir",
                state.to_str().unwrap(),
                "work-context",
                "claim",
                "--session",
                "proof-session",
                "--capability-file",
                &cap,
                "--file",
                file.to_str().unwrap(),
                "--idempotency-key",
                "proof-claim-0001",
                "--format",
                "json",
            ],
        );
        assert_eq!(result.code, 0, "{}", result.stdout_text());
        let claim = data(&result)["context"].clone();
        let token = root.path().join("execution.token");
        fs::write(&token, "proof-private-execution-token-0001").unwrap();
        fs::set_permissions(token, fs::Permissions::from_mode(0o600)).unwrap();
        write_private_json(
            &root.path().join("targets.json"),
            &json!({
                "schema_version":"agent-session.operation-targets.v1",
                "targets":[{"kind":"path-exact", "repository":"example/repository", "value":"src/owned/lib.rs"}]
            }),
        );
        Self {
            root,
            state,
            checkout,
            cap,
            claim,
            _runtime: runtime,
        }
    }
    fn command(&self, command: &[&str]) -> CmdOutput {
        let mut args = vec!["--state-dir", self.state.to_str().unwrap()];
        args.extend(command);
        run(&self.checkout, &args)
    }
    fn admission(&self, prepare: bool, key: &str) -> Value {
        let r = self.command(&[
            if prepare { "broker" } else { "work-context" },
            if prepare {
                "prepare-admission-proof"
            } else {
                "admit"
            },
            "--session",
            "proof-session",
            "--capability-file",
            &self.cap,
            "--claim",
            self.claim["claim_id"].as_str().unwrap(),
            if prepare {
                "--claim-revision"
            } else {
                "--if-revision"
            },
            "1",
            "--operation",
            "edit",
            "--targets-file",
            self.root.path().join("targets.json").to_str().unwrap(),
            "--execution-token-file",
            self.root.path().join("execution.token").to_str().unwrap(),
            if prepare {
                "--admit-idempotency-key"
            } else {
                "--idempotency-key"
            },
            key,
            "--format",
            "json",
        ]);
        assert_eq!(
            r.code,
            0,
            "exact admission evidence missing: {} {}",
            r.stdout_text(),
            r.stderr_text()
        );
        data(&r)
    }
    fn prepare(&self, key: &str) -> Value {
        let before = fs::read(self.state.join("coordination/registry.json")).unwrap();
        let b = self.admission(true, key);
        assert_eq!(
            fs::read(self.state.join("coordination/registry.json")).unwrap(),
            before
        );
        assert_eq!(
            b["schema_version"],
            "agent-session.broker-admission-proof-preparation.v1"
        );
        assert_eq!(b["selector"].as_object().unwrap().len(), 5);
        b["selector"].clone()
    }
    fn proof_with(
        &self,
        selectors: Vec<Value>,
        inc: &str,
        generation: &str,
        cap: &str,
    ) -> CmdOutput {
        let batch = self.root.path().join("batch.json");
        write_private_json(
            &batch,
            &json!({"schema_version":"agent-session.broker-proof-batch.v1", "selectors":selectors}),
        );
        self.command(&[
            "broker",
            "proof",
            "--session",
            "proof-session",
            "--capability-file",
            cap,
            "--expected-incarnation",
            inc,
            "--expected-generation",
            generation,
            "--batch-file",
            batch.to_str().unwrap(),
            "--format",
            "json",
        ])
    }
    fn proof(&self, selectors: Vec<Value>) -> Value {
        let before = fs::read(self.state.join("coordination/registry.json")).unwrap();
        let r = self.proof_with(selectors.clone(), "proof-incarnation", "1", &self.cap);
        assert_eq!(r.code, 0, "{} {}", r.stdout_text(), r.stderr_text());
        assert_eq!(
            fs::read(self.state.join("coordination/registry.json")).unwrap(),
            before,
            "proof must not renew, reclaim, complete or replay operations"
        );
        let b = data(&r);
        assert_eq!(b["schema_version"], "agent-session.broker-proof.v1");
        assert_eq!(b["session_id"], "proof-session");
        assert_eq!(b["session_incarnation"], "proof-incarnation");
        assert_eq!(b["generation"], 1);
        assert_eq!(b["proofs"].as_array().unwrap().len(), selectors.len());
        for (i, item) in b["proofs"].as_array().unwrap().iter().enumerate() {
            assert_eq!(item["index"], i);
        }
        for private in [
            "proof-private-capability-material",
            "proof-private-execution-token",
            "execution_token_digest",
            "src/owned/lib.rs",
        ] {
            assert!(!b.to_string().contains(private), "private material leaked");
        }
        b
    }
    fn operation(&self, lease: &Value) -> Value {
        json!({"kind":"operation", "lease_id":lease["lease_id"], "revision_floor":lease["revision"],
            "claim_id":self.claim["claim_id"], "claim_revision":1})
    }
    fn complete(&self, lease: &Value) {
        let r = self.command(&[
            "work-context",
            "complete",
            "--session",
            "proof-session",
            "--capability-file",
            &self.cap,
            "--lease",
            lease["lease_id"].as_str().unwrap(),
            "--if-revision",
            "1",
            "--execution-token-file",
            self.root.path().join("execution.token").to_str().unwrap(),
            "--outcome",
            "pass",
            "--idempotency-key",
            "proof-complete-0001",
            "--format",
            "json",
        ]);
        assert_eq!(r.code, 0, "{}", r.stdout_text());
    }
}
#[test]
fn broker_proof_preparation_and_lost_reply_match_consumer_without_admission_replay() {
    let f = Fixture::new();
    let selector = f.prepare("proof-admit-0001");
    assert_eq!(selector["kind"], "admission");
    assert_eq!(selector["admit_idempotency_key"], "proof-admit-0001");
    assert_eq!(selector["claim_id"], f.claim["claim_id"]);
    assert_eq!(selector["claim_revision"], 1);
    assert_eq!(
        selector["expected_request_digest"].as_str().unwrap().len(),
        64
    );
    let unknown = f.proof(vec![selector.clone()]);
    assert_eq!(unknown["proofs"][0]["proof"]["status"], "unknown");
    assert_eq!(unknown["proofs"][0]["proof"]["provenance"], "not_retained");
    let lease = f.admission(false, "proof-admit-0001");
    assert_eq!(f.prepare("proof-admit-0001"), selector);
    let registry = load_coordination_registry(&f.state);
    let receipt = registry["receipts"]
        .as_object()
        .unwrap()
        .values()
        .find(|r| r["operation"] == "work-context-admit")
        .unwrap();
    assert_eq!(selector["expected_request_digest"], receipt["digest"]);
    let b = f.proof(vec![selector, f.operation(&lease)]);
    let p = &b["proofs"][0]["proof"];
    assert_eq!(p["status"], "committed");
    assert_eq!(p["provenance"], "retained_operation");
    assert_eq!(p["retained"], true);
    assert_eq!(p["lease_id"], lease["lease_id"]);
    assert_eq!(p["state"], "active");
    assert_eq!(p["revision"], 1);
    assert_eq!(p["outcome"], Value::Null);
    assert_eq!(
        p["claim"],
        json!({"claim_id":f.claim["claim_id"], "revision":1})
    );
    assert_eq!(b["proofs"][1]["proof"]["found"], true);
    assert_eq!(
        f.admission(false, "proof-admit-0001"),
        lease,
        "released replay remains compatible"
    );
}
#[test]
fn broker_proof_terminal_and_active_items_preserve_unrelated_consumer_records() {
    let f = Fixture::new();
    let first_selector = f.prepare("proof-admit-0001");
    let first = f.admission(false, "proof-admit-0001");
    f.complete(&first);
    let second = f.admission(false, "proof-admit-0002");
    let b = f.proof(vec![
        f.operation(&first),
        f.operation(&second),
        first_selector.clone(),
    ]);
    assert_eq!(b["proofs"][0]["proof"]["state"], "completed");
    assert_eq!(b["proofs"][1]["proof"]["state"], "active");
    assert_eq!(b["proofs"][2]["proof"]["state"], "completed");
    let mut records = vec![first["lease_id"].clone(), second["lease_id"].clone()];
    for item in b["proofs"].as_array().unwrap() {
        let p = &item["proof"];
        if item["ok"] == true
            && p["kind"] == "operation"
            && p["found"] == true
            && p["state"] == "completed"
        {
            records.retain(|id| *id != p["lease_id"]);
        }
    }
    assert_eq!(records, vec![second["lease_id"].clone()]);
    rewrite_registry(&f.state, |r| {
        r["operations"]
            .as_array_mut()
            .unwrap()
            .retain(|op| op["lease_id"] != first["lease_id"])
    });
    let historical = f.proof(vec![first_selector, f.operation(&first)]);
    assert_eq!(historical["proofs"][1]["proof"]["state"], "completed");
    let p = &historical["proofs"][0]["proof"];
    assert_eq!(p["status"], "committed");
    assert_eq!(p["provenance"], "historical_receipt");
    assert_eq!(p["retained"], false);
    for key in ["state", "revision", "outcome"] {
        assert_eq!(p[key], Value::Null);
    }
}
#[test]
fn broker_proof_uncertainty_and_selector_conflicts_are_item_scoped() {
    let f = Fixture::new();
    let admission = f.prepare("proof-admit-0001");
    let lease = f.admission(false, "proof-admit-0001");
    for state in [
        "completing",
        "reconcile_pending",
        "completed",
        "failed",
        "abandoned",
        "expired",
    ] {
        rewrite_registry(&f.state, |r| {
            r["operations"][0]["state"] = json!(state);
            r["operations"][0]["revision"] = json!(2);
        });
        let b = f.proof(vec![admission.clone(), f.operation(&lease)]);
        assert_eq!(b["proofs"][0]["proof"]["state"], state);
        assert_eq!(b["proofs"][1]["proof"]["state"], state);
    }
    let valid = f.operation(&lease);
    let mut floor = valid.clone();
    floor["revision_floor"] = json!(3);
    let mut claim = valid.clone();
    claim["claim_revision"] = json!(2);
    let mut other = valid.clone();
    other["claim_id"] = json!("other-claim");
    let mut digest = admission.clone();
    digest["expected_request_digest"] = json!("0".repeat(64));
    let mut missing = valid.clone();
    missing["lease_id"] = json!("not-retained");
    let b = f.proof(vec![
        floor,
        claim,
        other,
        digest,
        missing,
        json!({"kind":"invalid"}),
        valid,
    ]);
    for i in [0, 1, 2, 3, 5] {
        assert_eq!(b["proofs"][i]["ok"], false);
    }
    assert_eq!(b["proofs"][4]["proof"]["found"], false);
    assert_eq!(b["proofs"][6]["ok"], true);
    rewrite_registry(&f.state, |r| r["operations"] = json!([]));
    let absent = f.proof(vec![admission.clone()]);
    assert_eq!(absent["proofs"][0]["proof"]["status"], "unknown");
    assert_eq!(absent["proofs"][0]["proof"]["provenance"], "not_retained");
    rewrite_registry(&f.state, |r| {
        for receipt in r["receipts"].as_object_mut().unwrap().values_mut() {
            receipt["expires_at_epoch"] = json!(1);
        }
    });
    assert_eq!(
        f.proof(vec![admission])["proofs"][0]["proof"]["status"],
        "unknown"
    );
}
#[test]
fn broker_proof_fences_authentication_incarnation_generation_and_batch_size() {
    let f = Fixture::new();
    let selector = f.prepare("proof-admit-0001");
    let wrong = f.root.path().join("wrong-capability");
    fs::write(&wrong, "different-private-capability-material").unwrap();
    fs::set_permissions(&wrong, fs::Permissions::from_mode(0o600)).unwrap();
    let before = load_coordination_registry(&f.state);
    for (inc, generation, cap) in [
        ("stale", "1", f.cap.as_str()),
        ("proof-incarnation", "2", f.cap.as_str()),
        ("proof-incarnation", "1", wrong.to_str().unwrap()),
    ] {
        let r = f.proof_with(vec![selector.clone()], inc, generation, cap);
        assert_ne!(r.code, 0, "{}", r.stdout_text());
        assert!(r.stdout_json().get("data").is_none());
    }
    assert_ne!(
        f.proof_with(vec![selector; 129], "proof-incarnation", "1", &f.cap)
            .code,
        0
    );
    assert_eq!(load_coordination_registry(&f.state), before);
}

#[test]
fn broker_proof_preparation_pins_released_digest_and_managed_identity() {
    let f = Fixture::new();
    let selector = f.prepare("proof-admit-0001");
    let expected = agent_session::coordination::request_digest(
        "work-context-admit",
        &json!({
            "claim": f.claim["claim_id"], "if_revision":1, "operation":"edit",
            "targets":[{"kind":"path-exact","repository":"example/repository","value":"src/owned/lib.rs"}],
            "provider_targets":[],"checkouts":[],"descendant":null,
            "execution_token_digest":super::coordination::digest("proof-private-execution-token-0001")
        }),
    );
    assert_eq!(selector["expected_request_digest"], expected);
    let result = super::coordination::run_with_env(
        &f.checkout,
        &[
            "--state-dir",
            f.state.to_str().unwrap(),
            "broker",
            "prepare-admission-proof",
            "--claim",
            f.claim["claim_id"].as_str().unwrap(),
            "--claim-revision",
            "1",
            "--operation",
            "edit",
            "--targets-file",
            f.root.path().join("targets.json").to_str().unwrap(),
            "--execution-token-file",
            f.root.path().join("execution.token").to_str().unwrap(),
            "--admit-idempotency-key",
            "proof-admit-0001",
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_ID", "proof-session"),
            ("AGENT_SESSION_CAPABILITY_FILE", &f.cap),
        ],
    );
    assert_eq!(result.code, 0, "{}", result.stdout_text());
    assert_eq!(data(&result)["selector"], selector);
    fs::write(
        f.root.path().join("execution.token"),
        "another-private-execution-token-0001",
    )
    .unwrap();
    assert_ne!(
        f.prepare("proof-admit-0001")["expected_request_digest"],
        expected
    );
}

#[test]
fn broker_proof_expired_active_lease_never_restores_execution_authority() {
    let f = Fixture::new();
    let selector = f.prepare("proof-admit-0001");
    let lease = f.admission(false, "proof-admit-0001");
    rewrite_registry(&f.state, |r| {
        r["operations"][0]["expires_at_epoch"] = json!(1);
    });
    let b = f.proof(vec![selector, f.operation(&lease)]);
    assert_eq!(b["proofs"][0]["proof"]["state"], "completing");
    assert_eq!(b["proofs"][1]["proof"]["state"], "completing");
    assert_eq!(
        load_coordination_registry(&f.state)["operations"][0]["state"],
        "active"
    );
}
