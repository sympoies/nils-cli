use super::*;
use pretty_assertions::assert_eq;

#[path = "provenance_import.rs"]
mod provenance_import;

const DIFF: &str = "diff --git a/tests/test_sample.py b/tests/test_sample.py\nindex 1111111..2222222 100644\n--- a/tests/test_sample.py\n+++ b/tests/test_sample.py\n@@ -1,2 +1,3 @@\n-def test_exact():\n-    assert result == 42\n+@pytest.mark.skip(reason='pending')\n+def test_exact():\n+    assert result is not None\n";

fn write_files(stub: &StubEnv, files: &[Value]) {
    for (index, page) in files.chunks(100).enumerate() {
        let metadata = page
            .iter()
            .cloned()
            .map(|mut file| {
                file.as_object_mut().unwrap().remove("patch");
                file
            })
            .collect::<Vec<_>>();
        fs::write(
            stub.tempdir
                .path()
                .join(format!("metadata-page{}.json", index + 1)),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
    }
    for (index, file) in files.iter().enumerate().filter(|(_, file)| {
        file["filename"]
            .as_str()
            .is_some_and(|path| path.ends_with(".py") || path.ends_with(".go"))
    }) {
        fs::write(
            stub.tempdir
                .path()
                .join(format!("patch-page{}.json", index + 1)),
            json!([file]).to_string(),
        )
        .unwrap();
    }
}

fn sample_files(stub: &StubEnv) {
    let patch = DIFF[DIFF.find("@@ ").unwrap()..].trim_end();
    use base64::{Engine, engine::general_purpose::STANDARD};
    use sha1::{Digest, Sha1};

    let mut head = String::new();
    for (name, prefix) in [("base", '-'), ("head", '+')] {
        let source = patch
            .lines()
            .filter_map(|line| line.strip_prefix(prefix))
            .collect::<Vec<_>>()
            .join("\n");
        let mut hash = Sha1::new();
        hash.update(format!("blob {}\0", source.len()));
        hash.update(source.as_bytes());
        let sha = format!("{:x}", hash.finalize());
        let root = stub.tempdir.path();
        fs::write(
            root.join(format!("{name}-content.json")),
            json!({"type":"file", "sha":sha, "size":source.len()}).to_string(),
        )
        .unwrap();
        fs::write(root.join(format!("{sha}.json")),
            json!({"sha":sha, "size": source.len(), "encoding":"base64", "content":STANDARD.encode(source)}).to_string()).unwrap();
        if name == "head" {
            head = sha;
        }
    }
    write_files(
        stub,
        &[
            json!({"filename":"tests/test_sample.py", "status":"modified", "sha":head,
        "additions":3, "deletions":2, "patch":patch}),
        ],
    );
}

fn integrity_fixture(body: &str, confirmation: &str, author: &str) -> StubEnv {
    integrity_fixture_with_ledger(body, confirmation, author, true)
}

fn integrity_fixture_with_ledger(
    body: &str,
    confirmation: &str,
    author: &str,
    ledger: bool,
) -> StubEnv {
    let mut r = review(HEAD, "pass");
    r["body"] = json!(format!("{}\n{confirmation}\n", r["body"].as_str().unwrap()));
    let state = if ledger {
        records(handoff(None), Some(HEAD))
    } else {
        vec![]
    };
    let stub = fixture(HEAD, &state, vec![r]);
    let diff = stub.tempdir.path().join("integrity.diff");
    let metadata = stub.tempdir.path().join("integrity.json");
    fs::write(&diff, DIFF).unwrap();
    fs::write(
        &metadata,
        json!({"head": HEAD, "base": OLD, "author": author, "body": body, "changed_files":1})
            .to_string(),
    )
    .unwrap();
    sample_files(&stub);
    let directory = stub.tempdir.path().to_string_lossy().into_owned();
    stub.env("TEST_INTEGRITY_FILES_DIR", directory)
        .env("TEST_INTEGRITY_DIFF", diff.to_string_lossy())
        .env("TEST_INTEGRITY_METADATA", metadata.to_string_lossy())
}

const DECLARATIONS: &str = "## Test changes\n\n- tests/test_sample.py | assertion-change | Broader assertion retains the supported result contract.\n- allow tests/test_sample.py | new-skip | Intentional quarantine with an independently validated replacement.\n";

#[test]
fn inspect_keeps_assigned_handoff_when_declarations_are_missing_or_malformed() {
    for body in [
        "",
        "## Test changes\ntests/test_sample.py | assertion-change | Missing list marker.\n",
    ] {
        let stub = integrity_fixture(body, "", "contributor");
        let out = inspect_handoff(&stub);
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        let data = parse_envelope(&out.stdout);
        assert_eq!(data["data"]["status"], "awaiting-designated-review");
        assert!(data["data"].get("approval").is_none());
        assert_eq!(data["data"]["handoff"]["assigned_head"], HEAD);
        let calls = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
        assert!(!calls.contains("--method POST"), "{calls}");
        assert_refusal(&check(&stub, HEAD), 65, "test_changes_undeclared");
    }
}

#[test]
fn inspect_keeps_assigned_handoff_when_test_changes_are_unconfirmed() {
    let stub = integrity_fixture(DECLARATIONS, "", "contributor");
    let out = inspect_handoff(&stub);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout);
    assert_eq!(data["data"]["status"], "awaiting-designated-review");
    assert!(data["data"].get("approval").is_none());
    assert_eq!(data["data"]["handoff"]["assigned_head"], HEAD);
    assert_refusal(&check(&stub, HEAD), 65, "test_changes_unconfirmed");
}

#[test]
fn malformed_paragraph_is_rejected_by_preview() {
    let body = "## Test changes\ntests/test_sample.py | assertion-change | Broader assertion retains the supported result contract.\n";
    let stub = integrity_fixture(body, "", "contributor");
    let preview = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-handoff",
            "test-changes",
            "7",
            "--expected-head",
            HEAD,
        ],
    );
    assert_refusal(&preview, 65, "test_changes_undeclared");
    assert!(
        preview
            .stdout
            .contains("- tests/test_sample.py | assertion-change |"),
        "{}",
        preview.stdout
    );
    assert!(
        preview.stdout.contains("- path | kind | reason"),
        "{}",
        preview.stdout
    );
}

#[test]
fn malformed_paragraph_is_rejected_before_missing_ledger() {
    let body = "## Test changes\ntests/test_sample.py | assertion-change | Broader assertion retains the supported result contract.\n";
    let stub = integrity_fixture_with_ledger(body, "", "contributor", false);
    let view = stub.tempdir.path().join("view.json");
    fs::write(&view, json!({"number":7,"url":"https://github.com/acme/widgets/pull/7","state":"OPEN", "isDraft":false,"title":"example","headRefName":"feat/example","headRefOid":HEAD, "headRepository":{"name":"widgets"},"baseRefName":"main","mergeable":"MERGEABLE", "mergedAt":"","mergeCommit":null,"labels":[],"body":body,"closingIssuesReferences":[]}).to_string()).unwrap();
    let stub = stub.env("TEST_PR_VIEW", view.to_string_lossy());
    let out = check(&stub, HEAD);
    assert_refusal(&out, 65, "test_changes_undeclared");
    assert!(
        out.stdout
            .contains("- tests/test_sample.py | assertion-change | <reason>"),
        "{}",
        out.stdout
    );
}

#[test]
fn handoff_rejects_undeclared_test_changes_with_actionable_files() {
    let stub = integrity_fixture("", "", "contributor");
    let out = check(&stub, HEAD);
    assert_refusal(&out, 65, "test_changes_undeclared");
    for expected in [
        "tests/test_sample.py",
        "assertion-change",
        "new-skip",
        "## Test changes",
        "review-handoff test-changes",
    ] {
        assert!(out.stdout.contains(expected), "{}", out.stdout);
    }
}

#[test]
fn pre_review_test_changes_prints_findings_and_digest_without_a_review_or_writes() {
    let stub = fixture(HEAD, &[], vec![]);
    let diff = stub.tempdir.path().join("integrity.diff");
    let metadata = stub.tempdir.path().join("integrity.json");
    fs::write(&diff, DIFF).unwrap();
    fs::write(
        &metadata,
        json!({"head": HEAD, "base": OLD, "author": "contributor", "body": DECLARATIONS, "changed_files":1})
            .to_string(),
    )
    .unwrap();
    sample_files(&stub);
    let directory = stub.tempdir.path().to_string_lossy().into_owned();
    let stub = stub
        .env("TEST_INTEGRITY_FILES_DIR", directory)
        .env("TEST_INTEGRITY_DIFF", diff.to_string_lossy())
        .env("TEST_INTEGRITY_METADATA", metadata.to_string_lossy());
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-handoff",
            "test-changes",
            "7",
            "--expected-head",
            HEAD,
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let value = parse_envelope(&out.stdout);
    assert_eq!(value["data"]["findings"].as_array().unwrap().len(), 2);
    assert!(
        value["data"]["digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    let confirmation = format!(
        "Test changes confirmed: {}",
        value["data"]["digest"].as_str().unwrap()
    );
    let admitted = check(
        &integrity_fixture(DECLARATIONS, &confirmation, "contributor"),
        HEAD,
    );
    assert_eq!(admitted.code, 0, "{} {}", admitted.stdout, admitted.stderr);
    let log = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
    assert!(!log.contains("graphql"), "{log}");
    assert!(!log.contains("--method"), "{log}");
    assert!(!log.contains("reviews"), "{log}");
}

#[test]
fn pre_review_test_changes_declares_a_read_only_operation_effect() {
    let stub = StubEnv::new();
    let out = run_forge_cli(
        &stub,
        &[
            "operation-effect",
            "--format",
            "json",
            "--",
            "pr",
            "review-handoff",
            "test-changes",
            "7",
            "--expected-head",
            HEAD,
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let value = parse_envelope(&out.stdout);
    assert_eq!(value["data"]["effect"], "read_only");
    assert_eq!(value["data"]["provider_effect"], "network_read");
}

#[test]
fn preview_accepts_large_cleanup_without_fetching_non_test_patches() {
    let stub = fixture(HEAD, &[], vec![]);
    let files = (0..2500).map(|index| json!({
        "filename":format!("evidence/record{index}.json"), "status":"removed", "sha":OLD,
        "additions":0, "deletions":1, "patch":format!("@@ -1 +0,0 @@\n-{}", "x".repeat(1800))
    })).chain((0..4).map(|index| json!({
        "filename":format!("tests/test_contract{index}.py"), "status":"added", "sha":HEAD,
        "additions":2, "deletions":0, "patch":"@@ -0,0 +1,2 @@\n+def test_contract():\n+    assert True"
    }))).collect::<Vec<_>>();
    assert!(
        files
            .iter()
            .map(|file| file["patch"].as_str().unwrap().len())
            .sum::<usize>()
            > 4 * 1024 * 1024
    );
    write_files(&stub, &files);
    let metadata = stub.tempdir.path().join("integrity.json");
    fs::write(&metadata, json!({"head":HEAD, "base":OLD, "author":"contributor", "body":"", "changed_files":files.len()}).to_string()).unwrap();
    let directory = stub.tempdir.path().to_string_lossy().into_owned();
    let stub = stub
        .env("TEST_INTEGRITY_FILES_DIR", directory)
        .env("TEST_INTEGRITY_METADATA", metadata.to_string_lossy());
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-handoff",
            "test-changes",
            "7",
            "--expected-head",
            HEAD,
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let value = parse_envelope(&out.stdout);
    assert_eq!(value["data"]["findings"], json!([]));
    assert!(
        value["data"]["confirmation"]
            .as_str()
            .unwrap()
            .starts_with("Test changes confirmed: sha256:")
    );
    let calls = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
    assert!(!calls.contains("/compare/"), "{calls}");
    let patches = calls
        .lines()
        .filter(|line| line.contains("per_page=1&"))
        .collect::<Vec<_>>();
    assert_eq!(patches.len(), 4);
    assert!(
        patches
            .iter()
            .all(|line| (2501..=2504).any(|index| line.contains(&format!("page={index} ")))),
        "{patches:?}"
    );
}

#[test]
fn declaration_and_allowlist_still_require_designated_reviewer_confirmation() {
    let stub = integrity_fixture(DECLARATIONS, "", "contributor");
    let out = check(&stub, HEAD);
    assert_refusal(&out, 65, "test_changes_unconfirmed");
    assert!(
        out.stdout.contains("Test changes confirmed:"),
        "{}",
        out.stdout
    );
}

#[test]
fn confirmed_changes_pass_but_author_self_confirmation_does_not() {
    let probe = check(&integrity_fixture(DECLARATIONS, "", "contributor"), HEAD);
    assert_refusal(&probe, 65, "test_changes_unconfirmed");
    let value = parse_envelope(&probe.stdout);
    let detail = value["error"]["details"]["detail"]
        .as_str()
        .or_else(|| value["error"]["detail"].as_str())
        .expect("confirmation recovery detail");
    let confirmation = detail
        .lines()
        .find(|line| line.starts_with("Test changes confirmed:"))
        .unwrap();
    let out = check(
        &integrity_fixture(DECLARATIONS, confirmation, "contributor"),
        HEAD,
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let own = check(
        &integrity_fixture(DECLARATIONS, confirmation, "review-app[bot]"),
        HEAD,
    );
    assert_refusal(&own, 65, "test_changes_unconfirmed");
    let changed = DECLARATIONS.replace("Broader assertion", "Updated assertion");
    let stale = check(
        &integrity_fixture(&changed, confirmation, "contributor"),
        HEAD,
    );
    assert_refusal(&stale, 65, "test_changes_unconfirmed");
}

#[test]
fn stale_and_retired_declarations_reach_each_warning_surface() {
    let body = "## Test changes\n\n\
        - api_test.go | deleted-test:TestRemoved | The replacement covers the removed behavior.\n\
        - api_test.go | deleted-test | The replacement test covers the same behavior.\n\
        - api_test.go | assertion-change | This row describes an unchanged assertion.\n";
    let fixture = |confirmation: &str| {
        let stub = integrity_fixture(body, confirmation, "contributor");
        write_files(
            &stub,
            &[json!({
                "filename": "api_test.go", "status": "modified", "sha": HEAD,
                "additions": 0, "deletions": 1,
                "patch": "@@ -3 +2,0 @@\n-func TestRemoved(t *testing.T) {}"
            })],
        );
        stub
    };
    let preview = |stub: &StubEnv, format: &str| {
        run_forge_cli(
            stub,
            &[
                "--provider",
                "github",
                "--repo",
                "acme/widgets",
                "--format",
                format,
                "pr",
                "review-handoff",
                "test-changes",
                "7",
                "--expected-head",
                HEAD,
            ],
        )
    };
    let stub = fixture("");
    let json = preview(&stub, "json");
    assert_eq!(json.code, 0, "{} {}", json.stdout, json.stderr);
    let envelope = parse_envelope(&json.stdout);
    let warnings = envelope["data"]["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .starts_with("Retired file-level form")
    );
    assert!(warnings[1].as_str().unwrap().contains("match no finding"));

    let text = preview(&stub, "text");
    assert_eq!(text.code, 0, "{} {}", text.stdout, text.stderr);
    let confirmation = envelope["data"]["confirmation"].as_str().unwrap();
    let admission = check(&fixture(confirmation), HEAD);
    assert_eq!(
        admission.code, 0,
        "{} {}",
        admission.stdout, admission.stderr
    );
    for warning in warnings {
        let line = format!("warning: {}", warning.as_str().unwrap());
        assert!(text.stdout.contains(&line), "{}", text.stdout);
        assert!(admission.stderr.contains(&line), "{}", admission.stderr);
    }
}

#[test]
fn emit_declaration_uses_findings_even_when_body_is_free_form() {
    let stub = integrity_fixture("## Test changes\nChanged some tests.\n", "", "contributor");
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "pr",
            "review-handoff",
            "test-changes",
            "7",
            "--expected-head",
            HEAD,
            "--emit-declaration",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert!(out.stdout.contains("## Test changes\n\n"), "{}", out.stdout);
    assert!(
        out.stdout
            .contains("- tests/test_sample.py | assertion-change | <reason>"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("- tests/test_sample.py | new-skip | <reason>"),
        "{}",
        out.stdout
    );
    let invalid = integrity_fixture(&out.stdout, "", "contributor");
    assert_refusal(&check(&invalid, HEAD), 65, "test_changes_undeclared");
}

#[test]
fn mismatched_declaration_reports_exact_expected_rows() {
    let stub = integrity_fixture(
        "## Test changes\n- tests/test_sample.py | wrong-kind | A concrete reason for the change.\n",
        "",
        "contributor",
    );
    let out = check(&stub, HEAD);
    assert_refusal(&out, 65, "test_changes_undeclared");
    for row in [
        "- tests/test_sample.py | assertion-change | <reason>",
        "- tests/test_sample.py | new-skip | <reason>",
    ] {
        assert!(out.stdout.contains(row), "{}", out.stdout);
    }
}

#[test]
fn ordinary_preview_rows_require_replacement_reasons() {
    let stub = integrity_fixture(DECLARATIONS, "", "contributor");
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "pr",
            "review-handoff",
            "test-changes",
            "7",
            "--expected-head",
            HEAD,
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let rows = out
        .stdout
        .lines()
        .filter(|line| line.starts_with("- "))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        rows.contains("- tests/test_sample.py | assertion-change | <reason>"),
        "{rows}"
    );
    let body = format!("## Test changes\n{rows}\n");
    assert_refusal(
        &check(&integrity_fixture(&body, "", "contributor"), HEAD),
        65,
        "test_changes_undeclared",
    );
}
