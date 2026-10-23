use super::*;
use crate::backend::BackendSuccess;
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use std::cell::RefCell;

const UPSTREAM: &str = "cccccccccccccccccccccccccccccccccccccccc";
const TREE: &str = "dddddddddddddddddddddddddddddddddddddddd";
const BLOB: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

struct ImportFixture {
    body: String,
    files: Vec<Value>,
    tree: Value,
    calls: RefCell<Vec<String>>,
    truncate: bool,
    commit: String,
}

impl ImportFixture {
    fn new(count: usize) -> Self {
        let content = "x".repeat(11_600);
        let files = (0..count)
            .map(|n| {
                json!({
                    "filename": format!("vendor/tests/import{n}.test.ts"), "status": "added",
                    "sha": BLOB, "additions": 1, "deletions": 0,
                    "patch": format!("@@ -0,0 +1 @@\n+{content}")
                })
            })
            .collect::<Vec<_>>();
        let tree = json!({"sha": TREE, "truncated": false,
            "tree": (0..count).map(|n| json!({"path": format!("tests/import{n}.test.ts"),
                "type": "blob", "mode": "100644", "sha": BLOB})).collect::<Vec<_>>()});
        Self {
            body: format!(
                "## Provenance import\n{{\"repository\":\"acme/upstream\",\"commit\":\"{UPSTREAM}\",\"mappings\":[{{\"destination\":\"vendor/\",\"source\":\"\"}}],\"adaptations\":[]}}\n"
            ),
            files,
            tree,
            calls: RefCell::new(Vec::new()),
            truncate: false,
            commit: UPSTREAM.into(),
        }
    }

    fn ctx(&self) -> ProviderContext {
        ProviderContext {
            provider: crate::provider::Provider::GitHub,
            host: "github.com".into(),
            source: crate::provider::DetectionSource::Flag,
            repo: Some("acme/widgets".into()),
        }
    }

    fn inspect(&self) -> Result<(Subject, String, Vec<diff::Finding>), ForgeError> {
        comparison(self, &self.ctx(), 7, HEAD)
    }
}

impl BackendRunner for ImportFixture {
    fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
        let argv = call.plan_argv();
        let endpoint = argv.iter().find(|arg| arg.starts_with("repos/")).unwrap();
        self.calls.borrow_mut().push(endpoint.clone());
        let stdout = if endpoint.contains("/files?") {
            let page: usize = endpoint.rsplit("page=").next().unwrap().parse().unwrap();
            let start = (page - 1) * 100;
            let end = (start + 100).min(self.files.len());
            let end = if self.truncate {
                end.saturating_sub(1).max(start)
            } else {
                end
            };
            serde_json::to_string(&self.files[start.min(end)..end]).unwrap()
        } else if endpoint.contains("/git/commits/") {
            json!({"sha": self.commit, "tree": {"sha": TREE}}).to_string()
        } else if endpoint.contains("/git/trees/") {
            self.tree.to_string()
        } else if endpoint.contains("/compare/")
            && argv
                .iter()
                .any(|arg| arg == "{sha: .merge_base_commit.sha}")
        {
            json!({"sha": BASE}).to_string()
        } else if endpoint.contains("/compare/") {
            self.files.iter().map(|file| format!("diff --git a/{0} b/{0}\nnew file mode 100644\n--- /dev/null\n+++ b/{0}\n{1}\n", file["filename"].as_str().unwrap(), file["patch"].as_str().unwrap())).collect()
        } else {
            json!({"head": HEAD, "base": BASE, "author": "contributor", "body": self.body,
                "changed_files": self.files.len()})
            .to_string()
        };
        Ok(BackendSuccess {
            stdout,
            stderr: String::new(),
        })
    }
}

#[test]
fn complete_large_identical_import_is_admitted_only_after_independent_confirmation() {
    let fixture = ImportFixture::new(625);
    assert!(
        fixture
            .files
            .iter()
            .map(|f| f["patch"].as_str().unwrap().len())
            .sum::<usize>()
            > 7_200_000
    );
    let (subject, binding, findings) = fixture.inspect().unwrap();
    assert!(findings.is_empty());
    assert!(binding.contains("sha256:"));
    assert!(
        !fixture
            .calls
            .borrow()
            .iter()
            .any(|call| call.contains("/compare/"))
    );
    assert_eq!(
        ensure(&fixture, &fixture.ctx(), 7, HEAD, "reviewer", "")
            .unwrap_err()
            .kind(),
        "test_changes_unconfirmed"
    );
    let report = format!("Test changes confirmed: {}", digest(&subject, &binding));
    ensure(&fixture, &fixture.ctx(), 7, HEAD, "reviewer", &report).unwrap();
    assert_eq!(
        ensure(&fixture, &fixture.ctx(), 7, HEAD, "contributor", &report)
            .unwrap_err()
            .kind(),
        "test_changes_unconfirmed"
    );
}

#[test]
fn declared_adaptation_remains_visible_and_needs_normal_declaration() {
    let mut fixture = ImportFixture::new(625);
    fixture.files[0]["sha"] = json!(HEAD);
    fixture.files[0]["patch"] = json!("@@ -0,0 +1 @@\n+test.skip('contract', () => {});");
    fixture.body = fixture.body.replace(
        "\"adaptations\":[]",
        "\"adaptations\":[\"vendor/tests/import0.test.ts\"]",
    );
    let (_, _, findings) = fixture.inspect().unwrap();
    assert_eq!(
        findings,
        vec![
            diff::Finding {
                path: "vendor/tests/import0.test.ts".into(),
                kind: "import-adaptation"
            },
            diff::Finding {
                path: "vendor/tests/import0.test.ts".into(),
                kind: "new-skip"
            }
        ]
    );
    assert_eq!(
        ensure(&fixture, &fixture.ctx(), 7, HEAD, "reviewer", "")
            .unwrap_err()
            .kind(),
        "test_changes_undeclared"
    );
}

#[test]
fn incomplete_provider_enumeration_and_upstream_tree_are_refused() {
    let mut fixture = ImportFixture::new(625);
    fixture.truncate = true;
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "test_changes_incomplete"
    );
    fixture.truncate = false;
    fixture.tree["truncated"] = json!(true);
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "provenance_import_incomplete"
    );
}

#[test]
fn wrong_upstream_commit_missing_blob_and_undeclared_adaptation_are_refused() {
    let mut fixture = ImportFixture::new(1);
    fixture.commit = BASE.into();
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "provenance_import_mismatch"
    );
    fixture.commit = UPSTREAM.into();
    fixture.tree["tree"] = json!([]);
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "provenance_import_mismatch"
    );
    fixture.tree["tree"] =
        json!([{"path":"tests/import0.test.ts","type":"blob","mode":"100644","sha":HEAD}]);
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "provenance_import_mismatch"
    );
}

#[test]
fn undeclared_large_import_keeps_existing_comparison_limits() {
    let mut fixture = ImportFixture::new(625);
    fixture.body.clear();
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "test_changes_limit_exceeded"
    );
}

#[test]
fn provenance_keeps_residual_file_and_byte_limits() {
    let mut fixture = ImportFixture::new(300);
    let paths: Vec<_> = fixture
        .files
        .iter()
        .map(|file| file["filename"].clone())
        .collect();
    fixture.body = fixture.body.replace(
        "\"adaptations\":[]",
        &format!("\"adaptations\":{}", json!(paths)),
    );
    for file in &mut fixture.files {
        file["sha"] = json!(HEAD);
    }
    assert_limit(fixture.inspect().unwrap_err(), "file_count");
    let mut fixture = ImportFixture::new(1);
    fixture.files[0]["sha"] = json!(HEAD);
    fixture.files[0]["patch"] = json!(format!("@@ -0,0 +1 @@\n+{}", "x".repeat(4 * 1024 * 1024)));
    fixture.body = fixture.body.replace(
        "\"adaptations\":[]",
        "\"adaptations\":[\"vendor/tests/import0.test.ts\"]",
    );
    assert_limit(fixture.inspect().unwrap_err(), "diff_bytes");
}

#[test]
fn duplicate_paths_missing_residual_patches_and_whole_hunk_truncation_fail_closed() {
    let mut fixture = ImportFixture::new(2);
    fixture.files[1] = fixture.files[0].clone();
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "test_changes_incomplete"
    );
    let mut fixture = ImportFixture::new(1);
    fixture.files[0]["status"] = json!("modified");
    fixture.files[0]["patch"] = Value::Null;
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "test_changes_incomplete"
    );
    fixture.files[0]["patch"] = json!("@@ -1 +1 @@\n-assert!(a);\n+assert!(b);");
    fixture.files[0]["deletions"] = json!(1);
    fixture.files[0]["additions"] = json!(2);
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "test_changes_incomplete"
    );
}

#[test]
fn existing_modified_and_deleted_tests_cannot_be_omitted_by_import_mappings() {
    let mut fixture = ImportFixture::new(2);
    fixture.files[0] = json!({"filename":"vendor/tests/import0.test.ts", "status":"modified",
        "sha":BLOB, "additions":1, "deletions":1, "patch":"@@ -1 +1 @@\n-assert(value === 42);\n+assert(value);"});
    fixture.files[1] = json!({"filename":"vendor/tests/import1.test.ts", "status":"removed",
        "sha":BLOB, "additions":0, "deletions":1, "patch":"@@ -1 +0,0 @@\n-// shared test helper"});
    let (_, _, findings) = fixture.inspect().unwrap();
    assert_eq!(
        findings,
        vec![
            diff::Finding {
                path: "vendor/tests/import0.test.ts".into(),
                kind: "assertion-change"
            },
            diff::Finding {
                path: "vendor/tests/import1.test.ts".into(),
                kind: "deleted-test"
            }
        ]
    );
}

#[test]
fn verified_blob_and_declaration_changes_expire_confirmation() {
    let mut fixture = ImportFixture::new(1);
    let (subject, binding, _) = fixture.inspect().unwrap();
    let report = format!("Test changes confirmed: {}", digest(&subject, &binding));
    let original = provenance::summary(&binding).unwrap()["manifest_digest"].clone();
    fixture.files[0]["sha"] = json!(HEAD);
    fixture.tree["tree"][0]["sha"] = json!(HEAD);
    let (_, binding, _) = fixture.inspect().unwrap();
    assert_ne!(
        provenance::summary(&binding).unwrap()["manifest_digest"],
        original
    );
    assert_eq!(
        ensure(&fixture, &fixture.ctx(), 7, HEAD, "reviewer", &report)
            .unwrap_err()
            .kind(),
        "test_changes_unconfirmed"
    );
    fixture.files[0]["sha"] = json!(BLOB);
    fixture.tree["tree"][0]["sha"] = json!(BLOB);
    fixture.body.push('\n');
    assert_eq!(
        ensure(&fixture, &fixture.ctx(), 7, HEAD, "reviewer", &report)
            .unwrap_err()
            .kind(),
        "test_changes_unconfirmed"
    );
}

#[test]
fn invalid_or_example_declarations_cannot_activate_an_import_exemption() {
    let mut fixture = ImportFixture::new(1);
    let original = fixture.body.clone();
    fixture.body = format!("```markdown\n{original}```\n");
    fixture.inspect().unwrap();
    assert!(
        fixture
            .calls
            .borrow()
            .iter()
            .any(|call| call.contains("/compare/"))
    );
    for body in [
        original.repeat(2),
        original.replace(UPSTREAM, "main"),
        original.replace("vendor/", "../vendor/"),
        original.replace("acme/upstream", "acme/upstream?ref=main"),
        original.replace(
            "\"source\":\"\"}",
            "\"source\":\"\"},{\"destination\":\"vendor/tests/\",\"source\":\"\"}",
        ),
    ] {
        fixture.body = body;
        assert_eq!(
            fixture.inspect().unwrap_err().kind(),
            "provenance_import_invalid"
        );
    }
}

fn added_fixture(count: usize) -> ImportFixture {
    let mut fixture = ImportFixture::new(count);
    fixture.tree["tree"] = json!([]);
    let paths: Vec<_> = fixture
        .files
        .iter()
        .map(|file| file["filename"].clone())
        .collect();
    fixture.body = fixture.body.replace(
        "\"adaptations\":[]",
        &format!("\"adaptations\":[],\"additions\":{}", json!(paths)),
    );
    fixture
}

#[test]
fn declared_additions_retain_skip_detection_patch_completeness_and_residual_limits() {
    let mut fixture = added_fixture(1);
    fixture.files[0]["patch"] = json!("@@ -0,0 +1 @@\n+test.skip('contract', () => {});");
    let (_, _, findings) = fixture.inspect().unwrap();
    assert_eq!(
        findings,
        vec![
            diff::Finding {
                path: "vendor/tests/import0.test.ts".into(),
                kind: "import-addition"
            },
            diff::Finding {
                path: "vendor/tests/import0.test.ts".into(),
                kind: "new-skip"
            },
        ]
    );
    fixture.files[0]["patch"] = Value::Null;
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "test_changes_incomplete"
    );
    fixture.files[0]["patch"] = json!(format!("@@ -0,0 +1 @@\n+{}", "x".repeat(4 * 1024 * 1024)));
    assert_limit(fixture.inspect().unwrap_err(), "diff_bytes");
    assert_limit(added_fixture(300).inspect().unwrap_err(), "file_count");
}

#[test]
fn additions_must_be_exact_unique_new_mapped_paths_not_adaptations() {
    let original = added_fixture(1).body;
    for body in [
        original.replace(
            "\"additions\":[\"vendor/tests/import0.test.ts\"]",
            "\"additions\":[\"vendor/tests/import0.test.ts\",\"vendor/tests/import0.test.ts\"]",
        ),
        original.replace(
            "\"additions\":[\"vendor/tests/import0.test.ts\"]",
            "\"additions\":[\"outside/NOTICE.txt\"]",
        ),
        original.replace(
            "\"additions\":[\"vendor/tests/import0.test.ts\"]",
            "\"additions\":[\"vendor/../NOTICE.txt\"]",
        ),
        original.replace(
            "\"adaptations\":[]",
            "\"adaptations\":[\"vendor/tests/import0.test.ts\"]",
        ),
    ] {
        let mut fixture = added_fixture(1);
        fixture.body = body;
        assert_eq!(
            fixture.inspect().unwrap_err().kind(),
            "provenance_import_invalid"
        );
    }
    let mut fixture = added_fixture(1);
    fixture.body = fixture.body.replace("import0.test.ts", "missing.test.ts");
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "provenance_import_mismatch"
    );
    let mut fixture = added_fixture(1);
    fixture.tree["tree"] =
        json!([{"path":"tests/import0.test.ts","type":"blob","mode":"100644","sha":BLOB}]);
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "provenance_import_mismatch"
    );
    let mut fixture = added_fixture(1);
    fixture.files[0]["status"] = json!("modified");
    assert_eq!(
        fixture.inspect().unwrap_err().kind(),
        "provenance_import_mismatch"
    );
}
