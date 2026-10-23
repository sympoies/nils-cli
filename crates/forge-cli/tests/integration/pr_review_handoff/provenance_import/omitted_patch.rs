use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use pretty_assertions::{assert_eq, assert_ne};
use sha1::{Digest, Sha1};

fn blob(content: &[u8]) -> Value {
    let mut hash = Sha1::new();
    hash.update(format!("blob {}\0", content.len()));
    hash.update(content);
    json!({"sha":format!("{:x}", hash.finalize()), "size":content.len(),
        "encoding":"base64", "content":STANDARD.encode(content)})
}

fn omitted(before: Option<&[u8]>, after: &[u8], path: &str) -> StubEnv {
    let stub = imported("");
    let root = stub.tempdir.path();
    let head = blob(after);
    let base = blob(before.unwrap_or_default());
    for (name, value) in [("head", &head), ("base", &base)] {
        fs::write(
            root.join(format!("{name}-content.json")),
            json!({"type":"file", "sha":value["sha"], "size":value["size"]}).to_string(),
        )
        .unwrap();
        fs::write(
            root.join(format!("{}.json", value["sha"].as_str().unwrap())),
            value.to_string(),
        )
        .unwrap();
    }
    let mut page: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(root.join("page7.json")).unwrap()).unwrap();
    page[0] = json!({"filename":path, "status": if before.is_some() {"modified"} else {"added"},
        "sha":head["sha"], "additions":1, "deletions":usize::from(before.is_some()), "patch":null});
    fs::write(root.join("page7.json"), serde_json::to_vec(&page).unwrap()).unwrap();
    let script = fs::read_to_string(root.join("gh")).unwrap().replace(
        "case \"$1 $2\" in",
        &format!(
            r#"case "$1 $2" in
  "api repos/acme/widgets/compare/{OLD}...{HEAD}") echo '{{"sha":"{OLD}"}}'; exit 0 ;;
  "api repos/acme/widgets/contents/"*)
    case "$2" in
      *ref={OLD}) cat "$TEST_PROVENANCE_DIR/base-content.json" ;;
      *ref={HEAD}) cat "$TEST_PROVENANCE_DIR/head-content.json" ;;
      *) exit 99 ;;
    esac; exit $? ;;
  "api repos/acme/widgets/git/blobs/"*) cat "$TEST_PROVENANCE_DIR/${{2##*/}}.json"; exit $? ;;
"#
        ),
    );
    stub.gh_stub(&script)
}

#[test]
fn omitted_large_text_patch_is_inspected_from_exact_base_head_blobs() {
    let padding = "// retained padding\n".repeat(30_000);
    let before = format!("assert(value === 42);\n{padding}");
    let after = format!("assert(value);\n{padding}");
    let stub = omitted(
        Some(before.as_bytes()),
        after.as_bytes(),
        "tests/large.test.ts",
    );
    let out = preview(&stub);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["findings"],
        json!([
        {"path":"tests/large.test.ts", "kind":"assertion-change"}])
    );
    let calls = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
    assert!(calls.contains(&format!("contents/tests/large.test.ts?ref={OLD}")));
    assert!(calls.contains(&format!("contents/tests/large.test.ts?ref={HEAD}")));
    assert!(!calls.contains("ref=main"));
}

#[test]
fn omitted_blob_identity_size_and_content_mismatches_fail_closed() {
    for field in ["sha", "size", "content"] {
        let stub = omitted(
            None,
            b"test.skip('contract', () => {});\n",
            "tests/added.test.ts",
        );
        let root = stub.tempdir.path();
        let mut value = blob(b"test.skip('contract', () => {});\n");
        let sha = value["sha"].as_str().unwrap().to_string();
        value[field] = match field {
            "sha" => json!(BLOB),
            "size" => json!(1),
            _ => json!(STANDARD.encode(b"different bytes\n")),
        };
        fs::write(root.join(format!("{sha}.json")), value.to_string()).unwrap();
        assert_refusal(&preview(&stub), 65, "test_changes_incomplete");
    }
}

#[test]
fn omitted_binary_patch_has_an_exact_path_typed_refusal() {
    let stub = omitted(None, b"binary\0asset", "assets/font.woff2");
    let out = preview(&stub);
    assert_refusal(&out, 65, "test_changes_incomplete");
    let error = &parse_envelope(&out.stdout)["error"];
    assert_eq!(
        error["details"]["cause"]["code"],
        "residual_binary_patch_unavailable"
    );
    assert_eq!(error["details"]["cause"]["path"], "assets/font.woff2");
    assert_eq!(error["details"]["retryable"], false);
    assert_eq!(
        error["details"]["next_action"],
        "correct_provenance_declaration"
    );
}

#[test]
fn omitted_text_reconstruction_retains_the_ordinary_diff_byte_limit() {
    let stub = omitted(
        None,
        "x".repeat(4 * 1024 * 1024).as_bytes(),
        "tests/large.test.ts",
    );
    assert_refusal(&preview(&stub), 65, "test_changes_limit_exceeded");
}

fn check_moved_base(status: &str) {
    const MERGE_BASE: &str = "ffffffffffffffffffffffffffffffffffffffff";
    const OTHER_BASE: &str = "1111111111111111111111111111111111111111";
    let after = if status == "removed" {
        b"".as_slice()
    } else {
        b"assert(value);\n".as_slice()
    };
    let path = if status == "renamed" {
        "tests/renamed.test.ts"
    } else {
        "tests/large.test.ts"
    };
    let stub = omitted(Some(b"assert(value === 42);\n"), after, path);
    let root = stub.tempdir.path();
    let mut page: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(root.join("page7.json")).unwrap()).unwrap();
    page[0]["status"] = json!(status);
    if status == "removed" {
        page[0]["sha"] = blob(b"assert(value === 42);\n")["sha"].clone();
        page[0]["additions"] = json!(0);
    }
    if status == "renamed" {
        page[0]["previous_filename"] = json!("tests/large.test.ts");
    }
    fs::write(root.join("page7.json"), serde_json::to_vec(&page).unwrap()).unwrap();
    let script = fs::read_to_string(root.join("gh")).unwrap()
        .replace(&format!("*ref={OLD}) cat \"$TEST_PROVENANCE_DIR/base-content.json\""),
            &format!("*ref={OLD}) cat \"$TEST_PROVENANCE_DIR/head-content.json\" ;;\n      *ref={MERGE_BASE}) cat \"$TEST_PROVENANCE_DIR/base-content.json\""))
        .replace("case \"$1 $2\" in", &format!("case \"$1 $2\" in\n  \"api repos/acme/widgets/compare/{OLD}...{HEAD}\") echo '{{\"sha\":\"{MERGE_BASE}\"}}'; exit 0 ;;"));
    fs::write(root.join("gh"), &script).unwrap();
    let out = preview(&stub);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = &parse_envelope(&out.stdout)["data"];
    assert!(
        data["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "assertion-change")
    );
    if status != "modified" {
        assert!(
            data["findings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["kind"] == "deleted-test")
        );
    }
    assert_eq!(data["provenance_import"]["merge_base"], MERGE_BASE);
    fs::write(root.join("gh"), script.replace(MERGE_BASE, OTHER_BASE)).unwrap();
    let other = preview(&stub);
    assert_eq!(other.code, 0, "{} {}", other.stdout, other.stderr);
    assert_ne!(
        parse_envelope(&other.stdout)["data"]["digest"],
        data["digest"]
    );
}

#[test]
fn moved_base_modified_text_keeps_assertion_findings_and_merge_base_binding() {
    check_moved_base("modified");
}

#[test]
fn moved_base_removed_text_keeps_deletion_findings_and_merge_base_binding() {
    check_moved_base("removed");
}

#[test]
fn moved_base_renamed_text_keeps_deletion_findings_and_merge_base_binding() {
    check_moved_base("renamed");
}
