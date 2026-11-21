use pretty_assertions::assert_eq;

use super::*;

const LOCAL_ROUTE: &str = "/coordination/companion/messages/v1";
const EDGE_ROUTE: &str = "/api/coordination/prompter/v1/messages";
const BODY: &str = "Synthetic companion message\n";
const ID: &str = "18446744073709551615";
const CHECK_INBOX: &str = "check the companion inbox before submitting again";

fn send(fixture: &Fixture, body: &[u8], args: &[&str]) -> CmdOutput {
    let path = fixture.root.join("companion-body.txt");
    fs::write(&path, body).expect("fixture body");
    let path = path.to_str().expect("UTF-8 fixture path");
    let mut argv = vec!["send-companion", "--body-file", path];
    argv.extend_from_slice(args);
    fixture.run("message", &argv, true)
}

fn post(daemon: &Serve, bearer: &str, body: &Value) -> Value {
    reqwest::blocking::Client::new()
        .post(format!("http://{}{LOCAL_ROUTE}", daemon.addr))
        .bearer_auth(bearer)
        .json(body)
        .send()
        .expect("daemon response")
        .json()
        .expect("daemon JSON")
}

fn receipt() -> Value {
    json!({"ok": true, "data": {"id": ID}})
}

fn assert_error(output: &CmdOutput, exit: i32, code: &str) -> Value {
    assert_eq!(output.code, exit, "{}", output.stderr_text());
    let json: Value = serde_json::from_slice(&output.stdout).expect("CLI error JSON");
    assert_eq!(
        json["schema_version"],
        "cli.agent-session.message-send-companion.v1"
    );
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], code);
    json
}

#[test]
fn sends_exact_source_and_returns_opaque_receipt() {
    let fixture = Fixture::new();
    let edge = Aggregator::start();
    let _daemon = fixture.serve(&[], Some(&edge));

    edge.reply_json(201, &receipt());
    let result = send(
        &fixture,
        BODY.as_bytes(),
        &["--format", "json", "--color", "#c2E5f2", "--urgent"],
    );
    assert_eq!(result.code, 0, "{}", result.stderr_text());
    assert_eq!(
        serde_json::from_slice::<Value>(&result.stdout).unwrap(),
        json!({"schema_version":"cli.agent-session.message-send-companion.v1","ok":true,"data":{"id":ID}})
    );
    let seen = edge.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].target, EDGE_ROUTE);
    assert_eq!(
        seen[0].authorization.as_deref(),
        Some(format!("Bearer {RELAY_TOKEN}").as_str())
    );
    assert_eq!(
        seen[0].body,
        Some(
            json!({"source_session_id":SESSION,"source_incarnation":INCARNATION,"body":BODY,"color":"#c2E5f2","urgent":true})
        )
    );

    edge.reply_json(201, &receipt());
    let result = send(&fixture, BODY.as_bytes(), &[]);
    assert_eq!(result.code, 0, "{}", result.stderr_text());
    assert_eq!(result.stdout_text(), format!("{ID}\n"));
    let seen = edge.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[1].body,
        Some(json!({"source_session_id":SESSION,"source_incarnation":INCARNATION,"body":BODY}))
    );

    let help = fixture.run("message", &["send-companion", "--help"], false);
    assert_eq!(help.code, 0);
    assert!(help.stdout_text().to_lowercase().contains(CHECK_INBOX));
}

#[test]
fn refuses_stale_capabilities_other_bearers_and_injected_fields() {
    let fixture = Fixture::new();
    let edge = Aggregator::start();
    let daemon = fixture.serve(&[], Some(&edge));
    let body = json!({"body": BODY});
    for token in [
        OPERATOR,
        RELAY_TOKEN,
        INGRESS_TOKEN,
        "unrecognized-session-capability-0000000000",
    ] {
        assert_eq!(
            post(&daemon, token, &body)["error"]["code"],
            "companion-source-forbidden"
        );
    }
    for field in [
        "source_session_id",
        "source_incarnation",
        "owner",
        "machine",
        "recipient",
        "token",
    ] {
        let mut injected = body.clone();
        injected[field] = json!("synthetic-injection");
        assert_eq!(
            post(&daemon, CAPABILITY, &injected)["error"]["code"],
            "companion-input-invalid"
        );
    }

    let path = fixture
        .state_dir
        .join("sessions")
        .join(SESSION)
        .join("session.json");
    let mut replacement: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    replacement["runtime"]["launch_id"] = json!("replacement-incarnation");
    private_file(&path, &serde_json::to_vec(&replacement).unwrap());
    let output = send(&fixture, BODY.as_bytes(), &["--format", "json"]);
    assert_error(&output, 65, "companion-source-forbidden");
    assert!(edge.seen().is_empty());
}

#[test]
fn checks_input_at_cli_and_daemon_boundaries() {
    let fixture = Fixture::new();
    let edge = Aggregator::start();
    let daemon = fixture.serve(&[], Some(&edge));
    let exact = "é".repeat(2048);
    let overflow = format!("{exact}x");

    for (body, color, exit, code) in [
        (&[0xff][..], "#123456", 64, "companion-input-invalid"),
        (
            " \n\u{2003}".as_bytes(),
            "#123456",
            64,
            "companion-input-invalid",
        ),
        (BODY.as_bytes(), "#12345g", 64, "companion-input-invalid"),
        (
            overflow.as_bytes(),
            "#123456",
            65,
            "companion-body-too-large",
        ),
    ] {
        let output = send(&fixture, body, &["--format", "json", "--color", color]);
        assert_error(&output, exit, code);
    }
    for (body, code) in [
        (json!({"body":" \n\u{2003}"}), "companion-input-invalid"),
        (
            json!({"body":BODY,"color":"#12345g"}),
            "companion-input-invalid",
        ),
        (json!({"body":overflow}), "companion-body-too-large"),
    ] {
        assert_eq!(post(&daemon, CAPABILITY, &body)["error"]["code"], code);
    }
    assert!(edge.seen().is_empty());

    edge.reply_json(201, &receipt());
    let output = send(&fixture, exact.as_bytes(), &[]);
    assert_eq!(output.code, 0, "{}", output.stderr_text());
    let seen = edge.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body.as_ref().unwrap()["body"], exact);
}

#[test]
fn maps_only_documented_refusals_and_keeps_diagnostics_private() {
    let fixture = Fixture::new();
    let edge = Aggregator::start();
    let _daemon = fixture.serve(&[], Some(&edge));
    let canaries = [BODY.trim(), CAPABILITY, RELAY_TOKEN, "companion-body.txt"];
    for (index, (status, relay_code, exit, cli_code)) in [
        (400, "invalid-request", 64, "companion-input-invalid"),
        (401, "unauthorized", 69, "companion-relay-unavailable"),
        (403, "source-forbidden", 65, "companion-source-forbidden"),
        (
            409,
            "session-incarnation-conflict",
            65,
            "companion-source-forbidden",
        ),
        (413, "body-too-large", 65, "companion-body-too-large"),
        (429, "inbox-full", 65, "companion-inbox-full"),
        (503, "source-unavailable", 69, "companion-relay-unavailable"),
        (500, "inbox-error", 1, "companion-submit-ambiguous"),
        (503, "inbox-full", 1, "companion-submit-ambiguous"),
    ]
    .into_iter()
    .enumerate()
    {
        edge.reply_json(
            status,
            &json!({"ok":false,"error":{"code":relay_code,"message":canaries.join(" ")}}),
        );
        let output = send(&fixture, BODY.as_bytes(), &["--format", "json"]);
        let failure = assert_error(&output, exit, cli_code);
        assert_eq!(edge.seen().len(), index + 1);
        if exit == 1 {
            assert!(
                failure["error"]["message"]
                    .as_str()
                    .unwrap()
                    .to_lowercase()
                    .contains(CHECK_INBOX)
            );
        }
        for canary in canaries {
            assert!(!output.stdout_text().contains(canary));
            assert!(!output.stderr_text().contains(canary));
        }
    }
    let stderr = fs::read_to_string(fixture.root.join("serve.stderr")).unwrap();
    for canary in canaries {
        assert!(!stderr.contains(canary));
    }
}

#[test]
fn reports_ambiguity_without_retrying_either_hop() {
    let fixture = Fixture::new();
    let edge = Aggregator::start();
    let daemon = fixture.serve(&[], Some(&edge));

    for (index, reply) in [
        String::new(),
        raw(201, r#"{"ok":true,"data":{"id":15}}"#, ""),
        raw(307, "", &format!("Location: {}/redirect\r\n", edge.url)),
    ]
    .into_iter()
    .enumerate()
    {
        edge.reply(reply);
        let output = send(&fixture, BODY.as_bytes(), &[]);
        assert_eq!(output.code, 1, "{}", output.stderr_text());
        assert!(output.stderr_text().to_lowercase().contains(CHECK_INBOX));
        assert_eq!(edge.seen().len(), index + 1);
    }

    drop(daemon);
    let local = Aggregator::start();
    local.reply(String::new());
    let endpoint = fixture.state_dir.join("coordination/daemon-endpoint.json");
    private_file(
        &endpoint,
        &serde_json::to_vec(&json!({"url":local.url})).unwrap(),
    );
    let output = send(&fixture, BODY.as_bytes(), &["--format", "json"]);
    assert_error(&output, 1, "companion-submit-ambiguous");
    assert_eq!(local.seen().len(), 1);
    assert_eq!(
        local.seen()[0].authorization.as_deref(),
        Some(format!("Bearer {CAPABILITY}").as_str())
    );
}

#[test]
fn reports_unavailability_before_a_request_can_reach_the_edge() {
    let fixture = Fixture::new();
    let output = send(&fixture, BODY.as_bytes(), &["--format", "json"]);
    assert_error(&output, 69, "companion-relay-unavailable");
    let _daemon = fixture.serve(&[], None);
    let output = send(&fixture, BODY.as_bytes(), &["--format", "json"]);
    assert_error(&output, 69, "companion-relay-unavailable");
}
