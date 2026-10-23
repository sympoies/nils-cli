use super::super::*;
use pretty_assertions::assert_eq;

const UPSTREAM: &str = "cccccccccccccccccccccccccccccccccccccccc";
const TREE: &str = "dddddddddddddddddddddddddddddddddddddddd";
const BLOB: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn imported(confirmation: &str) -> StubEnv {
    let mut r = review(HEAD, "pass");
    r["body"] = json!(format!("{}\n{confirmation}\n", r["body"].as_str().unwrap()));
    let stub = fixture(HEAD, &records(handoff(None), Some(HEAD)), vec![r]);
    let root = stub.tempdir.path();
    let body = format!(
        "## Provenance import\n{}\n",
        json!({"repository":"acme/upstream", "commit":UPSTREAM,
        "mappings":[{"destination":"vendor/", "source":""}], "adaptations":[]})
    );
    fs::write(
        root.join("integrity.json"),
        json!({"head":HEAD,"base":OLD,"author":"contributor","body":body,"changed_files":625})
            .to_string(),
    )
    .unwrap();
    let files = (0..625)
        .map(|n| {
            json!({"filename":format!("vendor/tests/import{n}.test.ts"),
        "status":"added", "sha":BLOB, "additions":1, "deletions":0,
        "patch":format!("@@ -0,0 +1 @@\n+{}", "x".repeat(11_600))})
        })
        .collect::<Vec<_>>();
    for (index, page) in files.chunks(100).enumerate() {
        fs::write(
            root.join(format!("page{}.json", index + 1)),
            serde_json::to_vec(page).unwrap(),
        )
        .unwrap();
    }
    fs::write(
        root.join("commit.json"),
        json!({"sha":UPSTREAM,"tree":{"sha":TREE}}).to_string(),
    )
    .unwrap();
    fs::write(root.join("tree.json"), json!({"sha":TREE,"truncated":false,"tree":(0..625).map(|n|
        json!({"path":format!("tests/import{n}.test.ts"), "type":"blob","mode":"100644","sha":BLOB})).collect::<Vec<_>>()}).to_string()).unwrap();
    let metadata = root.join("integrity.json").to_string_lossy().into_owned();
    let directory = root.to_string_lossy().into_owned();
    stub.env("TEST_INTEGRITY_METADATA", metadata)
        .env("TEST_PROVENANCE_DIR", directory)
}

#[test]
fn large_import_preview_and_designated_gate_share_the_exact_confirmation() {
    let stub = imported("");
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
    let data = &value["data"];
    assert_eq!(data["findings"], json!([]));
    assert_eq!(data["provenance_import"]["verified_files"], 625);
    assert_eq!(data["provenance_import"]["file_count"], 625);
    assert!(
        data["provenance_import"]["manifest_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    assert_refusal(&check(&stub, HEAD), 65, "test_changes_unconfirmed");
    let confirmed = imported(data["confirmation"].as_str().unwrap());
    let out = check(&confirmed, HEAD);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let calls = fs::read_to_string(confirmed.tempdir.path().join("calls.log")).unwrap();
    assert!(calls.contains("files?per_page=100&page=7"));
    assert!(!calls.contains("/compare/"));
}
