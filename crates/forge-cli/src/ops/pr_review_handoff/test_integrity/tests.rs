use super::*;
use pretty_assertions::assert_eq;

#[path = "provenance_tests.rs"]
mod provenance_tests;

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
fn skip_markers_require_standalone_identifiers() {
    for line in [
        "sys.exit(1)",
        "raise SystemExit(2)",
        "os._exit(0)",
        "sys.dont_write_bytecode = True",
        "prefix_xit('contract')",
        "$xit('contract')",
        "éxit('contract')",
        "myit.skip('contract')",
        "mytest.skip('contract')",
        "mydescribe.skip('contract')",
        "prefix_xtest('contract')",
        "prefix_xdescribe('contract')",
    ] {
        assert_eq!(
            kinds(&patch("tests/test_sample.py", "", line)),
            Vec::<&str>::new(),
            "{line}"
        );
    }
    for line in [
        "xit('contract')",
        "it.skip('contract')",
        "xdescribe('contract')",
        "xtest('contract')",
        "test.skip('contract')",
        "describe.skip('contract')",
        "return xit ('contract')",
        "await it . skip ('contract')",
        "{ xdescribe('contract') }",
    ] {
        assert_eq!(
            kinds(&patch("api.test.ts", "", line)),
            vec!["new-skip"],
            "{line}"
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

#[test]
fn renamed_test_sources_require_deletion_declarations_for_the_old_path() {
    for destination in [
        "api_check.py",
        "test_api.py.disabled",
        "tests/disabled/test_api.py",
    ] {
        let patch = format!(
            "diff --git a/tests/test_api.py b/{destination}\nsimilarity index 100%\nrename from tests/test_api.py\nrename to {destination}\n"
        );
        assert_eq!(
            diff::detect(&patch).unwrap(),
            vec![diff::Finding {
                path: "tests/test_api.py".into(),
                kind: "deleted-test",
            }]
        );
    }
}

#[test]
fn javascript_module_test_sources_include_all_module_extensions() {
    for extension in ["cjs", "mjs", "cts", "mts"] {
        let path = format!("api.test.{extension}");
        assert_eq!(
            kinds(&patch(&path, "test('contract', () => {});", "")),
            vec!["deleted-test"]
        );
        let deleted = format!(
            "diff --git a/{path} b/{path}\ndeleted file mode 100644\n--- a/{path}\n+++ /dev/null\n@@ -1 +0,0 @@\n-// no recognizable case signature\n"
        );
        assert_eq!(kinds(&deleted), vec!["deleted-test"]);
    }
}

#[test]
fn rust_module_removal_and_conditional_disabling_require_declarations() {
    for module in ["#[cfg(test)]\nmod tests;", "#[cfg(test)] mod tests;"] {
        assert_eq!(
            kinds(&patch("src/lib.rs", module, "")),
            vec!["deleted-test"]
        );
    }
    for marker in [
        "#![cfg(any())]",
        "#[cfg(any())]",
        "#[cfg_attr(feature = \"slow\", ignore)]",
    ] {
        assert!(
            kinds(&patch(
                "src/lib.rs",
                "#[test]\nfn contract() {}",
                &format!("{marker}\n#[test]\nfn contract() {{}}")
            ))
            .contains(&"new-skip"),
            "{marker}"
        );
    }
}

#[test]
fn deleted_test_file_without_case_signature_is_still_a_deleted_test() {
    assert_eq!(
        kinds(
            "diff --git a/tests/helper.rs b/tests/helper.rs\ndeleted file mode 100644\n--- a/tests/helper.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-// shared test setup\n"
        ),
        vec!["deleted-test"]
    );
}

#[test]
fn a_truncated_middle_file_cannot_be_hidden_by_a_complete_last_file() {
    let truncated =
        "diff --git a/tests/a.rs b/tests/a.rs\n@@ -1,2 +1,1 @@\n-assert!(a);\n+assert!(b);\n";
    assert_eq!(
        diff::detect(&format!(
            "{truncated}{}",
            patch("src/lib.rs", "", "fn f() {}")
        ))
        .unwrap_err()
        .kind(),
        "test_changes_incomplete"
    );
}

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BASE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const BODY: &str = "## Test changes\n- tests/sample.rs | assertion-change | Replacement retains the supported contract.";

struct ComparisonRunner {
    subjects: std::cell::RefCell<std::collections::VecDeque<serde_json::Value>>,
    patch: String,
}

fn subject(head: &str, base: &str, body: &str) -> serde_json::Value {
    serde_json::json!({"head": head, "base": base, "author": "contributor", "body": body})
}

impl BackendRunner for ComparisonRunner {
    fn run(&self, call: &BackendCall) -> Result<crate::backend::BackendSuccess, ForgeError> {
        let argv = call.plan_argv();
        let stdout = if argv.iter().any(|s| s.contains("/compare/")) {
            assert!(
                argv.iter()
                    .any(|s| s.contains("application/vnd.github.diff"))
            );
            self.patch.clone()
        } else {
            self.subjects
                .borrow_mut()
                .pop_front()
                .expect("unexpected metadata read")
                .to_string()
        };
        Ok(crate::backend::BackendSuccess {
            stdout,
            stderr: String::new(),
        })
    }
}

fn admit(
    first: serde_json::Value,
    second: serde_json::Value,
    patch: &str,
    report: &str,
) -> Result<(), ForgeError> {
    let runner = ComparisonRunner {
        subjects: std::cell::RefCell::new([first, second].into()),
        patch: patch.into(),
    };
    let ctx = crate::provider::ProviderContext {
        provider: crate::provider::Provider::GitHub,
        host: "github.com".into(),
        source: crate::provider::DetectionSource::Flag,
        repo: Some("acme/widgets".into()),
    };
    ensure(&runner, &ctx, 7, HEAD, "independent-reviewer", report)
}

fn assertion_patch() -> String {
    patch(
        "tests/sample.rs",
        "assert_eq!(value, 42);",
        "assert!(value > 0);",
    )
}

fn confirmation(patch: &str, base: &str, head: &str, body: &str) -> String {
    format!(
        "Test changes confirmed: {}",
        sha256_digest(format!("{base}\n{head}\n{patch}\n{}", declarations(body)).as_bytes())
    )
}

#[test]
fn confirmation_binds_the_patch_head_and_base() {
    let patch = assertion_patch();
    let metadata = subject(HEAD, BASE, BODY);
    let confirmed = confirmation(&patch, BASE, HEAD, BODY);
    admit(metadata.clone(), metadata.clone(), &patch, &confirmed).unwrap();
    for (patch, base, head) in [
        (patch.replace("value > 0", "value >= 0"), BASE, HEAD),
        (
            patch.clone(),
            "cccccccccccccccccccccccccccccccccccccccc",
            HEAD,
        ),
        (
            patch.clone(),
            BASE,
            "dddddddddddddddddddddddddddddddddddddddd",
        ),
    ] {
        let stale = confirmation(&patch, base, head, BODY);
        assert_eq!(
            admit(
                metadata.clone(),
                metadata.clone(),
                &assertion_patch(),
                &stale
            )
            .unwrap_err()
            .kind(),
            "test_changes_unconfirmed"
        );
    }
}

#[test]
fn admission_rejects_head_mismatch_and_every_metadata_reread_drift() {
    let patch = assertion_patch();
    let metadata = subject(HEAD, BASE, BODY);
    let report = confirmation(&patch, BASE, HEAD, BODY);
    let wrong_head = subject(BASE, BASE, BODY);
    assert_eq!(
        admit(wrong_head.clone(), wrong_head, &patch, &report)
            .unwrap_err()
            .kind(),
        "test_changes_incomplete"
    );
    for field in ["head", "base", "author", "body"] {
        let mut changed = metadata.clone();
        changed[field] = serde_json::json!("changed");
        assert_eq!(
            admit(metadata.clone(), changed, &patch, &report)
                .unwrap_err()
                .kind(),
            "test_changes_incomplete",
            "{field}"
        );
    }
}

#[test]
fn declarations_must_match_kind_and_minimum_reason_length() {
    let patch = assertion_patch();
    for body in [
        BODY.replace("assertion-change", "deleted-test"),
        BODY.replace("Replacement retains the supported contract.", "too short"),
    ] {
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

fn assert_limit(error: ForgeError, limit: &str) {
    assert_eq!(error.kind(), "test_changes_limit_exceeded");
    let ForgeError::Validation {
        detail: Some(detail),
        ..
    } = error
    else {
        panic!("expected validation details")
    };
    assert!(detail.contains("\"retryable\":false"), "{detail}");
    assert!(detail.contains(limit), "{detail}");
    assert!(detail.contains("split"), "{detail}");
}

#[test]
fn deterministic_diff_limits_are_non_retryable_and_name_recovery() {
    let file =
        "diff --git a/asset.png b/asset.png\nBinary files a/asset.png and b/asset.png differ\n";
    diff::detect(&file.repeat(299)).unwrap();
    assert_limit(diff::detect(&file.repeat(300)).unwrap_err(), "file_count");
    let mut large = "diff --git a/a.rs b/a.rs\n".to_string();
    large.push_str(&" ".repeat(4 * 1024 * 1024 - large.len()));
    diff::detect(&large).unwrap();
    large.push(' ');
    assert_limit(diff::detect(&large).unwrap_err(), "diff_bytes");
    for path in [
        "\"a/tests/space name.rs\"",
        "a/tests/a.rs\t",
        "a/tests/é.rs",
        "a/tests/a|b.rs",
    ] {
        assert_limit(
            diff::detect(&format!(
                "diff --git a/tests/a.rs b/tests/a.rs\n--- {path}\n"
            ))
            .unwrap_err(),
            "path_bytes",
        );
    }
}

#[test]
fn fifty_declared_and_confirmed_file_kind_pairs_are_admitted() {
    let patch = (0..50)
        .map(|n| patch(&format!("tests/sample{n}.rs"), "assert!(value);", ""))
        .collect::<String>();
    let rows = (0..50)
        .map(|n| {
            format!(
                "- tests/sample{n}.rs | assertion-change | Replacement retains the supported contract.\n"
            )
        })
        .collect::<String>();
    let body = format!("## Test changes\n{rows}");
    let metadata = subject(HEAD, BASE, &body);
    assert_eq!(diff::detect(&patch).unwrap().len(), 50);
    admit(
        metadata.clone(),
        metadata,
        &patch,
        &confirmation(&patch, BASE, HEAD, &body),
    )
    .unwrap();
}

#[test]
fn git_quoted_headers_are_non_retryable_path_limits() {
    assert_limit(
        diff::detect("diff --git \"a/tests/space name.rs\" \"b/tests/space name.rs\"\n")
            .unwrap_err(),
        "path_bytes",
    );
}

#[test]
fn a_rename_with_only_a_quoted_destination_is_a_non_retryable_path_limit() {
    assert_limit(diff::detect("diff --git a/tests/sample.rs \"b/tests/space name.rs\"\nsimilarity index 100%\nrename from tests/sample.rs\nrename to tests/space name.rs\n").unwrap_err(), "path_bytes");
}

#[test]
fn declaration_reason_at_minimum_length_is_admitted() {
    let body = "## Test changes\n- tests/sample.rs | assertion-change | Exactly12chr\n";
    let metadata = subject(HEAD, BASE, body);
    admit(
        metadata.clone(),
        metadata,
        &assertion_patch(),
        &confirmation(&assertion_patch(), BASE, HEAD, body),
    )
    .unwrap();
}
