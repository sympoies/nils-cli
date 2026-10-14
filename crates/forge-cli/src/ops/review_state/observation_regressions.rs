use super::*;
use pretty_assertions::assert_eq;

fn observations(status: ReviewFindingStatus) -> Vec<ReviewFindingObservation> {
    (1..=5)
        .map(|index| ReviewFindingObservation {
            fingerprint: format!("correctness:inventory:f{index}"),
            root_cause_fingerprint: None,
            blocking: true,
            status,
            threads: Vec::new(),
        })
        .collect()
}

#[test]
fn new_head_partial_observation_requires_complete_inventory() {
    let initial = observations(ReviewFindingStatus::Open);
    let previous = observe_review_loop(None, "head-1", &initial).unwrap().state;
    let snapshot = previous.clone();
    let mut fixed = initial[4].clone();
    fixed.status = ReviewFindingStatus::Fixed;

    let error = observe_review_loop(Some(&previous), "head-2", &[fixed])
        .expect_err("omitted findings need an explicit current disposition");
    assert_eq!(error.kind(), "review_observation_incomplete");
    for index in 1..=4 {
        assert!(
            error
                .detail()
                .unwrap()
                .contains(&format!("correctness:inventory:f{index}"))
        );
    }
    assert_eq!(previous, snapshot);
}

#[test]
fn an_explicitly_fixed_finding_still_requires_reopen_budget() {
    let initial = observations(ReviewFindingStatus::Open);
    let previous = observe_review_loop(None, "head-1", &initial).unwrap().state;
    let fixed = observe_review_loop(
        Some(&previous),
        "head-2",
        &observations(ReviewFindingStatus::Fixed),
    )
    .unwrap()
    .state;

    let error = observe_review_loop(Some(&fixed), "head-3", &initial)
        .expect_err("a real reopen must retain the zero-reopen fence");
    assert_eq!(error.kind(), "review_finding_reopened");
    assert_eq!(fixed.budget.max_auto_reopens_per_fingerprint, 0);
}
