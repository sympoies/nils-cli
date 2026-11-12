use super::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

// Translate the small raw-diff fixtures into the PR-files API representation.
pub(super) fn files(patch: &str) -> Vec<Value> {
    patch
        .split("diff --git a/")
        .skip(1)
        .map(|section| {
            let (header, rest) = section.split_once('\n').unwrap();
            let (old, new) = header.split_once(" b/").unwrap();
            let hunks = rest
                .find("@@ ")
                .map(|start| rest[start..].trim_end_matches('\n'));
            let status = if rest.starts_with("deleted file mode") {
                "removed"
            } else if old != new {
                "renamed"
            } else {
                "modified"
            };
            let mut file = json!({"filename": new, "status": status, "sha": HEAD,
            "additions": hunks.unwrap_or("").lines().filter(|line| line.starts_with('+')).count(),
            "deletions": hunks.unwrap_or("").lines().filter(|line| line.starts_with('-')).count(),
            "patch": hunks});
            if status == "renamed" {
                file["previous_filename"] = json!(old);
            }
            file
        })
        .collect()
}

pub(super) fn page(files: &[Value], endpoint: &str, argv: &[String]) -> Value {
    let page: usize = endpoint.rsplit("page=").next().unwrap().parse().unwrap();
    let size = if endpoint.contains("per_page=1&") {
        1
    } else {
        100
    };
    let start = (page - 1) * size;
    let mut batch = files[start..(start + size).min(files.len())].to_vec();
    if argv.iter().any(|arg| arg == "map(del(.patch))") {
        for file in &mut batch {
            file.as_object_mut().unwrap().remove("patch");
        }
    }
    json!(batch)
}

#[test]
fn ordinary_binary_assets_are_metadata_only_and_inline_tests_remain_inspected() {
    let patch = format!(
        "diff --git a/evidence/asset.png b/evidence/asset.png\nBinary files a/evidence/asset.png and b/evidence/asset.png differ\n{}",
        patch("src/lib.rs", "#[test]\nfn contract() {}", "")
    );
    let metadata = subject(HEAD, BASE, "");
    assert_eq!(
        admit(metadata.clone(), metadata, &patch, "")
            .unwrap_err()
            .kind(),
        "test_changes_undeclared"
    );
}

#[test]
fn ordinary_oversized_test_patches_name_the_paths_without_source_text() {
    for count in [1, 2] {
        let size = 4 * 1024 * 1024 / count;
        let patch = (0..count)
            .map(|index| patch(&format!("tests/large{index}.rs"), &"x".repeat(size), ""))
            .collect::<String>();
        let metadata = subject(HEAD, BASE, "");
        let error = admit(metadata.clone(), metadata, &patch, "").unwrap_err();
        assert_eq!(error.kind(), "test_changes_limit_exceeded");
        assert!(
            error
                .to_string()
                .contains(&format!("tests/large{}.rs", count - 1)),
            "{error}"
        );
        assert!(!error.to_string().contains(&"x".repeat(100)), "{error}");
        assert_limit(error, "diff_bytes");
    }
}

const INLINE_TESTS: &str = "#[test]\nfn keeps_first() {}\n\n#[test]\nfn drops_second() {}\n\n#[tokio::test]\nasync fn drops_third() {}";

fn inline_deletion_patch() -> String {
    patch("src/lib.rs", INLINE_TESTS, "#[test]\nfn keeps_first() {}")
}

#[test]
fn inline_test_deletions_are_reported_one_row_per_deleted_test() {
    let kinds: Vec<String> = diff::detect(&inline_deletion_patch())
        .unwrap()
        .iter()
        .map(|finding| finding.kind.to_string())
        .collect();
    assert_eq!(
        kinds,
        vec!["deleted-test:drops_second", "deleted-test:drops_third"]
    );
}

#[test]
fn an_undeclared_inline_test_deletion_fails_even_when_a_sibling_is_declared() {
    let patch = inline_deletion_patch();
    let row = |name: &str| {
        format!("- src/lib.rs | deleted-test:{name} | Replacement keeps the removed contract.")
    };
    let both = format!(
        "## Test changes\n{}\n{}",
        row("drops_second"),
        row("drops_third")
    );
    let metadata = subject(HEAD, BASE, &both);
    admit(
        metadata.clone(),
        metadata,
        &patch,
        &confirmation(&patch, BASE, HEAD, &both),
    )
    .unwrap();
    let one = format!("## Test changes\n{}", row("drops_second"));
    let metadata = subject(HEAD, BASE, &one);
    assert_eq!(
        admit(
            metadata.clone(),
            metadata,
            &patch,
            &confirmation(&patch, BASE, HEAD, &one)
        )
        .unwrap_err()
        .kind(),
        "test_changes_undeclared"
    );
}

fn deleted_kinds(path: &str, before: &str, after: &str) -> Vec<String> {
    diff::detect(&patch(path, before, after))
        .unwrap()
        .iter()
        .map(|finding| finding.kind.to_string())
        .collect()
}

#[test]
fn one_line_test_forms_are_named_by_their_own_function() {
    let tokio = "#[tokio::test] async fn keeps() {}\n#[tokio::test] async fn drops_one() {}\n#[tokio::test] async fn drops_two() {}";
    assert_eq!(
        deleted_kinds("src/lib.rs", tokio, "#[tokio::test] async fn keeps() {}"),
        vec!["deleted-test:drops_one", "deleted-test:drops_two"]
    );
    let plain = "#[test] fn a1() {}\n#[test] fn a2() {}\n#[test] fn a3() {}";
    assert_eq!(
        deleted_kinds("src/lib.rs", plain, "#[test] fn a1() {}"),
        vec!["deleted-test:a2", "deleted-test:a3"]
    );
}

#[test]
fn javascript_cases_are_named_by_their_full_title() {
    let before = "it('a: x', () => {});\nit('a: y', () => {});\nit('a: z', () => {});";
    assert_eq!(
        deleted_kinds("tests/api.test.ts", before, "it('a: x', () => {});"),
        vec!["deleted-test:it('a:y'", "deleted-test:it('a:z'"]
    );
}
