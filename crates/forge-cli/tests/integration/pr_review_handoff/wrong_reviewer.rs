//! Recovery from an assignment whose reviewer cannot publish as its author.
use super::*;
use pretty_assertions::assert_eq;

fn surrender(stub: &StubEnv, head: &str, tip: &str) -> super::super::support::CmdOutput {
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
            "surrender",
            "7",
            "--expected-head",
            head,
            "--expected-state",
            tip,
        ],
    )
}

#[test]
fn wrong_reviewer_surrenders_without_generation_then_replacement_publishes() {
    let mut h = handoff(None);
    h.review_author = "wrong-review-author".into();
    let seeded = records(h, None);
    let stub = writable_ledger(fixture(HEAD, &seeded, vec![]), &seeded)
        .env("AGENT_REVIEWER_SESSION", "")
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "");
    let out = surrender(&stub, HEAD, &seeded[0].record_digest);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout);
    assert_eq!(data["data"]["handoff"]["surrendered"], true);
    assert_eq!(data["data"]["handoff"]["assignment_generation"], 1);
    let coordinator = stub.env("AGENT_SESSION_ID", "worker-session");
    let out = assign_handoff(
        &coordinator,
        REVIEWER_UUID,
        OLD,
        data["data"]["state_tip_digest"].as_str().unwrap(),
        false,
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout);
    assert_eq!(data["data"]["handoff"]["assignment_generation"], 2);
    assert_eq!(data["data"]["handoff"]["review_author"], "review-app[bot]");
    assert_eq!(data["data"]["head_sha"], HEAD);
    let reviewer = publishing_provider(coordinator)
        .env("AGENT_SESSION_ID", REVIEWER_UUID)
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");
    let findings = reviewer.tempdir.path().join("clean.json");
    fs::write(&findings, "[]").unwrap();
    let out = run_forge_cli(
        &reviewer,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-loop",
            "observe",
            "7",
            "--expected-head",
            HEAD,
            "--auto-state",
            "--findings-file",
            findings.to_str().unwrap(),
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_refusal(&check(&reviewer, HEAD), 65, "awaiting_designated_review");
    let body = reviewer.tempdir.path().join("report.md");
    fs::write(&body, review(HEAD, "pass")["body"].as_str().unwrap()).unwrap();
    let out = run_forge_cli(
        &reviewer,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review",
            "7",
            "--submit-review",
            "--decision",
            "comments-only",
            "--expected-head",
            HEAD,
            "--comment-file",
            body.to_str().unwrap(),
            "--lens",
            "testing",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let out = check(&reviewer, HEAD);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(parse_envelope(&out.stdout)["data"]["status"], "reviewed");
}

fn publishing_provider(stub: StubEnv) -> StubEnv {
    fs::rename(
        stub.tempdir.path().join("gh"),
        stub.tempdir.path().join("ledger-gh"),
    )
    .unwrap();
    fs::write(
        stub.tempdir.path().join("review.json"),
        serde_json::to_vec(&review(HEAD, "pass")).unwrap(),
    )
    .unwrap();
    stub.gh_stub(r#"#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
published = root / 'published-body'
r = json.loads((root / 'review.json').read_text())
r['submittedAt'] = '2026-07-20T12:00:04Z'
if args[:2] == ['api', 'repos/acme/widgets/pulls/7/reviews']:
    published.write_text(next(a[5:] for a in args if a.startswith('body=')))
    print(r['url'])
elif args[:2] == ['api', 'graphql'] and 'reviews(first:' in ' '.join(args) and 'states: [PENDING]' not in ' '.join(args):
    r['body'] = published.read_text() if published.exists() else ''
    print(json.dumps({'data': {'viewer': {'login':'review-app[bot]'}, 'repository': {'pullRequest': {
        'headRefOid': r['commit']['oid'], 'reviews': {'nodes':[r] if published.exists() else [], 'pageInfo': {'hasNextPage':False,'endCursor':None}}}}}}))
elif args[:2] == ['api', 'repos/acme/widgets/pulls/7/reviews/1']:
    print(json.dumps({'id':1, 'html_url':r['url'], 'state':r['state'], 'commit_id':r['commit']['oid'],
        'user':{'login':'review-app[bot]', 'type':'Bot', 'node_id':'BOT_REVIEW_APP'}, 'body':published.read_text()}))
elif args[:2] == ['api', 'repos/acme/widgets/pulls/7/reviews/1/comments?per_page=100&page=1']:
    print(json.dumps([{'pull_request_review_id':1, 'commit_id':r['commit']['oid'], 'in_reply_to_id':9}]))
else:
    os.execv(str(root / 'ledger-gh'), [str(root / 'ledger-gh')] + args)
"#)
}

#[test]
fn surrender_requires_persisted_reviewer_and_current_head_and_tip() {
    let seeded = records(handoff(None), None);
    for (actor, head, tip, kind) in [
        (
            "worker-session",
            HEAD,
            seeded[0].record_digest.as_str(),
            "review_writer_conflict",
        ),
        (
            "another-reviewer",
            HEAD,
            seeded[0].record_digest.as_str(),
            "review_writer_conflict",
        ),
        (
            "reviewer-session",
            OLD,
            seeded[0].record_digest.as_str(),
            "review_state_conflict",
        ),
        ("reviewer-session", HEAD, "none", "review_state_conflict"),
    ] {
        let stub = fixture(HEAD, &seeded, vec![])
            .env("AGENT_REVIEWER_SESSION", "")
            .env("AGENT_SESSION_ID", actor)
            .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "");
        assert_refusal(&surrender(&stub, head, tip), 65, kind);
        assert!(
            !fs::read_to_string(stub.tempdir.path().join("calls.log"))
                .unwrap()
                .contains("--method POST")
        );
    }
}

#[test]
fn ownership_refusals_explain_reviewer_surrender_and_original_coordinator_recovery() {
    let seeded = records(handoff(None), None);
    let stub = fixture(HEAD, &seeded, vec![]).env("AGENT_REVIEWER_SESSION", "");
    let out = assign_handoff(&stub, REVIEWER_UUID, OLD, &seeded[0].record_digest, true);
    assert_refusal(&out, 65, "review_handover_required");
    assert!(out.stdout.contains("surrender"), "{}", out.stdout);
    assert!(out.stdout.contains("reviewer-declined"), "{}", out.stdout);
    let reviewer = stub
        .env("AGENT_SESSION_ID", "reviewer-session")
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "");
    let out = observe(&reviewer, HEAD);
    assert!(
        out.stdout.contains("review_assignment_generation_conflict"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("AGENT_REVIEW_ASSIGNMENT_GENERATION"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("inspect"), "{}", out.stdout);
}
