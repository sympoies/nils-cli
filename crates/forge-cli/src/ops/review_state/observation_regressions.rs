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

fn fixed_inventory() -> ReviewLoopState {
    let initial = observe_review_loop(None, "head-1", &observations(ReviewFindingStatus::Open))
        .unwrap()
        .state;
    observe_review_loop(
        Some(&initial),
        "head-2",
        &observations(ReviewFindingStatus::Fixed),
    )
    .unwrap()
    .state
}

#[test]
fn same_head_delta_retains_fixed_findings_when_adding_a_new_finding() {
    let previous = fixed_inventory();
    let mut new_finding = observations(ReviewFindingStatus::Open).remove(0);
    new_finding.fingerprint = "correctness:inventory:new".into();
    let transition = observe_review_loop(Some(&previous), "head-2", &[new_finding]).unwrap();

    assert!(transition.changed);
    assert_eq!(transition.state.round, previous.round);
    assert_eq!(
        transition.state.no_progress_rounds,
        previous.no_progress_rounds
    );
    assert_eq!(transition.state.budget, previous.budget);
    for (fingerprint, finding) in &previous.findings {
        assert_eq!(&transition.state.findings[fingerprint], finding);
    }
    let new = &transition.state.findings["correctness:inventory:new"];
    assert_eq!(new.status, ReviewFindingStatus::Open);
    assert!(new.blocking);
}

#[test]
fn same_head_full_set_retains_fixed_findings_when_adding_a_new_finding() {
    let previous = fixed_inventory();
    let mut full_set = observations(ReviewFindingStatus::Fixed);
    let mut new_finding = observations(ReviewFindingStatus::Open).remove(0);
    new_finding.fingerprint = "correctness:inventory:new".into();
    let delta = observe_review_loop(Some(&previous), "head-2", &[new_finding.clone()]).unwrap();
    full_set.push(new_finding);

    let full = observe_review_loop(Some(&previous), "head-2", &full_set)
        .expect("already fixed rows are retained at the same head");
    assert_eq!(full.state, delta.state);
    assert!(full.changed);
    let retry = observe_review_loop(Some(&full.state), "head-2", &full_set).unwrap();
    assert!(!retry.changed);
    assert_eq!(retry.state, full.state);
}

#[test]
fn same_head_first_fixed_disposition_explains_recovery() {
    for status in [ReviewFindingStatus::Open, ReviewFindingStatus::Accepted] {
        let initial = observations(status);
        let previous = observe_review_loop(None, "head-1", &initial).unwrap().state;
        let error = observe_review_loop(
            Some(&previous),
            "head-1",
            &observations(ReviewFindingStatus::Fixed),
        )
        .expect_err("a first fixed disposition still needs a repaired head");
        assert_eq!(error.kind(), "review_state_conflict");
        assert!(error.message().contains("repaired head"));
        assert!(error.message().contains("already fixed"));
        assert!(error.message().contains("omit"));
    }
}

#[test]
fn same_head_fixed_reassertion_merges_threads_without_advancing_lifecycle() {
    let previous = fixed_inventory();
    let mut fixed = observations(ReviewFindingStatus::Fixed).remove(0);
    fixed.threads = vec!["thread-2".into(), "thread-1".into(), "thread-2".into()];
    let transition = observe_review_loop(Some(&previous), "head-2", &[fixed]).unwrap();
    let mut expected = previous;
    expected
        .findings
        .get_mut("correctness:inventory:f1")
        .unwrap()
        .threads = vec!["thread-1".into(), "thread-2".into()];
    assert!(transition.changed);
    assert_eq!(transition.state, expected);
}
