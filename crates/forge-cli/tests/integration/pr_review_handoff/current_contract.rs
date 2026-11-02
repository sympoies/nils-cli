//! Current record readers reject omitted writer-owned fields without repairs.
use super::*;
use forge_cli::ops::review_state::{ReviewFindingObservation, ReviewFindingStatus};
use pretty_assertions::assert_eq;

#[test]
fn markerless_first_generation_publication_is_rejected() {
    let seeded = records(handoff(None), Some(HEAD));
    let stub = fixture_with_unbound_native_and_comments(
        HEAD,
        &seeded,
        vec![review(HEAD, "pass")],
        None,
        None,
    );
    let out = check(&stub, HEAD);
    assert_refusal(&out, 65, "awaiting_designated_review");
    assert!(out.stdout.contains("handoff binding"), "{}", out.stdout);
    assert!(out.stdout.contains("reviewer"), "{}", out.stdout);
}

#[test]
fn missing_base_ref_in_retained_ancestor_is_rejected() {
    let mut owner = handoff(None);
    let mut seeded = records(owner.clone(), None);
    owner.base_ref = "main".into();
    owner.assignment_generation = 2;
    owner.base_sha = HEAD.into();
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            HEAD,
            1,
            Some(seeded[0].record_digest.clone()),
            ReviewStatePayload::ReviewHandoff { handoff: owner },
        )
        .unwrap(),
    );
    // Build a synthetic retired ancestor with its original canonical digest;
    // the following current handoff cannot hide it.
    let mut values: Vec<_> = seeded
        .iter()
        .map(|r| serde_json::to_value(r).unwrap())
        .collect();
    values[0]["payload"]["handoff"]
        .as_object_mut()
        .unwrap()
        .remove("base_ref");
    let mut previous = Value::Null;
    for value in &mut values {
        value["previous_digest"] = previous;
        value.as_object_mut().unwrap().remove("record_digest");
        let digest = review_state::sha256_digest(&serde_json::to_vec(value).unwrap());
        value["record_digest"] = json!(digest);
        previous = json!(digest);
    }
    let bodies: Vec<_> = values.iter().map(marker).collect();
    let error = review_state::parse_chain(bodies.iter().map(String::as_str), "acme/widgets", 7)
        .expect_err("every retained handoff must contain base_ref");
    assert_eq!(error.kind(), "review_state_conflict");
    assert!(error.detail().unwrap_or_default().contains("base_ref"));
    assert!(error.message().contains("steward"));
}

fn require_emitted_field(field: &str, finding: bool) {
    let observations = [ReviewFindingObservation {
        fingerprint: "correctness:record:field".into(),
        root_cause_fingerprint: None,
        status: ReviewFindingStatus::Open,
        blocking: true,
        threads: vec![],
    }];
    let state =
        review_state::observe_review_loop(None, HEAD, if finding { &observations } else { &[] })
            .unwrap()
            .state;
    let record = ReviewStateRecord::new(
        "acme/widgets",
        7,
        HEAD,
        0,
        None,
        ReviewStatePayload::ReviewLoop { state },
    )
    .unwrap();
    let mut value = serde_json::to_value(&record).unwrap();
    let target = if finding {
        &mut value["payload"]["state"]["findings"]["correctness:record:field"]
    } else {
        &mut value["payload"]["state"]
    };
    target.as_object_mut().unwrap().remove(field).unwrap();
    let body = marker(&value);
    let error = review_state::parse_chain([body.as_str()], "acme/widgets", 7)
        .expect_err("current writer-owned fields are required");
    assert_eq!(error.kind(), "review_state_conflict");
    assert!(error.detail().unwrap_or_default().contains(field));
    assert!(error.message().contains("steward"));
}

#[test]
fn missing_findings_is_rejected() {
    require_emitted_field("findings", false);
}
#[test]
fn missing_extensions_is_rejected() {
    require_emitted_field("extensions", false);
}
#[test]
fn missing_reopen_count_is_rejected() {
    require_emitted_field("reopen_count", true);
}
#[test]
fn missing_threads_is_rejected() {
    require_emitted_field("threads", true);
}

fn marker(value: &Value) -> String {
    let hex: String = serde_json::to_vec(value)
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("<!-- forge-cli:review-state:v1 {hex} -->")
}
