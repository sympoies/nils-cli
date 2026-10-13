use pretty_assertions::assert_eq;

use super::support::{StubEnv, parse_envelope, run_forge_cli};

#[test]
fn completions_cli_exports_raw_facts_and_coverage() {
    let stub = StubEnv::new().gh_stub(r#"#!/bin/sh
test "$1" = api && test "$2" = graphql || exit 97
cat <<'JSON'
{"data":{"search":{"issueCount":1,"pageInfo":{"hasNextPage":false,"endCursor":"end"},"nodes":[{"__typename":"Issue","id":"I_7","number":7,"url":"https://github.com/acme/widget/issues/7","title":"A neutral completion","issueState":"CLOSED","stateReason":"COMPLETED","createdAt":"2029-12-01T00:00:00Z","closedAt":"2030-01-01T12:00:00Z","updatedAt":"2031-01-01T00:00:00Z","repository":{"nameWithOwner":"acme/widget","isFork":false,"isArchived":false},"author":null,"labels":{"totalCount":1,"nodes":[{"name":"raw label"}],"pageInfo":{"hasNextPage":false,"endCursor":"label-end"}}}]}}}
JSON
"#);
    let output = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "search",
            "completions",
            "repo:acme/widget state:closed",
            "--since",
            "2030-01-01T00:00:00Z",
            "--until",
            "2030-01-02T00:00:00Z",
        ],
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr);
    let envelope = parse_envelope(&output.stdout);
    assert_eq!(
        envelope["schema_version"],
        "cli.forge-cli.search.completions.v1"
    );
    assert_eq!(envelope["data"]["pagination"]["complete"], true);
    assert_eq!(
        envelope["data"]["items"][0]["completed_at"],
        "2030-01-01T12:00:00Z"
    );
    assert_eq!(
        envelope["data"]["items"][0]["author"],
        serde_json::Value::Null
    );
    assert_eq!(envelope["data"]["items"][0]["labels"][0], "raw label");
}

#[test]
fn completions_cli_dry_run_is_a_read_only_scoped_graphql_plan() {
    let stub = StubEnv::new().gh_stub("#!/bin/sh\nexit 97\n");
    let output = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "--dry-run",
            "--format",
            "json",
            "search",
            "completions",
            "label:\"needs review\"",
            "--kind",
            "pr",
            "--since",
            "2030-01-01T00:00:00Z",
            "--until",
            "2030-01-02T00:00:00Z",
        ],
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr);
    let envelope = parse_envelope(&output.stdout);
    let plan = envelope["data"]["plan"].as_array().unwrap();
    assert!(plan.iter().any(|arg| arg == "searchQuery=label:\"needs review\" repo:acme/widget is:pr is:merged merged:2030-01-01T00:00:00Z..2030-01-02T00:00:00Z"));
    assert!(plan.iter().any(|arg| {
        arg.as_str()
            .is_some_and(|arg| arg.contains("closingIssuesReferences"))
    }));
}

#[test]
fn completions_cli_rejects_invalid_window_and_explicitly_unsupported_providers() {
    let stub = StubEnv::new().gh_stub("#!/bin/sh\nexit 97\n");
    for provider in ["gitlab", "local"] {
        let output = run_forge_cli(
            &stub,
            &[
                "--provider",
                provider,
                "--repo",
                "acme/widget",
                "--format",
                "json",
                "search",
                "completions",
                "state:closed",
                "--since",
                "2030-01-01T00:00:00Z",
                "--until",
                "2030-01-02T00:00:00Z",
            ],
        );
        assert_eq!(
            parse_envelope(&output.stdout)["error"]["code"],
            "provider_unsupported"
        );
    }
    let output = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "--format",
            "json",
            "search",
            "completions",
            "state:closed",
            "--since",
            "2030-01-02T00:00:00Z",
            "--until",
            "2030-01-01T00:00:00Z",
        ],
    );
    assert_eq!(output.code, 65);
    assert_eq!(
        parse_envelope(&output.stdout)["error"]["code"],
        "completion_window_invalid"
    );
}
