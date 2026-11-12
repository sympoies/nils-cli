//! Per-test row keys: module paths, retired/stale rows and JavaScript titles.
use super::*;
use pretty_assertions::assert_eq;

fn deleted(path: &str, before: &str, after: &str) -> Vec<String> {
    kinds(&patch(path, before, after))
}

fn detail(error: ForgeError) -> String {
    let ForgeError::Validation {
        detail: Some(detail),
        ..
    } = error
    else {
        panic!("expected validation details")
    };
    detail
}

const TWO_MODULES: &str =
    "mod first {\n    #[test]\n    fn same() {}\n}\nmod second {\n    #[test]\n    fn same() {}\n}";
const FIRST_ONLY: &str = "mod first {\n    #[test]\n    fn same() {}\n}";
const SECOND_ONLY: &str = "mod second {\n    #[test]\n    fn same() {}\n}";

#[test]
fn same_named_rust_tests_in_different_modules_get_separate_rows() {
    assert_eq!(
        deleted("src/lib.rs", TWO_MODULES, FIRST_ONLY),
        vec!["deleted-test:second::same"]
    );
    assert_eq!(
        deleted("src/lib.rs", TWO_MODULES, SECOND_ONLY),
        vec!["deleted-test:first::same"]
    );
    let nested = "mod outer {\n    mod inner {\n        #[test]\n        fn same() {}\n    }\n    #[test]\n    fn same() {}\n}";
    let kept = "mod outer {\n    mod inner {\n        #[test]\n        fn same() {}\n    }\n}";
    assert_eq!(
        deleted("src/lib.rs", nested, kept),
        vec!["deleted-test:outer::same"]
    );
}

#[test]
fn a_row_for_one_module_does_not_cover_the_same_name_in_another() {
    let patch = patch("src/lib.rs", TWO_MODULES, FIRST_ONLY);
    for name in ["same", "first::same"] {
        let body = format!(
            "## Test changes\n- src/lib.rs | deleted-test:{name} | Another test keeps this contract."
        );
        let metadata = subject(HEAD, BASE, &body);
        assert_eq!(
            admit(
                metadata.clone(),
                metadata,
                &patch,
                &confirmation(&patch, BASE, HEAD, &body)
            )
            .unwrap_err()
            .kind(),
            "test_changes_undeclared"
        );
    }
}

#[test]
fn the_retired_file_level_row_and_stale_rows_are_reported() {
    let patch = patch(
        "src/lib.rs",
        "#[test]\nfn keeps() {}\n#[test]\nfn drops() {}",
        "#[test]\nfn keeps() {}",
    );
    let body = "## Test changes\n- src/lib.rs | deleted-test | Another test keeps this contract.\n- src/lib.rs | deleted-test:renamed_away | Another test keeps this contract.";
    let metadata = subject(HEAD, BASE, body);
    let error = admit(
        metadata.clone(),
        metadata,
        &patch,
        &confirmation(&patch, BASE, HEAD, body),
    )
    .unwrap_err();
    assert_eq!(error.kind(), "test_changes_undeclared");
    let detail = detail(error);
    assert!(
        detail.contains("Retired file-level form")
            && detail.contains("- src/lib.rs | deleted-test\\n"),
        "{detail}"
    );
    assert!(
        detail.contains("match no finding") && detail.contains("deleted-test:renamed_away"),
        "{detail}"
    );
    assert!(detail.contains("deleted-test:drops"), "{detail}");
}

#[test]
fn javascript_titles_on_their_own_line_are_extracted() {
    let before = "it(\n  'handles x',\n  () => {},\n);\ntest(\n\n  \"handles y\", () => {});\nit('keeps', () => {});";
    assert_eq!(
        deleted("tests/api.test.ts", before, "it('keeps', () => {});"),
        vec![
            "deleted-test:it('handlesx'",
            "deleted-test:test(\"handlesy\""
        ]
    );
}

#[test]
fn javascript_titles_with_escaped_quotes_are_extracted() {
    let before = "it('it\\'s a', () => {});\nit('it\\'s b', () => {});";
    assert_eq!(
        deleted("tests/api.test.ts", before, "it('it\\'s a', () => {});"),
        vec!["deleted-test:it('it\\'sb'"]
    );
}

#[test]
fn javascript_each_tables_are_one_row_named_by_the_table_title() {
    let before = "it.each([[1, 2]])('adds %i', () => {});\ntest.each([\n  [1, 2],\n  [3, 4],\n])(\n  'sums %i',\n  () => {},\n);\nit.each`\n  a | b\n  ${1} | ${2}\n`('tags $a', () => {});\nit('keeps', () => {});";
    assert_eq!(
        deleted("tests/api.test.ts", before, "it('keeps', () => {});"),
        vec![
            "deleted-test:it.each('adds%i'",
            "deleted-test:it.each('tags$a'",
            "deleted-test:test.each('sums%i'"
        ]
    );
}
