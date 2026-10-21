use super::*;
use crate::serve::prompt_receipts::Outcome;
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;

fn fixture() -> (tempfile::TempDir, CliContext, Receipt) {
    let temp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: temp.path().to_path_buf(),
        host: None,
    };
    fs::create_dir_all(session_dir(&context, "receipt-session")).unwrap();
    let receipt = Receipt::unknown(
        "receipt-session".into(),
        "launch-one".into(),
        "attempt-one".into(),
    );
    (temp, context, receipt)
}

#[test]
fn unfinished_and_partial_claims_are_unknown_and_consumed() {
    let (_temp, context, receipt) = fixture();
    let fingerprint = fingerprint("private prompt canary", true);
    let ClaimResult::New(_owner) = claim(&context, receipt.clone(), &fingerprint).unwrap() else {
        panic!("not new")
    };
    let result = lookup(&context, &receipt).unwrap().unwrap();
    assert_eq!(result.receipt.outcome, Outcome::Unknown);
    assert!(matches!(
        claim(&context, receipt.clone(), &fingerprint).unwrap(),
        ClaimResult::Existing(_)
    ));
    let claim_path = path(&context, &receipt);
    fs::write(&claim_path, b"{").unwrap();
    assert!(matches!(
        claim(&context, receipt.clone(), &fingerprint).unwrap(),
        ClaimResult::Existing(_)
    ));
    assert_eq!(
        lookup(&context, &receipt).unwrap().unwrap().receipt.outcome,
        Outcome::Unknown
    );
    assert!(
        !fs::read_to_string(claim_path)
            .unwrap()
            .contains("private prompt canary")
    );
}

#[test]
fn results_are_private_durable_scoped_and_contain_no_prompt_text() {
    let (_temp, context, receipt) = fixture();
    let fingerprint = fingerprint("prompt-text-canary", false);
    let ClaimResult::New(owner) = claim(&context, receipt.clone(), &fingerprint).unwrap() else {
        panic!("not new")
    };
    let mut submitted = receipt.clone();
    submitted.outcome = Outcome::Submitted;
    submitted.code = None;
    submitted.confirmation_diagnostic = None;
    finish(
        owner,
        &ResultRecord {
            receipt: submitted.clone(),
            status: 200,
            queued: true,
            native_confirmation: None,
        },
    )
    .unwrap();
    assert_eq!(
        lookup(&context, &receipt).unwrap().unwrap().receipt,
        submitted
    );
    let mut other = receipt.clone();
    other.session_incarnation = "launch-two".into();
    assert!(lookup(&context, &other).unwrap().is_none());
    other = receipt.clone();
    other.request_id = "attempt-two".into();
    assert!(lookup(&context, &other).unwrap().is_none());
    for entry in fs::read_dir(session_dir(&context, &receipt.session_id)).unwrap() {
        let entry = entry.unwrap();
        assert_eq!(
            entry.metadata().unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            !fs::read_to_string(entry.path())
                .unwrap()
                .contains("prompt-text-canary")
        );
    }
}

#[test]
fn changed_text_or_idle_policy_cannot_reuse_a_consumed_key() {
    let (_temp, context, receipt) = fixture();
    claim(&context, receipt.clone(), &fingerprint("first", true)).unwrap();
    for fingerprint in [fingerprint("second", true), fingerprint("first", false)] {
        let error = match claim(&context, receipt.clone(), &fingerprint) {
            Err(error) => error,
            Ok(_) => panic!("changed payload admitted"),
        };
        assert_eq!(error.code(), "request-id-conflict");
    }
}

#[test]
fn missing_or_corrupt_result_never_reopens_a_claim() {
    let (_temp, context, receipt) = fixture();
    claim(&context, receipt.clone(), &fingerprint("first", false)).unwrap();
    fs::write(
        path(&context, &receipt).with_extension("result"),
        b"invalid",
    )
    .unwrap();
    assert_eq!(
        lookup(&context, &receipt).unwrap().unwrap().receipt.outcome,
        Outcome::Unknown
    );
}

#[tokio::test]
async fn post_paste_guard_failure_is_unknown_even_when_not_submitted() {
    let receipt = Receipt::unknown("session".into(), "launch".into(), "attempt".into());
    for (code, details) in [
        (
            "send-session-not-idle",
            serde_json::json!({"stage":"submit", "pasted":true,"submitted":false}),
        ),
        (
            "send-native-unconfirmed",
            serde_json::json!({"stage":"submit","pasted":false,"submitted":null,"dispatched":true}),
        ),
        ("tmux-command-failed", serde_json::Value::Null),
    ] {
        let response = crate::serve::envelope_err(CliError::runtime(
            code,
            "delivery uncertain",
            Some(details),
        ));
        let result = super::super::classify(response, receipt.clone()).await;
        assert_eq!(result.receipt.outcome, Outcome::Unknown);
        assert_eq!(
            result.receipt.code.as_deref(),
            Some("structured-prompt-outcome-unknown")
        );
    }
}
