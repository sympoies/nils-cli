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

fn preview(stub: &StubEnv) -> crate::integration::support::CmdOutput {
    run_forge_cli(
        stub,
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
    )
}

fn scoped_import(upstream_operations: &str, upstream_rule: bool) -> StubEnv {
    let stub = imported("");
    let root = stub.tempdir.path();
    let metadata = root.join("integrity.json");
    let value = fs::read_to_string(&metadata)
        .unwrap()
        .replace("acme/upstream", "other/upstream");
    fs::write(metadata, value).unwrap();
    let config = root.join("config/forge-cli");
    fs::create_dir_all(&config).unwrap();
    let mut policy = format!(
        r#"version = 1
[credentials.reader]
kind = "env"
name = "FIXTURE_READ_CREDENTIAL"
[profiles.downstream]
expected_login = "reader"
credential = "reader"
commit_name = "Example"
commit_email = "example@example.invalid"
signing_fingerprint = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
operations = ["api_read", "api_write"]
[profiles.upstream]
expected_login = "reader"
credential = "reader"
commit_name = "Example"
commit_email = "example@example.invalid"
signing_fingerprint = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
operations = [{upstream_operations}]
[principals.contributor]
profiles = ["downstream", "upstream"]
[[rules]]
id = "downstream"
principal = "contributor"
repo = "github.com/acme/widgets"
profile = "downstream"
"#
    );
    if upstream_rule {
        policy.push_str(
            r#"
[[rules]]
id = "upstream"
principal = "contributor"
org = "github.com/other"
profile = "upstream"
"#,
        );
    }
    fs::write(config.join("identity.toml"), policy).unwrap();
    let script = fs::read_to_string(root.join("gh"))
        .unwrap()
        .replace("acme/upstream", "other/upstream")
        .replace(
            "case \"$1 $2\" in",
            "case \"$1 $2\" in\n  \"api user\") echo '{\"login\":\"reader\"}'; exit 0 ;;",
        );
    let config_home = root.join("config").to_string_lossy().into_owned();
    stub.gh_stub(&script)
        .env("XDG_CONFIG_HOME", config_home)
        .env("FORGE_IDENTITY_PRINCIPAL", "contributor")
        .env("FIXTURE_READ_CREDENTIAL", "fixture-read-credential")
}

#[test]
fn provenance_upstream_cross_owner_reads_use_authorized_api_read_scope_and_restore_pr_scope() {
    let stub = scoped_import("\"api_read\"", true);
    let out = preview(&stub);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["provenance_import"]["verified_files"],
        625
    );
    let audit = fs::read_to_string(
        stub.tempdir
            .path()
            .join("xdg-state/forge-cli/identity-audit.jsonl"),
    )
    .unwrap();
    let records: Vec<Value> = audit
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let upstream: Vec<_> = records
        .iter()
        .filter(|row| row["target"]["repo"] == "other/upstream")
        .collect();
    assert!(!upstream.is_empty());
    for row in upstream {
        assert_eq!(row["operation"], "api_read");
        assert_eq!(row["profile_id"], "upstream");
    }
    assert_eq!(records.last().unwrap()["target"]["repo"], "acme/widgets");
}

#[test]
fn provenance_upstream_identity_refusals_keep_their_typed_codes() {
    for (operations, rule, code) in [
        ("\"api_read\"", false, "identity_repository_unknown"),
        ("\"api_write\"", true, "identity_operation_denied"),
    ] {
        let stub = scoped_import(operations, rule);
        let out = preview(&stub);
        assert_refusal(&out, 65, code);
        let calls = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
        assert!(!calls.contains("/git/commits/"));
    }
}

#[test]
fn provenance_exact_added_notice_is_in_preview_and_requires_declaration() {
    let stub = imported("");
    let root = stub.tempdir.path();
    let mut page: Vec<Value> =
        serde_json::from_str(&fs::read_to_string(root.join("page7.json")).unwrap()).unwrap();
    page.push(
        json!({"filename":"vendor/NOTICE.txt", "status":"added", "sha":HEAD,
        "additions":1, "deletions":0, "patch":"@@ -0,0 +1 @@\n+License notice"}),
    );
    fs::write(root.join("page7.json"), serde_json::to_vec(&page).unwrap()).unwrap();
    let mut metadata: Value =
        serde_json::from_str(&fs::read_to_string(root.join("integrity.json")).unwrap()).unwrap();
    metadata["changed_files"] = json!(626);
    fs::write(root.join("integrity.json"), metadata.to_string()).unwrap();
    assert_refusal(&preview(&stub), 65, "provenance_import_mismatch");
    metadata["body"] = json!(metadata["body"].as_str().unwrap().replace(
        "\"adaptations\":[]",
        "\"adaptations\":[],\"additions\":[\"vendor/NOTICE.txt\"]"
    ));
    fs::write(root.join("integrity.json"), metadata.to_string()).unwrap();
    let out = preview(&stub);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let value = parse_envelope(&out.stdout);
    assert_eq!(
        value["data"]["findings"],
        json!([{"path":"vendor/NOTICE.txt", "kind":"import-addition"}])
    );
    assert_eq!(value["data"]["provenance_import"]["verified_files"], 625);
    assert_eq!(
        value["data"]["provenance_import"]["additions"],
        json!(["vendor/NOTICE.txt"])
    );
    assert_refusal(&check(&stub, HEAD), 65, "test_changes_undeclared");
    metadata["body"] = json!(format!(
        "{}\n## Test changes\n- vendor/NOTICE.txt | import-addition | Added license notice accompanies the imported assets; existing tests are retained.\n",
        metadata["body"].as_str().unwrap()
    ));
    fs::write(root.join("integrity.json"), metadata.to_string()).unwrap();
    let out = preview(&stub);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let value = parse_envelope(&out.stdout);
    let script = fs::read_to_string(root.join("gh")).unwrap().replace(
        "## Review Report",
        &format!(
            "## Review Report\\n{}",
            value["data"]["confirmation"].as_str().unwrap()
        ),
    );
    fs::write(root.join("gh"), script).unwrap();
    let out = check(&stub, HEAD);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
}

#[path = "provenance_import/omitted_patch.rs"]
mod omitted_patch;
