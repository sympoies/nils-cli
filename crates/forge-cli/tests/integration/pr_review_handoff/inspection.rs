use super::*;
use pretty_assertions::assert_eq;

pub(super) fn assert_pending(stub: &StubEnv) {
    let out = inspect_handoff(stub);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = parse_envelope(&out.stdout);
    assert_eq!(data["data"]["status"], "awaiting-designated-review");
    assert!(data["data"].get("approval").is_none());
}

#[test]
fn approved_exact_head_empty_findings_agree_between_inspect_and_check() {
    let mut owner = handoff(None);
    owner.approval_policy = forge_cli::cli::ApprovalPolicy::FinalApproval;
    let seeded = records(owner, Some(HEAD));
    let mut report = review(HEAD, "pass");
    report["state"] = json!("APPROVED");
    let stub = fixture(HEAD, &seeded, vec![report]);

    let checked = check(&stub, HEAD);
    assert_eq!(checked.code, 0, "{} {}", checked.stdout, checked.stderr);
    let checked = parse_envelope(&checked.stdout);
    assert_eq!(checked["data"]["status"], "reviewed");
    assert_eq!(checked["data"]["approval"]["state"], "finally-approved");

    let inspected = inspect_handoff(&stub);
    assert_eq!(
        inspected.code, 0,
        "{} {}",
        inspected.stdout, inspected.stderr
    );
    let inspected = parse_envelope(&inspected.stdout);
    assert_eq!(inspected["data"]["status"], checked["data"]["status"]);
    assert_eq!(inspected["data"]["approval"], checked["data"]["approval"]);
    assert_eq!(inspected["data"]["handoff"], checked["data"]["handoff"]);
    assert_eq!(inspected["data"]["head_sha"], HEAD);
    assert_eq!(
        inspected["data"]["state_tip_digest"],
        seeded.last().unwrap().record_digest
    );
    let calls = fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap();
    assert!(!calls.contains("--method POST"));
}
