//! Regression contracts for automation metadata reads.
use super::support::{StubEnv, parse_envelope, run_forge_cli};
use pretty_assertions::assert_eq;

fn read(args: &[&str], script: &str) -> serde_json::Value {
    let stub = StubEnv::new().gh_stub(script);
    let out = run_forge_cli(&stub, args);
    assert_eq!(out.code, 0, "stdout={} stderr={}", out.stdout, out.stderr);
    parse_envelope(&out.stdout)["data"].clone()
}

#[test]
fn metadata_issue_list_body_and_update_time() {
    for labels in [false, true] {
        let mut args = vec![
            "issue",
            "list",
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "--state",
            "open",
            "--limit",
            "300",
            "--format",
            "json",
        ];
        if labels {
            args.extend(["--label", "bug"]);
        }
        let data = read(
            &args,
            r#"#!/bin/sh
case "$1" in
 issue) case "$*" in *body,updatedAt*) ;; *) exit 97;; esac;;
esac
printf '%s\n' '[{"number":7,"url":"https://github.com/acme/widget/issues/7","html_url":"https://github.com/acme/widget/issues/7","title":"Example","state":"open","body":"Details","updatedAt":"2026-01-02T00:00:00Z","updated_at":"2026-01-02T00:00:00Z"}]'
"#,
        );
        assert_eq!(data["items"][0]["body"], "Details");
        assert_eq!(data["items"][0]["updated_at"], "2026-01-02T00:00:00Z");
    }
}

#[test]
fn metadata_issue_view_closure() {
    let data = read(
        &[
            "issue",
            "view",
            "7",
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "--format",
            "json",
        ],
        r#"#!/bin/sh
case "$*" in *closedAt,stateReason*) ;; *) exit 97;; esac
printf '%s\n' '{"number":7,"url":"https://github.com/acme/widget/issues/7","title":"Example","state":"CLOSED","closedAt":"2026-01-02T00:00:00Z","stateReason":"NOT_PLANNED"}'
"#,
    );
    assert_eq!(data["closed_at"], "2026-01-02T00:00:00Z");
    assert_eq!(data["state_reason"], "not_planned");
}

#[test]
fn metadata_org_search_does_not_add_implicit_repo() {
    let data = read(
        &[
            "search",
            "issues",
            "--provider",
            "github",
            "--limit",
            "100",
            "--format",
            "json",
            "org:acme org:example is:issue is:open",
        ],
        r#"#!/bin/sh
case "$*" in *--repo*) echo implicit-repo >&2; exit 97;; esac
case "$*" in *updatedAt,labels*) ;; *) exit 97;; esac
printf '%s\n' '[{"number":7,"url":"https://github.com/acme/widget/issues/7","title":"Example","state":"open","repository":{"nameWithOwner":"acme/widget"},"updatedAt":"2026-01-02T00:00:00Z","labels":[{"name":"bug"}]}]'
"#,
    );
    assert_eq!(data["repo"], "");
    assert_eq!(data["items"][0]["updated_at"], "2026-01-02T00:00:00Z");
    assert_eq!(data["items"][0]["labels"], serde_json::json!(["bug"]));
}

#[test]
fn metadata_search_quoted_and_escaped_qualifiers_preserve_implicit_repo() {
    use super::support::run_forge_cli_in;
    let stub = StubEnv::new().gh_stub("#!/bin/sh\necho unexpected-backend >&2\nexit 97\n");
    let root = stub.tempdir.path().join("checkout");
    std::fs::create_dir(&root).unwrap();
    for args in [
        vec!["init", "-b", "main"],
        vec![
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widget.git",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .current_dir(&root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    for kind in ["issues", "prs"] {
        for query in [
            r#""see org:github""#,
            r#""see user:operator""#,
            r#""see repo:other/project""#,
            r#"\org:github"#,
            r#""see \" org:github""#,
        ] {
            let out = run_forge_cli_in(
                &stub,
                &[
                    "search",
                    kind,
                    "--provider",
                    "github",
                    "--format",
                    "json",
                    "--dry-run",
                    query,
                ],
                Some(&root),
            );
            assert_eq!(out.code, 0, "query={query}: {} {}", out.stdout, out.stderr);
            let plan = parse_envelope(&out.stdout)["data"]["plan"]
                .as_array()
                .unwrap()
                .clone();
            assert!(
                plan.windows(2)
                    .any(|pair| pair[0] == "--repo" && pair[1] == "acme/widget"),
                "query={query}: {plan:?}"
            );
        }
        for query in [
            "org:github",
            "user:operator",
            "repo:other/project",
            r#""see org:github" org:example"#,
        ] {
            let out = run_forge_cli_in(
                &stub,
                &[
                    "search",
                    kind,
                    "--provider",
                    "github",
                    "--format",
                    "json",
                    "--dry-run",
                    query,
                ],
                Some(&root),
            );
            assert_eq!(out.code, 0, "query={query}: {} {}", out.stdout, out.stderr);
            let plan = parse_envelope(&out.stdout)["data"]["plan"]
                .as_array()
                .unwrap()
                .clone();
            assert!(
                !plan.iter().any(|arg| arg == "--repo"),
                "query={query}: {plan:?}"
            );
        }
    }
}

#[test]
fn metadata_repo_view_default_branch_sha() {
    let data = read(
        &[
            "repo",
            "view",
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "--format",
            "json",
        ],
        r#"#!/bin/sh
case "$1" in
 repo) printf '%s\n' '{"owner":{"login":"acme"},"name":"widget","url":"https://github.com/acme/widget","defaultBranchRef":{"name":"main"}}';;
 api) case "$*" in *repos/acme/widget/commits/main*) printf '%s\n' '{"sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}';; *) exit 97;; esac;;
 *) exit 97;;
esac
"#,
    );
    assert_eq!(
        data["default_branch_head_sha"],
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
}

#[test]
fn metadata_query_scope_with_explicit_host_and_conflicting_checkout() {
    use super::support::run_forge_cli_in;
    for enterprise in [false, true] {
        let host = if enterprise {
            "enterprise.example"
        } else {
            "github.com"
        };
        let script = format!(
            "#!/bin/sh\n[ \"$GH_HOST\" = '{host}' ] || exit 96\ncase \"$*\" in *--repo*) exit 97;; esac\nprintf '[]'\n"
        );
        let stub = StubEnv::new().gh_stub(&script);
        let root = stub.tempdir.path().join("checkout");
        std::fs::create_dir(&root).unwrap();
        for args in [
            vec!["init", "-b", "main"],
            vec![
                "remote",
                "add",
                "origin",
                "https://gitlab.com/other/project.git",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .current_dir(&root)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        let mut args = vec![
            "search",
            "issues",
            "--provider",
            "github",
            "--format",
            "json",
            "org:acme is:open",
        ];
        if enterprise {
            args.extend(["--host", host]);
        }
        let out = run_forge_cli_in(&stub, &args, Some(&root));
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        let v = parse_envelope(&out.stdout);
        assert_eq!(v["data"]["host"], host);
        assert_eq!(v["data"]["repo"], "");
    }
}

#[test]
fn metadata_repo_view_dry_run_includes_deferred_commit_read() {
    let stub = StubEnv::new().gh_stub("#!/bin/sh\necho backend-called >&2\nexit 97\n");
    let out = run_forge_cli(
        &stub,
        &[
            "repo",
            "view",
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "--host",
            "enterprise.example",
            "--dry-run",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let v = parse_envelope(&out.stdout);
    assert_eq!(v["data"]["plan"][1], "repo");
    assert_eq!(
        serde_json::Value::Array(v["data"]["follow_up"]["plan"].as_array().unwrap()[1..].to_vec()),
        serde_json::json!([
            "api",
            "--hostname",
            "enterprise.example",
            "repos/acme/widget/commits/<encoded_default_branch>"
        ])
    );
}
