//! GitHub issue timeline contract at the executable boundary.

use pretty_assertions::assert_eq;
use serde_json::json;

use super::support::{StubEnv, parse_envelope, run_forge_cli};

const TIMELINE_STUB: &str = r#"#!/bin/sh
[ "$1" = api ] && [ "$2" = graphql ] && [ "$3" = --hostname ] &&
[ "$4" = github.example.com ] || exit 97
case " $* " in
  *" owner=acme "*" name=widget "*" number=42 "*"itemTypes: [CLOSED_EVENT, REOPENED_EVENT]"*) ;;
  *) exit 97 ;;
esac
case " $* " in
  *" after=page-two "*)
    cat <<'JSON'
{"data":{"repository":{"issue":{"timelineItems":{"nodes":[{"__typename":"ClosedEvent","actor":{"login":"reviewer"},"created_at":"2030-01-02T00:00:00Z","closer":{"__typename":"Commit","oid":"0123456789abcdef","url":"https://github.example.com/acme/widget/commit/0123456789abcdef"}},{"__typename":"ReopenedEvent","actor":null,"created_at":"2030-01-03T00:00:00Z"}],"page_info":{"has_next_page":false,"end_cursor":"last"}}}}}}
JSON
    ;;
  *" after="*) exit 97 ;;
  *)
    printf '{"data":{"repository":{"issue":{"timelineItems":{"nodes":['
    i=0
    while [ "$i" -lt 100 ]; do
      [ "$i" -eq 0 ] || printf ','
      if [ "$((i % 2))" -eq 0 ]; then
        printf '%s' '{"__typename":"ClosedEvent","actor":{"login":"maintainer"},"created_at":"2030-01-01T00:00:00Z","closer":{"__typename":"PullRequest","number":7,"url":"https://github.example.com/acme/widget/pull/7"}}'
      else
        printf '%s' '{"__typename":"ReopenedEvent","actor":{"login":"maintainer"},"created_at":"2030-01-01T00:00:00Z"}'
      fi
      i=$((i+1))
    done
    printf '],"page_info":{"has_next_page":true,"end_cursor":"page-two"}}}}}}\n'
    ;;
esac
"#;

#[test]
fn github_issue_events_keep_all_matching_pages_and_supplied_references() {
    let stub = StubEnv::new().gh_stub(TIMELINE_STUB);
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--host",
            "github.example.com",
            "--repo",
            "acme/widget",
            "issue",
            "events",
            "42",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.code, 0, "stdout={} stderr={}", out.stdout, out.stderr);
    let envelope = parse_envelope(&out.stdout);
    assert_eq!(envelope["schema_version"], "cli.forge-cli.issue.events.v1");
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["provider"], "github");
    assert_eq!(envelope["data"]["number"], 42);
    let events = envelope["data"]["events"].as_array().expect("events");
    assert_eq!(events.len(), 102);
    assert_eq!(
        events[0],
        json!({
            "event": "closed",
            "actor": "maintainer",
            "created_at": "2030-01-01T00:00:00Z",
            "commit": null,
            "pull_request": {"number": 7, "url": "https://github.example.com/acme/widget/pull/7"}
        })
    );
    assert_eq!(
        events[100],
        json!({
            "event": "closed",
            "actor": "reviewer",
            "created_at": "2030-01-02T00:00:00Z",
            "commit": {"oid": "0123456789abcdef", "url": "https://github.example.com/acme/widget/commit/0123456789abcdef"},
            "pull_request": null
        })
    );
    assert_eq!(
        events[101],
        json!({
            "event": "reopened",
            "actor": null,
            "created_at": "2030-01-03T00:00:00Z",
            "commit": null,
            "pull_request": null
        })
    );
}

#[test]
fn issue_events_refuses_unsupported_providers_before_running_backends() {
    let stub = StubEnv::new()
        .gh_stub("#!/bin/sh\nexit 97\n")
        .glab_stub("#!/bin/sh\nexit 97\n");
    // A configured identity policy must also refuse GitLab with the same code.
    let config = stub.tempdir.path().join("xdg-config/forge-cli");
    std::fs::create_dir_all(&config).expect("identity config directory");
    std::fs::write(
        config.join("identity.toml"),
        include_str!("../../../nils-common/tests/fixtures/identity/policy.toml"),
    )
    .expect("identity policy");
    for (provider, repo) in [("local", "local:widget"), ("gitlab", "acme/widget")] {
        let out = run_forge_cli(
            &stub,
            &[
                "--provider",
                provider,
                "--repo",
                repo,
                "issue",
                "events",
                "42",
                "--format",
                "json",
            ],
        );
        assert_eq!(out.code, 64, "stdout={} stderr={}", out.stdout, out.stderr);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["error"]["code"], "provider_unsupported");
    }
}

#[test]
fn issue_events_declares_a_read_only_provider_operation() {
    let out = run_forge_cli(
        &StubEnv::new(),
        &[
            "operation-effect",
            "--format",
            "json",
            "--",
            "issue",
            "events",
            "42",
            "--repo",
            "acme/widget",
        ],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let envelope = parse_envelope(&out.stdout);
    assert_eq!(envelope["data"]["operation"], "issue.events");
    assert_eq!(envelope["data"]["effect"], "read_only");
    assert_eq!(envelope["data"]["provider_effect"], "network_read");
}

#[test]
fn issue_events_page_cap_fails_without_partial_results() {
    let stub = StubEnv::new().gh_stub(
        r#"#!/bin/sh
[ "$1" = api ] && [ "$2" = graphql ] || exit 97
page=0
for arg do
    case "$arg" in after=*) page=${arg#after=} ;; esac
done
printf '%s\n' "$page" >> requests
printf '{"data":{"repository":{"issue":{"timelineItems":{"nodes":[{"__typename":"ClosedEvent","actor":null,"created_at":"2030-01-01T00:00:00Z","closer":null}],"page_info":{"has_next_page":true,"end_cursor":"%s"}}}}}}\n' "$((page + 1))"
"#,
    );
    let out = run_github_issue_events(&stub);
    assert_eq!(out.code, 65, "stdout={} stderr={}", out.stdout, out.stderr);
    let envelope = parse_envelope(&out.stdout);
    assert_eq!(envelope["ok"], false);
    assert_eq!(envelope["error"]["code"], "issue_events_incomplete");
    assert!(envelope.get("data").is_none());
    let pages = std::fs::read_to_string(stub.tempdir.path().join("requests")).expect("requests");
    assert_eq!(pages.lines().count(), 100);
}

#[test]
fn issue_events_refuses_a_repeated_cursor() {
    let stub = StubEnv::new().gh_stub(
        r#"#!/bin/sh
printf '%s\n' '{"data":{"repository":{"issue":{"timelineItems":{"nodes":[],"page_info":{"has_next_page":true,"end_cursor":"same"}}}}}}'
"#,
    );
    let out = run_github_issue_events(&stub);
    assert_eq!(out.code, 65, "stdout={} stderr={}", out.stdout, out.stderr);
    let envelope = parse_envelope(&out.stdout);
    assert_eq!(envelope["error"]["code"], "issue_events_incomplete");
    assert!(
        envelope["error"]["message"]
            .as_str()
            .unwrap()
            .contains("repeated cursor")
    );
}

fn run_github_issue_events(stub: &StubEnv) -> super::support::CmdOutput {
    run_forge_cli(
        stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "issue",
            "events",
            "42",
            "--format",
            "json",
        ],
    )
}
