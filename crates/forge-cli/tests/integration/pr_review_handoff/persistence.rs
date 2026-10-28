//! Live synthetic writes exercise the assignment persistence boundary.
use super::super::support::parse_envelope;
use super::{HEAD, OLD, REVIEWER_UUID, assign_handoff, fixture, inspect_handoff, writable_ledger};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::fs;

#[test]
fn assignment_readback_failure_reports_persisted_tip_without_retrying() {
    for failure in ["unavailable", "not-visible"] {
        let stub =
            writable_ledger(fixture(HEAD, &[], vec![]), &[]).env("AGENT_REVIEWER_SESSION", "");
        let root = stub.tempdir.path();
        fs::rename(root.join("gh"), root.join("writable-gh")).unwrap();
        let stub = stub.gh_stub(&format!(r#"#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
if args[:2] == ['api', 'graphql'] and any('comments(first:' in a for a in args) and json.loads((root / 'ledger.json').read_text()) and not (root / 'failed-readback').exists():
    (root / 'failed-readback').touch()
    if {failure:?} == 'unavailable':
        print('provider read unavailable', file=sys.stderr)
        sys.exit(1)
    print(json.dumps({{'data': {{'viewer': {{'login': 'review-app[bot]'}}, 'repository': {{'pullRequest': {{'comments': {{'nodes': [], 'pageInfo': {{'hasNextPage': False, 'endCursor': None}}}}}}}}}}}}))
else:
    os.execv(str(root / 'writable-gh'), [str(root / 'writable-gh')] + args)
"#));
        let out = assign_handoff(&stub, REVIEWER_UUID, OLD, "none", false);
        let inspected = inspect_handoff(&stub);
        assert_eq!(
            inspected.code, 0,
            "{} {}",
            inspected.stdout, inspected.stderr
        );
        let persisted = parse_envelope(&inspected.stdout)["data"].clone();
        assert_eq!(persisted["handoff"]["assignment_generation"], 1);
        assert_eq!(out.code, 65, "{} {}", out.stdout, out.stderr);
        let error = parse_envelope(&out.stdout)["error"].clone();
        assert_eq!(
            error["code"],
            "review_handoff_persisted_verification_failed"
        );
        assert_eq!(error["details"]["persisted"], true);
        assert_eq!(error["details"]["retryable"], false);
        assert_eq!(
            error["details"]["state_tip_digest"],
            persisted["state_tip_digest"]
        );
        assert_eq!(
            error["details"]["handoff_digest"],
            persisted["handoff_digest"]
        );
        assert!(error["message"].as_str().unwrap().contains("persisted"));
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains(persisted["state_tip_digest"].as_str().unwrap())
        );
        let bodies: Vec<String> = serde_json::from_str(
            &fs::read_to_string(stub.tempdir.path().join("ledger.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(bodies.len(), 1);
    }
}

#[test]
fn genesis_assignment_returns_the_persisted_tip() {
    let stub = writable_ledger(fixture(HEAD, &[], vec![]), &[]).env("AGENT_REVIEWER_SESSION", "");
    let before = inspect_handoff(&stub);
    assert_eq!(before.code, 0, "{} {}", before.stdout, before.stderr);
    assert_eq!(
        parse_envelope(&before.stdout)["data"]["status"],
        "unassigned"
    );
    assert_eq!(
        parse_envelope(&before.stdout)["data"]["state_tip_digest"],
        Value::Null
    );

    let out = assign_handoff(&stub, REVIEWER_UUID, OLD, "none", false);
    // Check the provider state even if the command reported an error.
    let inspected = inspect_handoff(&stub);
    assert_eq!(
        inspected.code, 0,
        "{} {}",
        inspected.stdout, inspected.stderr
    );
    let persisted = parse_envelope(&inspected.stdout)["data"].clone();
    assert_eq!(persisted["handoff"]["assignment_generation"], 1);
    assert_eq!(persisted["handoff"]["assigned_head"], HEAD);
    assert_eq!(persisted["handoff"]["base_sha"], OLD);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["status"], "awaiting-designated-review");
    assert_eq!(data["state_tip_digest"], persisted["state_tip_digest"]);
    assert_eq!(data["handoff_digest"], persisted["state_tip_digest"]);
    let ledger: Vec<String> =
        serde_json::from_str(&fs::read_to_string(stub.tempdir.path().join("ledger.json")).unwrap())
            .unwrap();
    assert_eq!(ledger.len(), 1);
}
