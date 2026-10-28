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
