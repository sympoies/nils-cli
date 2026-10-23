use super::*;
use pretty_assertions::assert_eq;

#[path = "provenance_import.rs"]
mod provenance_import;

const DIFF: &str = "diff --git a/tests/test_sample.py b/tests/test_sample.py\nindex 1111111..2222222 100644\n--- a/tests/test_sample.py\n+++ b/tests/test_sample.py\n@@ -1,2 +1,3 @@\n-def test_exact():\n-    assert result == 42\n+@pytest.mark.skip(reason='pending')\n+def test_exact():\n+    assert result is not None\n";

fn integrity_fixture(body: &str, confirmation: &str, author: &str) -> StubEnv {
    let mut r = review(HEAD, "pass");
    r["body"] = json!(format!("{}\n{confirmation}\n", r["body"].as_str().unwrap()));
    let stub = fixture(HEAD, &records(handoff(None), Some(HEAD)), vec![r]);
    let diff = stub.tempdir.path().join("integrity.diff");
    let metadata = stub.tempdir.path().join("integrity.json");
    fs::write(&diff, DIFF).unwrap();
    fs::write(
        &metadata,
        json!({"head": HEAD, "base": OLD, "author": author, "body": body}).to_string(),
    )
    .unwrap();
    stub.env("TEST_INTEGRITY_DIFF", diff.to_string_lossy())
        .env("TEST_INTEGRITY_METADATA", metadata.to_string_lossy())
}

const DECLARATIONS: &str = "## Test changes\n\n- tests/test_sample.py | assertion-change | Broader assertion retains the supported result contract.\n- allow tests/test_sample.py | new-skip | Intentional quarantine with an independently validated replacement.\n";

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
        json!({"head": HEAD, "base": OLD, "author": "contributor", "body": DECLARATIONS})
            .to_string(),
    )
    .unwrap();
    let stub = stub
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
