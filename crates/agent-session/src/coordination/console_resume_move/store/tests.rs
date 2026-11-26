use axum::http::StatusCode;
use std::fs;

use pretty_assertions::{assert_eq, assert_ne};
use serde_json::json;

use super::*;
use crate::coordination::console_resume_move::{refusal::Failure, wire::START_SCHEMA};

fn context(directory: &tempfile::TempDir) -> CliContext {
    CliContext {
        state_dir: directory.path().to_path_buf(),
        host: None,
    }
}

fn move_id(index: u128) -> String {
    uuid::Uuid::from_u128(index | (4 << 76) | (0b10 << 62)).to_string()
}

fn start(caller: &str, index: u128, cwd: &str) -> (StartRequest, String) {
    start_with_operation(caller, index, cwd, &uuid::Uuid::now_v7().to_string())
}

fn start_with_operation(
    caller: &str,
    index: u128,
    cwd: &str,
    operation: &str,
) -> (StartRequest, String) {
    let body = json!({
        "schema_version": START_SCHEMA,
        "operation_id": operation,
        "machine": "destination",
        "caller": {
            "session_id": caller,
            "session_incarnation": "00000000-0000-4000-8000-0000000000a1",
        },
        "session": {
            "move_id": move_id(index),
            "agent": "claude",
            "resume": "00000000-0000-4000-8000-0000000000b1",
            "account": "fixture-account",
            "cwd": cwd,
        },
    })
    .to_string();
    let request = StartRequest::parse(body.as_bytes()).unwrap_or_else(|_| unreachable!());
    (request, body)
}

fn large(caller: &str, index: u128) -> (StartRequest, String) {
    start(caller, index, &format!("/{}", "a".repeat(15_000)))
}

fn refused(request: &StartRequest) -> Reply {
    Reply::failure(
        Failure::before_start(StatusCode::UNPROCESSABLE_ENTITY, "resume-account-mismatch"),
        Some(&request.session.move_id),
    )
}

fn claim(
    context: &CliContext,
    request: &StartRequest,
    body: &str,
) -> Result<Option<Retained>, Error> {
    Store::open(context).and_then(|store| store.claim(request, body))
}

fn first_claim(context: &CliContext, request: &StartRequest, body: &str) -> bool {
    matches!(claim(context, request, body), Ok(None))
}

#[test]
fn flooding_caller_cannot_starve_another_caller() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    let mut refusals = 0;
    for index in 1..=300 {
        let (request, body) = large("flooder", index);
        refusals += usize::from(!first_claim(&context, &request, &body));
    }
    assert!(refusals > 0, "an unbounded caller must be refused");

    let (request, body) = large("calm", 9_000);
    assert!(first_claim(&context, &request, &body));
}

#[test]
fn terminal_receipts_drop_the_retained_request_but_keep_the_fence() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    let (request, body) = large("caller", 1);
    assert!(first_claim(&context, &request, &body));
    let reply = refused(&request);
    Store::open(&context)
        .and_then(|store| store.record(&request, reply.clone()))
        .unwrap_or_else(|_| unreachable!());

    let stored = fs::metadata(directory.path().join("coordination").join(STORE))
        .unwrap()
        .len();
    assert!(
        stored < body.len() as u64 / 2,
        "terminal receipt kept the request body"
    );
    let (repeat, _) = large("caller", 1);
    assert!(matches!(
        Store::open(&context).and_then(|mut store| store.existing(&repeat)),
        Ok(Some(Retained::Terminal(_)))
    ));
    let (retargeted, _) = start("caller", 1, "/elsewhere");
    assert!(matches!(
        Store::open(&context).and_then(|mut store| store.existing(&retargeted)),
        Err(Error::Conflict)
    ));
}

#[test]
fn compacted_fences_are_bounded_per_caller() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    let mut accepted = 0;
    for index in 1..=600 {
        let (request, body) = start("flooder", index, "/work");
        if !first_claim(&context, &request, &body) {
            break;
        }
        Store::open(&context)
            .and_then(|store| store.record(&request, refused(&request)))
            .unwrap_or_else(|_| unreachable!());
        accepted += 1;
    }
    assert!(accepted < 600, "terminal fences must be bounded per caller");

    let (request, body) = start("flooder", 1, "/work");
    assert!(matches!(
        claim(&context, &request, &body),
        Ok(Some(Retained::Terminal(_)))
    ));
    let (request, body) = start("calm", 9_000, "/work");
    assert!(first_claim(&context, &request, &body));
}

#[test]
fn conflict_does_not_establish_the_original_outcome() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    let (request, body) = start("caller", 1, "/work");
    assert!(first_claim(&context, &request, &body));

    let (retargeted, _) = start("caller", 1, "/elsewhere");
    let Err(error) =
        Store::open(&context).and_then(|mut store| store.existing(&retargeted).map(|_| ()))
    else {
        panic!("a retargeted move must conflict");
    };
    let (status, reply) = error.reply(&retargeted.session.move_id).into_parts();
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(reply["error"]["code"], "resume-move-binding-mismatch");
    assert!(reply["error"]["details"].get("session_created").is_none());
    assert_eq!(
        reply["error"]["details"]["next_action"],
        "reconcile_destination"
    );
}

#[test]
fn crash_after_the_fence_keeps_the_original_request_for_reconciliation() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    let (request, body) = start("caller", 1, "/work");
    // The first claim authorizes the dispatch. Dropping it models a crash
    // before the forward and before any recorded reply.
    assert!(first_claim(&context, &request, &body));

    let (repeat, repeat_body) = start("caller", 1, "/work");
    for _ in 0..2 {
        let Ok(Some(Retained::Unresolved(retained))) = claim(&context, &repeat, &repeat_body)
        else {
            panic!("a fenced move must never be claimed again");
        };
        assert_eq!(retained, body);
    }
}

#[test]
fn retargeting_a_move_conflicts_for_every_bound_value() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    let (request, body) = start("caller", 1, "/work");
    assert!(first_claim(&context, &request, &body));

    for changed in [
        body.replace("\"destination\"", "\"other\""),
        body.replace("fixture-account", "other-account"),
        body.replace("\"/work\"", "\"/elsewhere\""),
        body.replace("\"claude\"", "\"codex\""),
        body.replace("0000000000b1", "0000000000b2"),
        body.replace("0000000000a1", "0000000000a2"),
    ] {
        assert_ne!(changed, body);
        let retargeted = StartRequest::parse(changed.as_bytes()).unwrap_or_else(|_| unreachable!());
        assert!(
            matches!(claim(&context, &retargeted, &changed), Err(Error::Conflict)),
            "{changed}"
        );
    }
}

#[test]
fn a_caller_cannot_claim_burn_or_probe_another_callers_move() {
    let directory = tempfile::tempdir().unwrap();
    let context = context(&directory);
    let operation = uuid::Uuid::now_v7().to_string();
    let (owner, owner_body) = start_with_operation("owner", 1, "/work", &operation);
    assert!(first_claim(&context, &owner, &owner_body));

    // Probe: another caller learns nothing about the owner's receipt.
    let (other, other_body) = start("other", 1, "/elsewhere");
    assert!(matches!(
        Store::open(&context).and_then(|mut store| store.existing(&other)),
        Ok(None)
    ));
    // Claim: the same move ID is an independent move for another caller.
    assert!(first_claim(&context, &other, &other_body));
    // Burn: reusing the owner's operation ID for another move binds only its own scope.
    let (burner, burner_body) = start_with_operation("other", 2, "/work", &operation);
    assert!(first_claim(&context, &burner, &burner_body));

    let (repeat, repeat_body) = start_with_operation("owner", 1, "/work", &operation);
    let Ok(Some(Retained::Unresolved(retained))) = claim(&context, &repeat, &repeat_body) else {
        panic!("the owner's receipt must be untouched");
    };
    assert_eq!(retained, owner_body);
}
