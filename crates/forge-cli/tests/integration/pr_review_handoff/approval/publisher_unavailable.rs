//! Recovery releases only an unavailable publisher's assignment, with no review invented.
use super::super::*;
use crate::integration::pr_merge::{github_merge_stub_full, make_github_repo};
use crate::integration::support::{CmdOutput, run_forge_cli_in};
use pretty_assertions::assert_eq;

fn recover(stub: &StubEnv, reason: &str, head: &str, tip: &str) -> CmdOutput {
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
            "recover",
            "7",
            "--reason",
            reason,
            "--expected-head",
            head,
            "--expected-state",
            tip,
        ],
    )
}

fn merge(stub: &StubEnv, repo: &std::path::Path, opt_out: bool) -> CmdOutput {
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
        "pr",
        "merge",
        "7",
        "--keep-branch",
    ];
    if opt_out {
        args.push("--review-convergence=false");
    }
    run_forge_cli_in(stub, &args, Some(repo))
}

fn merge_fixture(seeded: &[ReviewStateRecord]) -> StubEnv {
    let stub = fixture(HEAD, seeded, vec![]);
    let root = stub.tempdir.path();
    fs::rename(root.join("gh"), root.join("handoff-gh")).unwrap();
    let body = github_merge_stub_full(&stub, "", "", true, None,
        r#"[{"name":"ci","bucket":"pass","state":"COMPLETED","isRequired":true}]"#,
        r#"{"data":{"repository":{"mergeQueue":null,"issues":{"nodes":[]},"pullRequest":{"id":"PR_node7","isInMergeQueue":false}}}}"#, "")
        .replace("head123", HEAD)
        .replace("  *) echo \"unexpected gh args: $*\" >&2; exit 99 ;;",
            &format!("  *) exec '{}/handoff-gh' \"$@\" ;;", root.display()));
    let stub = writable_ledger(stub.gh_gate_stub(&body), seeded);
    // Thread reads also contain comments; only ledger reads use this adapter.
    let body = fs::read_to_string(stub.tempdir.path().join("gh"))
        .unwrap()
        .replace(
            "any('comments(first:' in a for a in args)",
            "any('authorAssociation body createdAt' in a for a in args)",
        );
    stub.gh_stub(&body)
}

#[test]
fn publisher_unavailable_releases_merge_without_fabricating_an_observation() {
    let repo = make_github_repo(Some("[review_convergence]\nrequire = true\n"));
    let repo = repo.path().join("repo");
    let seeded = records(super::strict_owner(), None);
    let stub = merge_fixture(&seeded);
    assert_refusal(&merge(&stub, &repo, true), 65, "awaiting_designated_review");

    let out = recover(&stub, "reviewer-declined", HEAD, &seeded[0].record_digest);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let tip = parse_envelope(&out.stdout)["data"]["state_tip_digest"]
        .as_str()
        .unwrap()
        .to_string();
    assert_refusal(
        &merge(&stub, &repo, true),
        69,
        "designated_reviewer_unavailable",
    );

    let out = recover(&stub, "publisher-unavailable", HEAD, &tip);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["handoff"]["assignment_generation"], 3);
    assert_eq!(data["handoff"]["returned_reason"], "publisher-unavailable");
    let bodies: Vec<String> =
        serde_json::from_str(&fs::read_to_string(stub.tempdir.path().join("ledger.json")).unwrap())
            .unwrap();
    let chain =
        review_state::parse_chain(bodies.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(chain.records.len(), 3);
    assert_eq!(
        chain.tip_digest.as_deref(),
        data["state_tip_digest"].as_str()
    );
    assert!(review_state::latest_review_loop_state(&chain).is_none());
    assert_refusal(&check(&stub, HEAD), 69, "designated_reviewer_unavailable");
    assert_refusal(&merge(&stub, &repo, false), 65, "review_state_conflict");
    let out = merge(&stub, &repo, true);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert!(stub.tempdir.path().join("github-merged").exists());

    let out = assign_handoff(
        &stub,
        REVIEWER_UUID,
        OLD,
        data["state_tip_digest"].as_str().unwrap(),
        false,
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["handoff"]["assignment_generation"],
        4
    );
    let stub = stub.env("AGENT_REVIEWER_SESSION", REVIEWER_UUID);
    assert_refusal(&merge(&stub, &repo, true), 65, "awaiting_designated_review");
}

#[test]
fn publisher_unavailable_requires_coordinator_and_exact_head_and_state() {
    let seeded = records(super::strict_owner(), None);
    for (actor, head, tip, error) in [
        (
            "reviewer-session",
            HEAD,
            seeded[0].record_digest.as_str(),
            "review_writer_conflict",
        ),
        (
            "worker-session",
            OLD,
            seeded[0].record_digest.as_str(),
            "review_state_conflict",
        ),
        (
            "worker-session",
            HEAD,
            "sha256:stale",
            "review_state_conflict",
        ),
    ] {
        let stub = merge_fixture(&seeded).env("AGENT_SESSION_ID", actor);
        assert_refusal(
            &recover(&stub, "publisher-unavailable", head, tip),
            65,
            error,
        );
        let bodies: Vec<String> = serde_json::from_str(
            &fs::read_to_string(stub.tempdir.path().join("ledger.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(bodies.len(), 1);
    }
}
