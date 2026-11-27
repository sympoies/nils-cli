use super::*;
use pretty_assertions::assert_eq;

use reqwest::StatusCode;

const DISCOVER: &str = "/api/coordination/attested-create/v1";
const CREATE: &str = "/api/coordination/sessions/v1";
const ID: &str = "fixture-steward";

fn request() -> Value {
    json!({
        "operation_id": uuid::Uuid::now_v7().to_string(),
        "machine": "host-b",
        "attest_role": true,
        "no_parent": true,
        "session": {"id": ID, "agent": "claude"}
    })
}

fn submit(serve: &Serve, body: &str) -> (StatusCode, Value) {
    let response = reqwest::blocking::Client::new()
        .post(format!(
            "http://{}/sessions/{SESSION}/console-start/v1",
            serve.addr
        ))
        .bearer_auth(CAPABILITY)
        .header("content-type", "application/json")
        .body(body.to_owned())
        .send()
        .expect("origin response");
    (
        response.status(),
        response.json().expect("origin response JSON"),
    )
}

fn capability(machine: &str) -> Value {
    json!({
        "schema_version": "agent-session.console-attested-create-capability.v1",
        "ok": true,
        "data": {"machine": machine, "capability": "console-attested-create/v1"}
    })
}

fn session() -> Value {
    json!({
        "id": ID,
        "role": "steward",
        "lineage": {"forge_context": {
            "initiator": "console-relay", "role": "steward", "attested_role": "steward"
        }}
    })
}

fn assert_discovery(seen: &Seen) {
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.target, DISCOVER);
    assert!(seen.body.is_none());
    assert!(seen.operation_id.is_none());
    assert!(
        seen.authorization
            .as_deref()
            .is_some_and(|header| { header.strip_prefix("Bearer ") == Some(RELAY_TOKEN) })
    );
}

fn assert_error((status, result): (StatusCode, Value), want: StatusCode, code: &str) {
    assert_eq!(result["error"]["code"], code, "{result}");
    assert_eq!(status, want);
    assert!(result.get("session").is_none());
}

fn assert_unavailable(response: (StatusCode, Value)) {
    assert_error(
        response,
        StatusCode::SERVICE_UNAVAILABLE,
        "console-create-attestation-unavailable",
    );
}

#[test]
fn console_attestation_requires_capability_from_the_configured_origin() {
    let fixture = Fixture::new();
    let relay = Aggregator::start();
    let serve = fixture.serve(&[], Some(&relay));
    let mut wrong_schema = capability(MACHINE);
    wrong_schema["schema_version"] = json!("wrong-schema");
    let mut wrong_capability = capability(MACHINE);
    wrong_capability["data"]["capability"] = json!("other-capability");
    for (index, (status, body)) in [
        (404, json!({})),
        (200, capability("other-origin")),
        (200, wrong_schema),
        (200, wrong_capability),
        (503, capability(MACHINE)),
    ]
    .iter()
    .enumerate()
    {
        relay.reply_json(*status, body);
        assert_unavailable(submit(&serve, &request().to_string()));
        let seen = relay.seen();
        assert_eq!(seen.len(), index + 1);
        assert_discovery(&seen[index]);
    }
}

#[test]
fn console_attestation_sends_explicit_intent_and_replays_confirmed_success() {
    let fixture = Fixture::new();
    let relay = Aggregator::start();
    let serve = fixture.serve(&[], Some(&relay));
    relay.reply_json(200, &capability(MACHINE));
    relay.reply_json(201, &json!({"ok": true, "data": {"session": session()}}));

    let body = request();
    let (status, result) = submit(&serve, &body.to_string());
    assert_eq!(status, StatusCode::CREATED, "{result}");
    assert_eq!(result["machine"], "host-b");
    assert_eq!(result["session"], session());
    let seen = relay.seen();
    assert_eq!(seen.len(), 2);
    assert_discovery(&seen[0]);
    let create = &seen[1];
    assert_eq!(create.method, "POST");
    assert_eq!(create.target, CREATE);
    assert!(create.authorization == seen[0].authorization);
    assert_eq!(
        create.operation_id.as_deref(),
        body["operation_id"].as_str()
    );
    let sent = create.body.as_ref().expect("create body");
    assert_eq!(sent["attest_role"], true);
    assert_eq!(sent["no_parent"], true);
    assert_eq!(sent["source_session_id"], SESSION);
    assert_eq!(sent["source_incarnation"], INCARNATION);
    assert_eq!(sent["machine"], "host-b");
    assert_eq!(sent["session"]["id"], ID);
    assert!(sent["session"].get("lineage").is_none());
    assert!(sent["session"].get("origin").is_none());
    assert!(sent["session"].get("role").is_none());

    let (status, replay) = submit(
        &serve,
        &json!({"operation_id": body["operation_id"], "reconcile": true}).to_string(),
    );
    assert_eq!(status, StatusCode::CREATED, "{replay}");
    assert_eq!(replay["reconciliation"]["state"], "created");
    assert_eq!(replay["session"], session());
    assert_eq!(relay.seen().len(), 2);
}

#[test]
fn console_attestation_requires_exact_id_role_and_complete_launch_context() {
    let fixture = Fixture::new();
    let relay = Aggregator::start();
    let serve = fixture.serve(&[], Some(&relay));
    for (index, (pointer, wrong)) in [
        ("/id", json!("different-session")),
        ("/role", json!("worker")),
        ("/lineage/forge_context/initiator", json!("operator")),
        ("/lineage/forge_context/role", Value::Null),
        ("/lineage/forge_context/attested_role", json!("worker")),
    ]
    .into_iter()
    .enumerate()
    {
        let mut wrong_session = session();
        *wrong_session.pointer_mut(pointer).expect("fixture field") = wrong;
        relay.reply_json(200, &capability(MACHINE));
        relay.reply_json(
            201,
            &json!({"ok": true, "data": {"session": wrong_session}}),
        );
        assert_unavailable(submit(&serve, &request().to_string()));
        let seen = relay.seen();
        assert_eq!(seen.len(), (index + 1) * 2);
        assert_discovery(&seen[index * 2]);
        assert_eq!(seen[index * 2 + 1].path(), CREATE);
    }
}

#[test]
fn console_attestation_retries_an_unconfirmed_reply_with_the_same_operation() {
    let fixture = Fixture::new();
    let relay = Aggregator::start();
    let serve = fixture.serve(&[], Some(&relay));
    relay.reply_json(200, &capability(MACHINE));
    relay.reply_json(201, &json!({"ok": true, "data": {"session": {"id": ID}}}));
    let body = request();
    assert_unavailable(submit(&serve, &body.to_string()));

    let lookup = json!({"operation_id": body["operation_id"], "reconcile": true});
    let (status, result) = submit(&serve, &lookup.to_string());
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(result["reconciliation"]["state"], "ambiguous");
    assert!(result.get("session").is_none());
    assert_eq!(relay.seen().len(), 2);

    relay.reply_json(200, &capability(MACHINE));
    relay.reply_json(201, &json!({"ok": true, "data": {"session": session()}}));
    let (status, result) = submit(&serve, &body.to_string());
    assert_eq!(status, StatusCode::CREATED, "{result}");
    assert_eq!(result["session"], session());
    let seen = relay.seen();
    assert_eq!(seen.len(), 4);
    assert_discovery(&seen[2]);
    assert_eq!(seen[1].operation_id, seen[3].operation_id);
    assert_eq!(seen[1].body, seen[3].body);
}

#[test]
fn console_attestation_rejects_invalid_intent_and_untrusted_root_inputs() {
    let fixture = Fixture::new();
    let relay = Aggregator::start();
    let serve = fixture.serve(&[], Some(&relay));
    let base = r#"{"session":{"id":"fixture-steward","agent":"claude"},"no_parent":true"#;
    for (input, code) in [
        (format!(r#"{base},"attest_role":false,"attest_role":true}}"#), "console-create-invalid-body"),
        (format!(r#"{base},"attest_role":"true"}}"#), "console-create-invalid-body"),
        (format!(r#"{base},"Attest_Role":true}}"#), "console-create-invalid-body"),
        (r#"{"session":{"id":"fixture-steward","agent":"claude","attest_role":true},"no_parent":true}"#.into(), "console-create-invalid-body"),
        (r#"{"attest_role":true,"no_parent":true,"session":{"agent":"claude"}}"#.into(), "console-create-invalid-body"),
        (format!(r#"{base},"attest_role":true,"role":null}}"#), "identity_session_binding_untrusted"),
        (r#"{"attest_role":true,"no_parent":true,"session":{"id":"fixture-steward","agent":"claude","role":null}}"#.into(), "identity_session_binding_untrusted"),
        (r#"{"attest_role":true,"session":{"id":"fixture-steward","agent":"claude"}}"#.into(), "role-requires-root"),
        (r#"{"attest_role":true,"no_parent":true,"session":{"id":"fixture-steward","agent":"claude","lineage":{"depth":0}}}"#.into(), "role-requires-root"),
        (r#"{"attest_role":true,"no_parent":true,"session":{"id":"fixture-steward","agent":"claude","provider_resume_id":"fixture"}}"#.into(), "resume-unsupported"),
    ] {
        assert_error(submit(&serve, &input), StatusCode::BAD_REQUEST, code);
    }
    assert!(relay.seen().is_empty());
}

#[test]
fn console_attestation_preserves_typed_refusal_without_relay_diagnostics() {
    let fixture = Fixture::new();
    let relay = Aggregator::start();
    let serve = fixture.serve(&[], Some(&relay));
    relay.reply_json(200, &capability(MACHINE));
    relay.reply_json(
        403,
        &json!({"ok": false, "error": {
            "code": "console-create-attestation-forbidden",
            "message": "private fixture diagnostic",
            "details": {"raw": "private fixture diagnostic"}
        }}),
    );
    let response = submit(&serve, &request().to_string());
    assert!(
        !response
            .1
            .to_string()
            .contains("private fixture diagnostic")
    );
    assert_error(
        response,
        StatusCode::FORBIDDEN,
        "console-create-attestation-forbidden",
    );
    assert_eq!(relay.seen().len(), 2);
}
