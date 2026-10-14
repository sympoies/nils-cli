use pretty_assertions::assert_eq;

use super::*;

fn assert_phrase_plan(kind: SearchKind) {
    for phrase in [
        "flaky retry",
        "retry   budget",
        "\"exact phrase\"",
        "\"state:closed retry\"",
    ] {
        let call =
            build_github_search_call(kind, "acme/widget", phrase, &[SearchMatchField::Title], 5)
                .expect("valid phrase");
        let plan = call.plan_argv();
        assert_eq!(&plan[1..4], &["search", kind.gh_noun(), phrase]);
        assert_eq!(plan.iter().filter(|arg| arg.as_str() == phrase).count(), 1);
    }
}

#[test]
fn ordinary_issues_phrase_preserves_the_backend_keyword() {
    assert_phrase_plan(SearchKind::Issues);
}

#[test]
fn ordinary_prs_phrase_preserves_the_backend_keyword() {
    assert_phrase_plan(SearchKind::Prs);
}

#[test]
fn structured_query_keeps_separate_qualifiers_and_a_quoted_phrase() {
    let call = build_github_search_call(
        SearchKind::Issues,
        "",
        "repo:acme/widget label:\"needs review\" \"exact phrase\" -state:open",
        &[SearchMatchField::Title],
        5,
    )
    .unwrap();
    let plan = call.plan_argv();
    assert_eq!(
        &plan[plan.len() - 5..],
        &[
            "--",
            "repo:acme/widget",
            "label:needs review",
            "exact phrase",
            "-state:open"
        ]
    );
}
