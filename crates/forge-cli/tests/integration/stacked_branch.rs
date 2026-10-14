use super::pr_merge::{
    github_merge_stub, gitlab_merge_api_stub, make_github_repo, make_gitlab_repo,
};
use super::support::{StubEnv, parse_envelope, run_forge_cli_in};
use pretty_assertions::assert_eq;
use std::fs;

fn merge_with_dependents(gitlab: bool, extra: &[&str]) -> (super::support::CmdOutput, String) {
    merge_with_response(gitlab, extra, None, false)
}

fn merge_with_response(
    gitlab: bool,
    extra: &[&str],
    response: Option<&str>,
    auto_delete: bool,
) -> (super::support::CmdOutput, String) {
    merge_with_source(gitlab, extra, response, auto_delete, None)
}

fn merge_with_source(
    gitlab: bool,
    extra: &[&str],
    response: Option<&str>,
    auto_delete: bool,
    source: Option<&str>,
) -> (super::support::CmdOutput, String) {
    merge_with_metadata(gitlab, extra, response, auto_delete, source, None)
}

fn merge_with_metadata(
    gitlab: bool,
    extra: &[&str],
    response: Option<&str>,
    auto_delete: bool,
    source: Option<&str>,
    unknown_source: Option<bool>,
) -> (super::support::CmdOutput, String) {
    let stub = StubEnv::new();
    let temp = if gitlab {
        make_gitlab_repo()
    } else {
        make_github_repo(None)
    };
    let log = stub.tempdir.path().join("merge-args");
    let mut script = if gitlab {
        gitlab_merge_api_stub(&stub)
    } else {
        github_merge_stub(&stub, "", "", false)
    };
    if let Some(source) = source {
        script = script.replace(
            "\"headRepository\":{\"nameWithOwner\":\"acme/widgets\"}",
            &format!("\"headRepository\":{{\"nameWithOwner\":\"{source}\"}}"),
        );
    }
    if let Some(null) = unknown_source {
        let (known, replacement) = if gitlab {
            (
                "\"source_project_id\": 1,",
                if null {
                    "\"source_project_id\": null,"
                } else {
                    ""
                },
            )
        } else {
            (
                "\"headRepository\":{\"nameWithOwner\":\"acme/widgets\"},",
                if null { "\"headRepository\":null," } else { "" },
            )
        };
        assert!(script.contains(known), "explicit source fixture missing");
        script = script.replace(known, replacement);
    }
    let row = if gitlab {
        r#"[{"iid":8,"web_url":"https://gitlab.example.com/group/project/-/merge_requests/8","state":"opened","target_branch":"feat/sample"}]"#
    } else {
        r#"[[{"number":8,"html_url":"https://github.com/acme/widgets/pull/8","state":"open","base":{"ref":"feat/reviews"}}]]"#
    };
    let row = response.unwrap_or(row);
    if gitlab {
        script = script.replace(
            "case \"$*\" in",
            &format!(
                "case \"$*\" in\n      *\"state=opened\"*) printf '%s\\n' '{}' ;;",
                row
            ),
        );
    } else {
        script = script.replace("case \"$1 $2\" in", &format!("case \"$1 $2\" in\n  \"api --paginate\") case \"$*\" in *\"state=open\"*) printf '%s\\n' '{}' ;; *) printf '%s\\n' '[[]]' ;; esac ;;", row));
        script = script.replace(
            "\"pr merge\") touch",
            &format!(
                "\"pr merge\") printf '%s\\n' \"$*\" > {}; touch",
                log.display()
            ),
        );
    }
    script = script.replace("case \"$1 $2\" in", "case \"$1 $2\" in\n  \"api repos/acme/widgets\") printf '%s\\n' '{\"delete_branch_on_merge\":false}' ;;");
    if auto_delete {
        script = script.replace(
            "delete_branch_on_merge\":false",
            "delete_branch_on_merge\":true",
        );
    }
    script = script.replace(
        "should_remove_source_branch=true\"*",
        "should_remove_source_branch=\"*",
    );
    let stub = if gitlab {
        stub.glab_stub(&script)
    } else {
        stub.gh_gate_stub(&script)
    };
    let mut args = vec![
        "--provider",
        if gitlab { "gitlab" } else { "github" },
        "pr",
        "merge",
        "7",
        "--format",
        "json",
    ];
    if gitlab {
        args.extend_from_slice(&["--host", "gitlab.example.com"]);
    }
    args.extend_from_slice(extra);
    let output = run_forge_cli_in(&stub, &args, Some(&temp.path().join("repo")));
    let merge_args = fs::read_to_string(log).unwrap_or_default();
    (output, merge_args)
}

fn assert_unknown_source_refuses_merge(gitlab: bool, null: bool) {
    let (out, args) = merge_with_metadata(
        gitlab,
        &[],
        Some(if gitlab { "[]" } else { "[[]]" }),
        false,
        None,
        Some(null),
    );
    assert_eq!(out.code, 69, "{} {}", out.stderr, out.stdout);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "branch_dependencies_unavailable"
    );
    assert!(args.is_empty(), "merge/cleanup mutation occurred: {args}");
}

#[test]
fn github_missing_source_identity_refuses_merge() {
    assert_unknown_source_refuses_merge(false, false);
}
#[test]
fn github_null_source_identity_refuses_merge() {
    assert_unknown_source_refuses_merge(false, true);
}
#[test]
fn gitlab_missing_source_identity_refuses_merge() {
    assert_unknown_source_refuses_merge(true, false);
}
#[test]
fn gitlab_null_source_identity_refuses_merge() {
    assert_unknown_source_refuses_merge(true, true);
}

#[test]
fn github_stacked_branch_is_preserved() {
    let (out, args) = merge_with_dependents(false, &[]);
    assert_eq!(out.code, 0, "{} {}", out.stderr, out.stdout);
    let payload = parse_envelope(&out.stdout);
    assert_eq!(payload["data"]["deleted_branch"], false);
    assert_eq!(
        payload["data"]["branch_cleanup"]["dependents"][0]["number"],
        8
    );
    assert!(!args.contains("--delete-branch"), "{args}");
}

#[test]
fn gitlab_stacked_branch_is_preserved() {
    let (out, args) = merge_with_dependents(true, &[]);
    assert_eq!(out.code, 0, "{} {}", out.stderr, out.stdout);
    let payload = parse_envelope(&out.stdout);
    assert_eq!(payload["data"]["deleted_branch"], false);
    assert_eq!(
        payload["data"]["branch_cleanup"]["dependents"][0]["number"],
        8
    );
    assert!(args.contains("should_remove_source_branch=false"), "{args}");
}

#[test]
fn explicit_override_deletes_both_provider_branches() {
    for gitlab in [false, true] {
        let (out, args) = merge_with_dependents(gitlab, &["--delete-branch-even-if-stacked"]);
        assert_eq!(out.code, 0, "{}", out.stdout);
        let payload = parse_envelope(&out.stdout);
        assert_eq!(payload["data"]["deleted_branch"], true);
        assert_eq!(payload["data"]["branch_cleanup"]["overridden"], true);
        assert!(args.contains(if gitlab {
            "should_remove_source_branch=true"
        } else {
            "--delete-branch"
        }));
    }
}

#[test]
fn independent_branches_keep_existing_cleanup() {
    for gitlab in [false, true] {
        let (out, args) =
            merge_with_response(gitlab, &[], Some(if gitlab { "[]" } else { "[[]]" }), false);
        assert_eq!(out.code, 0, "{}", out.stdout);
        let payload = parse_envelope(&out.stdout);
        assert_eq!(payload["data"]["deleted_branch"], true);
        assert!(payload["data"]["branch_cleanup"].is_null());
        assert!(args.contains(if gitlab {
            "should_remove_source_branch=true"
        } else {
            "--delete-branch"
        }));
    }
}

#[test]
fn malformed_dependency_read_defers_merge() {
    for gitlab in [false, true] {
        let (out, args) = merge_with_response(gitlab, &[], Some("{}"), false);
        assert_eq!(out.code, 69, "{}", out.stdout);
        assert_eq!(
            parse_envelope(&out.stdout)["error"]["code"],
            "branch_dependencies_unavailable"
        );
        assert!(args.is_empty(), "merge mutation ran: {args}");
    }
}

#[test]
fn github_automatic_deletion_defers_stacked_merge() {
    let (out, args) = merge_with_response(false, &[], None, true);
    assert_eq!(out.code, 65, "{}", out.stdout);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "stacked_branch_auto_delete"
    );
    assert!(args.is_empty());
}

#[test]
fn keep_branch_preserves_both_providers_and_conflicts_with_override() {
    for gitlab in [false, true] {
        let (out, _) = merge_with_response(
            gitlab,
            &["--keep-branch"],
            if gitlab { Some("{}") } else { None },
            false,
        );
        assert_eq!(out.code, 0, "{}", out.stdout);
        assert_eq!(parse_envelope(&out.stdout)["data"]["deleted_branch"], false);
        let (out, _) = merge_with_dependents(
            gitlab,
            &["--keep-branch", "--delete-branch-even-if-stacked"],
        );
        assert_eq!(out.code, 64, "{}", out.stdout);
    }
}

#[test]
fn github_dependency_on_later_page_is_reported() {
    let response = r#"[[],[{"number":9,"html_url":"https://github.com/acme/widgets/pull/9","state":"open","base":{"ref":"feat/reviews"}}]]"#;
    let (out, _) = merge_with_response(false, &[], Some(response), false);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["branch_cleanup"]["dependents"][0]["number"],
        9
    );
}

#[test]
fn github_keep_branch_still_refuses_automatic_deletion_with_dependents() {
    let (out, args) = merge_with_response(false, &["--keep-branch"], None, true);
    assert_eq!(out.code, 65, "{}", out.stdout);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "stacked_branch_auto_delete"
    );
    assert!(args.is_empty());
}

#[test]
fn stacked_branch_catalog_defines_validation_and_exit_contracts() {
    let catalog: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(include_str!("../../docs/specs/forge-cli-ops-v1.yaml")).unwrap();
    let validation = &catalog["validations_catalog"]["branch_dependencies"];
    assert_eq!(
        validation["error_kind"].as_str(),
        Some("stacked_branch_auto_delete")
    );
    assert_eq!(validation["exit"].as_str(), Some("DATA"));
    assert_eq!(
        validation["unavailable_error_kind"].as_str(),
        Some("branch_dependencies_unavailable")
    );
    assert_eq!(validation["unavailable_exit"].as_str(), Some("UNAVAILABLE"));
    let operation = catalog["operations"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|operation| operation["id"].as_str() == Some("pr.merge"))
        .unwrap();
    assert!(
        operation["validations"]
            .as_sequence()
            .unwrap()
            .iter()
            .any(|name| name.as_str() == Some("branch_dependencies"))
    );
    let spec = include_str!("../../docs/specs/forge-cli-spec-v1.md");
    assert!(spec.contains("| `branch_dependencies_unavailable`"));
    assert!(spec.contains("| `stacked_branch_auto_delete`"));
}

#[test]
fn github_keep_branch_on_fork_does_not_require_dependency_read() {
    let (out, args) = merge_with_source(
        false,
        &["--keep-branch"],
        Some("{}"),
        true,
        Some("example/fork"),
    );
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert!(!args.contains("--delete-branch"));
}
