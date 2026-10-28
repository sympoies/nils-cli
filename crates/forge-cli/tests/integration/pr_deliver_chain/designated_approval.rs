use super::*;
use pretty_assertions::assert_eq;

#[test]
fn restarted_no_merge_delivery_reads_persisted_assignment() {
    delivery_policy(None, Default::default(), 65);
}

#[test]
fn restarted_delivery_enforces_persisted_approval_policy() {
    use forge_cli::cli::ApprovalPolicy::{FinalApproval, SourcePass};
    delivery_policy(Some("COMMENTED"), SourcePass, 0);
    delivery_policy(Some("COMMENTED"), FinalApproval, 65);
    delivery_policy(Some("APPROVED"), FinalApproval, 0);
}

fn delivery_policy(
    native_state: Option<&str>,
    policy: forge_cli::cli::ApprovalPolicy,
    expected: i32,
) {
    let repo = make_git_repo();
    let repo_path = repo.path().join("repo");
    let stub = StubEnv::new();
    let gh_path = write_full_chain_stub(&stub);
    let inner = stub.tempdir.path().join("gh-inner-handoff");
    fs::rename(&gh_path, &inner).unwrap();
    let head = git_output(&repo_path, &["rev-parse", "HEAD"]);
    let handoff = forge_cli::ops::pr_review_handoff::ReviewHandoff {
        approval_policy: policy,
        coordinator_digest: forge_cli::ops::review_state::sha256_digest(b"worker-session"),
        reviewer_digest: forge_cli::ops::review_state::sha256_digest(b"reviewer-session"),
        review_author: "testuser-gh".into(),
        base_sha: "b".repeat(40),
        assigned_head: head.clone(),
        returned_reason: None,
        assignment_generation: 1,
        surrendered: false,
        coordinator_transfer: None,
        base_ref: None,
    };
    let record = ReviewStateRecord::new(
        "sympoies/nils-cli",
        123,
        &head,
        0,
        None,
        ReviewStatePayload::ReviewHandoff { handoff },
    )
    .unwrap();
    let mut comments = vec![
        serde_json::json!({"author":{"login":"testuser-gh"},"authorAssociation":"OWNER",
        "createdAt":"2026-07-20T12:00:00Z","body":record.marker().unwrap()}),
    ];
    if native_state.is_some() {
        let state = forge_cli::ops::review_state::observe_review_loop(None, &head, &[])
            .unwrap()
            .state;
        let observation = ReviewStateRecord::new(
            "sympoies/nils-cli",
            123,
            &head,
            1,
            Some(record.record_digest.clone()),
            ReviewStatePayload::ReviewLoop { state },
        )
        .unwrap()
        .with_assignment_generation(Some(1))
        .unwrap();
        comments.push(
            serde_json::json!({"author":{"login":"testuser-gh"},"authorAssociation":"OWNER",
            "createdAt":"2026-07-20T12:00:03Z","body":observation.marker().unwrap()}),
        );
    }
    let response = serde_json::json!({"data":{"viewer":{"login":"testuser-gh"},"repository":{"pullRequest":{
        "comments":{"nodes":comments,"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}});
    let review = serde_json::json!({"id":"REVIEW_1", "databaseId":1,
        "url":"https://github.com/sympoies/nils-cli/pull/123#pullrequestreview-1",
        "author":{"login":"testuser-gh", "__typename":"User", "id":"USER_REVIEWER"},
        "state":native_state.unwrap_or("COMMENTED"),"commit":{"oid":head},
        "submittedAt":"2026-07-20T12:00:02Z",
        "body":"<!-- agent-kit:specialist-review-report:v1 -->\n## Review Report\n\n- Reviewable: PR #123\n- Lens: testing maintainability\n- Lens verdict: pass\n- Scope: assigned source review\n- Evidence reviewed: fixture validation\n\n| Finding | Severity | Confidence | Evidence | Recommendation |\n| --- | --- | ---: | --- | --- |\n| No findings | none | 0.00 | fixture | none |\n"});
    let reviews = serde_json::json!({"data":{"viewer":{"login":"testuser-gh"},"repository":{"pullRequest":{
        "headRefOid":head,"reviews":{"nodes":[review],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}});
    let mut view: serde_json::Value = serde_json::from_str(FULL_PR_VIEW_JSON).unwrap();
    view["headRefOid"] = serde_json::json!(head);
    let base = "b".repeat(40);
    let integrity_subject = serde_json::json!({"head": head, "base": base, "author": "contributor", "body": "", "changed_files": 0});
    let wrapper = format!(
        "#!/bin/sh\ncase \"$1 $2\" in 'pr view') cat <<'JSON'\n{view}\nJSON\nexit 0;; 'api repos/sympoies/nils-cli/compare/'*) exit 0;; 'api repos/sympoies/nils-cli/pulls/123') case \"$*\" in *'author:'*) echo '{integrity_subject}';; *) echo '{base}';; esac; exit 0;; esac\ncase \"$*\" in *'comments(first:'*) cat <<'JSON'\n{response}\nJSON\nexit 0;; *'reviews(first:'*) cat <<'JSON'\n{reviews}\nJSON\nexit 0;; esac\nexec '{}' \"$@\"\n",
        inner.display()
    );
    fs::write(&gh_path, wrapper).unwrap();
    fs::set_permissions(&gh_path, fs::Permissions::from_mode(0o755)).unwrap();
    let stub = stub
        .env("FORGE_CLI_GH_BIN", gh_path.to_string_lossy())
        .env("AGENT_SESSION_ID", "worker-session");
    let out = run_in_repo(
        &stub,
        &repo_path,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "pr",
            "deliver",
            "--kind",
            "feature",
            "--title",
            "feat: sample feature",
            "--body",
            "## Summary\n\nFeature.\n\n## Test plan\n\nVerified.\n",
            "--head",
            "feat/sample",
            "--base",
            "main",
            "--timeout",
            "5s",
            "--no-merge",
        ],
    );
    assert_eq!(out.code, expected, "{} {}", out.stdout, out.stderr);
    if expected != 0 {
        assert!(
            out.stdout.contains(if native_state.is_some() {
                "designated_final_approval_required"
            } else {
                "awaiting_designated_review"
            }),
            "{}",
            out.stdout
        );
    }
    let data = parse_envelope(&out.stdout);
    assert_eq!(data["data"]["pr"]["merged"], false);
    assert!(
        !data["data"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["step"] == "ready" || s["step"] == "merge")
    );
}
