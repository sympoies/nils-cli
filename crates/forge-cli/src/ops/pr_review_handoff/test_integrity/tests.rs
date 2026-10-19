use super::*;
use pretty_assertions::assert_eq;

fn patch(path: &str, before: &str, after: &str) -> String {
    let old = before.lines().count();
    let new = after.lines().count();
    let lines = before
        .lines()
        .map(|l| format!("-{l}\n"))
        .chain(after.lines().map(|l| format!("+{l}\n")))
        .collect::<String>();
    format!(
        "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,{old} +1,{new} @@\n{lines}"
    )
}

fn kinds(patch: &str) -> Vec<&'static str> {
    diff::detect(patch)
        .unwrap()
        .iter()
        .map(|f| f.kind)
        .collect()
}

#[test]
fn detects_deleted_files_cases_inline_tests_and_weakened_multiline_assertions() {
    let deleted = "diff --git a/tests/test_old.py b/tests/test_old.py\ndeleted file mode 100644\n--- a/tests/test_old.py\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-def test_old():\n-    assert value == 42\n";
    assert_eq!(kinds(deleted), vec!["assertion-change", "deleted-test"]);
    assert_eq!(
        kinds(&patch("src/lib.rs", "#[test]\nfn contract() {}", "")),
        vec!["deleted-test"]
    );
    assert_eq!(
        kinds(&patch(
            "sample_test.go",
            "func TestContract(t *testing.T) {}",
            ""
        )),
        vec!["deleted-test"]
    );
    assert_eq!(
        kinds(&patch("tests/api.test.ts", "it('contract', () => {});", "")),
        vec!["deleted-test"]
    );
    assert_eq!(
        kinds(&patch(
            "src/lib.rs",
            "assert_eq!(\n result,\n 42\n);",
            "assert!(result.is_some());"
        )),
        vec!["assertion-change"]
    );
    assert_eq!(
        kinds(&patch(
            "tests/test_sample.py",
            "assert result == 42",
            "assert result is not None"
        )),
        vec!["assertion-change"]
    );
}

#[test]
fn flags_supported_new_skip_markers() {
    for (path, marker) in [
        ("src/lib.rs", "#[ignore]"),
        ("tests/test_api.py", "@pytest.mark.skipif(True)"),
        ("tests/test_api.py", "@pytest.mark.xfail"),
        ("api.test.ts", "test.skip('contract', () => {});"),
        ("api_test.go", "t.Skip(\"pending\")"),
        ("ApiTest.java", "@Disabled"),
    ] {
        assert!(
            kinds(&patch(path, "", marker)).contains(&"new-skip"),
            "{marker}"
        );
    }
}

#[test]
fn unchanged_assertions_reordering_additions_docs_and_comments_are_clean() {
    for (path, before, after) in [
        (
            "src/lib.rs",
            "assert_eq!(value, 42);",
            "  assert_eq!(value, 42);",
        ),
        (
            "src/lib.rs",
            "assert!(a);\nassert!(b);",
            "assert!(b);\nassert!(a);",
        ),
        ("src/lib.rs", "", "assert_eq!(value, 42);"),
        ("guide.md", "assert!(value);", ""),
        ("src/lib.rs", "// assert!(value);", "// #[ignore]"),
    ] {
        assert_eq!(
            kinds(&patch(path, before, after)),
            Vec::<&str>::new(),
            "{path}"
        );
    }
}

#[test]
fn malformed_truncated_and_binary_comparisons_fail_closed() {
    for patch in [
        "{\"truncated\":true}",
        "diff --git a/tests/a.py b/tests/a.py\n@@ -1,2 +1,1 @@\n-assert True\n+assert False\n",
        "diff --git a/a.rs b/a.rs\nBinary files a/a.rs and b/a.rs differ\n",
    ] {
        assert_eq!(
            diff::detect(patch).unwrap_err().kind(),
            "test_changes_incomplete"
        );
    }
}

#[test]
fn code_fences_cannot_supply_author_declarations() {
    assert_eq!(
        kinds("diff --git a/a.png b/a.png\nBinary files a/a.png and b/a.png differ\n"),
        Vec::<&str>::new()
    );
    assert_eq!(declarations("```md\n## Test changes\n- forged\n```\n"), "");
    assert_eq!(
        declarations("## Test changes\n- valid\n## Other\n- forged"),
        "- valid"
    );
}
