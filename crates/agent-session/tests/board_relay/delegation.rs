use super::*;
use pretty_assertions::assert_eq;

fn mint(fixture: &Fixture, operations: &[&str]) -> Value {
    let mut args = vec!["mint", "--session", SESSION, "--format", "json"];
    args.extend_from_slice(operations);
    let output = fixture.run("delegation", &args, true);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    output.stdout_json()["data"].clone()
}

fn worker(fixture: &Fixture, credential: &Path, args: &[&str]) -> CmdOutput {
    fixture.heartbeat();
    let mut argv = vec!["--state-dir", fixture.state_dir.to_str().unwrap()];
    argv.extend_from_slice(args);
    let options = CmdOptions::new()
        .with_cwd(&fixture.root)
        .without_ambient_managed_session_env()
        .with_env_remove_many(&ISOLATED_ENV)
        .with_envs(&[
            ("HOME", fixture.home.to_str().unwrap()),
            ("AGENT_SESSION_MACHINE", MACHINE),
            (
                "AGENT_SESSION_CAPABILITY_FILE",
                credential.to_str().unwrap(),
            ),
        ]);
    run_resolved("agent-session", &argv, &options)
}

#[test]
fn delegation_worker_starts_both_harnesses_without_session_identity_or_checkpoint() {
    for agent in ["codex", "claude"] {
        let fixture = Fixture::new();
        let aggregator = Aggregator::start();
        let _serve = fixture.serve(&[], Some(&aggregator));
        let delegation = mint(&fixture, &["--operation", "start-via-console"]);
        let path = Path::new(delegation["credential_file"].as_str().unwrap());
        aggregator.reply_json(
            201,
            &json!({"ok":true,"data":{"session":{"id":"worker-child","agent":agent}}}),
        );
        let output = worker(
            &fixture,
            path,
            &[
                "start",
                "--via-console",
                "--agent",
                agent,
                "--format",
                "json",
            ],
        );
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["data"]["session"]["id"],
            "worker-child"
        );
        let output = fixture.run(
            "delegation",
            &[
                "revoke",
                "--session",
                SESSION,
                "--id",
                delegation["id"].as_str().unwrap(),
                "--format",
                "json",
            ],
            true,
        );
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        let output = worker(
            &fixture,
            path,
            &[
                "start",
                "--via-console",
                "--agent",
                agent,
                "--format",
                "json",
            ],
        );
        assert_eq!(error_of(&output)["code"], "delegation-revoked");
        let registry: Value = serde_json::from_slice(
            &fs::read(fixture.state_dir.join("coordination/registry.json")).unwrap(),
        )
        .unwrap();
        let audit = registry["delegation_audit"].as_array().unwrap();
        assert_eq!(audit[0]["outcome"], "allowed");
        let token = fs::read_to_string(path).unwrap();
        assert!(!registry.to_string().contains(&token));
        assert!(!output.stdout_text().contains(&token));
    }
}

#[test]
fn delegation_remote_send_enforces_exact_recipient_and_machine_at_the_daemon() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    let delegation = mint(
        &fixture,
        &[
            "--operation",
            "message-send",
            "--recipient",
            "remote-machine/receipt-owner",
        ],
    );
    let path = Path::new(delegation["credential_file"].as_str().unwrap());
    let token = fs::read_to_string(path).unwrap();
    let body = json!({"to_machine":"other-machine", "to_session":"receipt-owner", "body":"worker receipt", "idempotency_key":"wrong-machine"});
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(format!(
            "http://{}/sessions/{SESSION}/messages/remote/v1",
            serve.addr
        ))
        .bearer_auth(&token)
        .json(&body)
        .send()
        .unwrap();
    let refused: Value = response.json().unwrap();
    assert_eq!(refused["error"]["code"], "delegation-scope-denied");
    let mut body = body;
    body["to_machine"] = json!("remote-machine");
    body["idempotency_key"] = json!("allowed-send");
    aggregator.reply_json(200, &json!({"schema_version":"agent-session.remote-peers.v1", "peers":[{"machine":"remote-machine", "session_id":"receipt-owner", "session_incarnation":"target-incarnation", "messaging_supported":true}]}));
    let response = client
        .post(format!(
            "http://{}/sessions/{SESSION}/messages/remote/v1",
            serve.addr
        ))
        .bearer_auth(&token)
        .json(&body)
        .send()
        .unwrap();
    let result: Value = response.json().unwrap();
    assert_eq!(result["state"], "queued", "{result}");
    assert_eq!(result["recipient"]["session_id"], "receipt-owner");
    let receipt = fixture.root.join("receipt.txt");
    fs::write(&receipt, "worker receipt").unwrap();
    let output = worker(
        &fixture,
        path,
        &[
            "message",
            "send",
            "--from",
            SESSION,
            "--to",
            "receipt-owner",
            "--to-machine",
            "remote-machine",
            "--body-file",
            receipt.to_str().unwrap(),
            "--idempotency-key",
            "allowed-send",
            "--format",
            "json",
        ],
    );
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["data"]["message_id"],
        result["message_id"]
    );

    let audit: Value = serde_json::from_slice(
        &fs::read(fixture.state_dir.join("coordination/registry.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        audit["delegation_audit"][0]["outcome"],
        "delegation-scope-denied"
    );
    assert_eq!(audit["delegation_audit"][1]["outcome"], "allowed");
    assert!(!audit.to_string().contains(&token));
}
