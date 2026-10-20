//! Ref/commit-bound code-scanning evidence; no live provider calls.
use super::support::{StubEnv, parse_envelope, run_forge_cli};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

const COMMIT: &str = "1111111111111111111111111111111111111111";
const REFERENCE: &str = "refs/pull/42/head";

fn instance(reference: &str, commit: &str) -> Value {
    json!({"ref": reference, "commit_sha": commit, "analysis_key": "workflow:analyze",
        "environment": "{}", "category": "language:rust", "state": "open",
        "message": {"text": "This command depends on an input."},
        "location": {"path": "src/launch.rs", "start_line": 12, "start_column": 5}})
}

fn fixture(instances: Value, analyses: Value) -> StubEnv {
    let alert = json!({"number": 7, "state": "open", "html_url": "https://github.com/acme/widget/security/code-scanning/7",
        "rule": {"id": "rust/command-line-injection", "security_severity_level": "critical"},
        "tool": {"name": "CodeQL"}, "most_recent_instance": instance("refs/heads/main", &"2".repeat(40))});
    let sarif = json!({"version": "2.1.0", "runs": [{"tool": {"driver": {"name": "CodeQL"}}, "results": [{
    "ruleId": "rust/command-line-injection", "properties": {"github/alertNumber": 7},
    "codeFlows": [{"threadFlows": [{"locations": [
        {"location": {"message": {"text": "source"}}},
        {"location": {"message": {"text": "intermediate"}}},
        {"location": {"message": {"text": "sink"}}}
    ]}]}]}]}]});
    StubEnv::new().gh_stub(&format!(r#"#!/bin/sh
case "$*" in
 'api -X GET repos/acme/widget/code-scanning/alerts/7') cat <<'JSON'
{alert}
JSON
 ;;
 'api -X GET repos/acme/widget/code-scanning/alerts/7/instances -f ref=refs/pull/42/head -f per_page=100 -f page=1') cat <<'JSON'
{instances}
JSON
 ;;
 'api -X GET repos/acme/widget/code-scanning/analyses -f ref=refs/pull/42/head -f per_page=100 -f page=1') cat <<'JSON'
{analyses}
JSON
 ;;
 'api -X GET repos/acme/widget/code-scanning/analyses/9 -H Accept:application/sarif+json') cat <<'JSON'
{sarif}
JSON
 ;;
 *) echo "unexpected backend request: $*" >&2; exit 97;;
esac
"#))
}

fn args() -> Vec<&'static str> {
    vec![
        "security",
        "alerts",
        "view",
        "7",
        "--kind",
        "code-scanning",
        "--ref",
        REFERENCE,
        "--commit",
        COMMIT,
        "--provider",
        "github",
        "--repo",
        "acme/widget",
        "--format",
        "json",
    ]
}

#[test]
fn security_code_scanning_view_binds_instances_and_preserves_sarif_flow() {
    let matching = instance(REFERENCE, COMMIT);
    let stub = fixture(
        json!([
            instance("refs/heads/main", COMMIT),
            instance(REFERENCE, &"2".repeat(40)),
            matching.clone()
        ]),
        json!([{"id": 8, "ref": REFERENCE, "commit_sha": "2".repeat(40),
            "analysis_key": "workflow:analyze", "environment": "{}", "category": "language:rust"},
            {"id": 10, "ref": REFERENCE, "commit_sha": COMMIT,
            "analysis_key": "workflow:analyze", "environment": "{}", "category": "language:python", "tool": {"name": "CodeQL"}},
            {"id": 9, "ref": REFERENCE, "commit_sha": COMMIT,
            "analysis_key": "workflow:analyze", "environment": "{}", "category": "language:rust", "tool": {"name": "CodeQL"}}]),
    );
    let mut argv = args();
    argv.push("--include-sarif");
    let out = run_forge_cli(&stub, &argv);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let value = parse_envelope(&out.stdout);
    assert_eq!(
        value["schema_version"],
        "cli.forge-cli.security.alerts.view.v1"
    );
    assert_eq!(value["data"]["reference"], REFERENCE);
    assert_eq!(value["data"]["commit_sha"], COMMIT);
    assert_eq!(value["data"]["instances"], json!([matching]));
    assert_eq!(value["data"]["analyses"].as_array().unwrap().len(), 1);
    assert_eq!(value["data"]["analyses"][0]["metadata"]["id"], 9);
    let flow = &value["data"]["analyses"][0]["sarif"]["runs"][0]["results"][0]["codeFlows"][0]["threadFlows"]
        [0]["locations"];
    assert_eq!(flow.as_array().unwrap().len(), 3);
    assert_eq!(flow[0]["location"]["message"]["text"], "source");
    assert_eq!(flow[1]["location"]["message"]["text"], "intermediate");
    assert_eq!(flow[2]["location"]["message"]["text"], "sink");
}

#[test]
fn security_code_scanning_view_does_not_treat_wrong_commit_as_evidence() {
    let stub = fixture(json!([instance(REFERENCE, &"2".repeat(40))]), json!([]));
    let out = run_forge_cli(&stub, &args());
    assert_eq!(out.code, 65, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "security_instance_not_found"
    );
}

#[test]
fn security_code_scanning_view_is_read_only_and_dry_run_never_calls_backend() {
    let stub = StubEnv::new().gh_stub("#!/bin/sh\necho unexpected-backend >&2\nexit 97\n");
    let mut argv = args();
    argv.extend(["--include-sarif", "--dry-run"]);
    let out = run_forge_cli(&stub, &argv);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert!(parse_envelope(&out.stdout)["data"]["plan"].is_array());
    let mut effect = vec!["operation-effect", "--format", "json", "--"];
    effect.extend(args());
    let out = run_forge_cli(&stub, &effect);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let value = parse_envelope(&out.stdout);
    assert_eq!(value["data"]["operation"], "security.alerts.view");
    assert_eq!(value["data"]["effect"], "read_only");
    assert_eq!(value["data"]["provider_effect"], "network_read");
}

#[test]
fn security_code_scanning_view_without_sarif_preserves_instance_status_only() {
    let stub = fixture(json!([instance(REFERENCE, COMMIT)]), json!([]));
    let path = &stub
        .envs
        .iter()
        .find(|(key, _)| key == "FORGE_CLI_GH_BIN")
        .unwrap()
        .1;
    let script = std::fs::read_to_string(path).unwrap().replace(
        " 'api -X GET repos/acme/widget/code-scanning/analyses -f ref=refs/pull/42/head -f per_page=100 -f page=1') cat <<'JSON'",
        " 'api -X GET repos/acme/widget/code-scanning/analyses -f ref=refs/pull/42/head -f per_page=100 -f page=1') exit 97; cat <<'JSON'");
    std::fs::write(path, script).unwrap();
    let out = run_forge_cli(&stub, &args());
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let value = parse_envelope(&out.stdout);
    assert_eq!(value["data"]["sarif_requested"], false);
    assert_eq!(value["data"]["analyses"], json!([]));
    assert_eq!(value["data"]["instances"][0]["state"], "open");
    assert_eq!(
        value["data"]["instances"][0]["analysis_key"],
        "workflow:analyze"
    );
}

#[test]
fn security_code_scanning_view_pages_before_selecting_exact_subject() {
    let matching = instance(REFERENCE, COMMIT);
    let stub = fixture(json!([]), json!([]));
    let path = &stub
        .envs
        .iter()
        .find(|(key, _)| key == "FORGE_CLI_GH_BIN")
        .unwrap()
        .1;
    let first = json!(vec![instance(REFERENCE, &"2".repeat(40)); 100]);
    let script = std::fs::read_to_string(path).unwrap().replace("#!/bin/sh", &format!(r#"#!/bin/sh
case "$*" in
 'api -X GET repos/acme/widget/code-scanning/alerts/7/instances -f ref=refs/pull/42/head -f per_page=100 -f page=1') cat <<'JSON'
{first}
JSON
 exit 0;;
 'api -X GET repos/acme/widget/code-scanning/alerts/7/instances -f ref=refs/pull/42/head -f per_page=100 -f page=2') cat <<'JSON'
[{matching}]
JSON
 exit 0;;
esac
"#));
    std::fs::write(path, script).unwrap();
    let out = run_forge_cli(&stub, &args());
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["instances"],
        json!([matching])
    );
}

#[test]
fn security_code_scanning_view_keeps_provider_refusal_as_error() {
    let stub = StubEnv::new()
        .gh_stub("#!/bin/sh\necho 'HTTP 403: code scanning access denied' >&2\nexit 1\n");
    let out = run_forge_cli(&stub, &args());
    assert_ne!(out.code, 0);
    let value = parse_envelope(&out.stdout);
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "backend_error");
    assert!(value.get("data").is_none());
}

#[test]
fn security_code_scanning_view_uses_selected_read_identity_without_fallback() {
    const CANARY: &str = "FIXTURE_SECURITY_READ_CREDENTIAL";
    for missing in [false, true] {
        let stub = fixture(json!([instance(REFERENCE, COMMIT)]), json!([]))
            .env("FORGE_IDENTITY_PRINCIPAL", "contributor")
            .env(
                "FIXTURE_ACCOUNT_A_CREDENTIAL",
                if missing { "" } else { CANARY },
            );
        let config = stub.tempdir.path().join("xdg-config/forge-cli");
        std::fs::create_dir_all(&config).unwrap();
        let policy = include_str!("../../../nils-common/tests/fixtures/identity/policy.toml")
            .replace("github.com/sandbox", "github.com/acme");
        std::fs::write(config.join("identity.toml"), policy).unwrap();
        let path = &stub
            .envs
            .iter()
            .find(|(key, _)| key == "FORGE_CLI_GH_BIN")
            .unwrap()
            .1;
        let script = std::fs::read_to_string(path).unwrap().replace(
            "#!/bin/sh",
            r#"#!/bin/sh
case "$1:$2" in
 api:user) printf '{"login":"account-a"}'; exit 0;;
 auth:token) echo forbidden-credential-fallback >&2; exit 97;;
esac
[ "$GH_TOKEN" = "$FIXTURE_ACCOUNT_A_CREDENTIAL" ] && [ -n "$GH_TOKEN" ] || exit 97
[ -z "$GITHUB_TOKEN" ] || exit 97
"#,
        );
        std::fs::write(path, script).unwrap();
        let out = run_forge_cli(&stub, &args());
        assert_eq!(
            out.code,
            if missing { 65 } else { 0 },
            "{} {}",
            out.stdout,
            out.stderr
        );
        assert!(!out.stdout.contains(CANARY));
        assert!(!out.stderr.contains(CANARY));
        let value = parse_envelope(&out.stdout);
        if missing {
            assert_eq!(value["error"]["code"], "identity_credential_missing");
        } else {
            let audit = std::fs::read_to_string(
                stub.tempdir
                    .path()
                    .join("xdg-state/forge-cli/identity-audit.jsonl"),
            )
            .unwrap();
            assert!(audit.lines().all(|line| {
                let row: Value = serde_json::from_str(line).unwrap();
                row["profile_id"] == "account-a"
                    && row["target"]["repo"] == "acme/widget"
                    && row["operation"] == "api_read"
            }));
        }
    }
}

#[test]
fn security_code_scanning_view_missing_analysis_is_not_a_clean_flow_result() {
    let stub = fixture(json!([instance(REFERENCE, COMMIT)]), json!([]));
    let mut argv = args();
    argv.push("--include-sarif");
    let out = run_forge_cli(&stub, &argv);
    assert_eq!(out.code, 65, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "security_analysis_not_found"
    );
}

#[test]
fn security_code_scanning_view_does_not_publish_a_truncated_instance_inventory() {
    let stub = fixture(json!([]), json!([]));
    let path = &stub
        .envs
        .iter()
        .find(|(key, _)| key == "FORGE_CLI_GH_BIN")
        .unwrap()
        .1;
    let rows = json!(vec![instance(REFERENCE, COMMIT); 100]);
    let script = std::fs::read_to_string(path).unwrap().replace("#!/bin/sh", &format!(r#"#!/bin/sh
case "$*" in
 'api -X GET repos/acme/widget/code-scanning/alerts/7/instances -f ref=refs/pull/42/head -f per_page=100 -f page='*) cat <<'JSON'
{rows}
JSON
 exit 0;;
esac
"#));
    std::fs::write(path, script).unwrap();
    let out = run_forge_cli(&stub, &args());
    assert_ne!(out.code, 0);
    let value = parse_envelope(&out.stdout);
    assert_eq!(value["error"]["code"], "security_read_incomplete");
    assert!(value.get("data").is_none());
}
