use super::*;
use pretty_assertions::assert_eq;

const VALID: &str =
    r#"{"schema_version":"agent-session.operator-rules.v1","coordinator_requires_root":false}"#;
const INVALID: &str = r#"{"schema_version":"agent-session.operator-rules.v1","coordinator_requires_root":false,"unexpected":true}"#;

fn write_rules(fixture: &Fixture, raw: &str) {
    fs::write(fixture.root.join("rules.json"), raw).unwrap();
}

fn post(serve: &Serve, route: &str, bearer: &str, body: Value) -> Value {
    reqwest::blocking::Client::new()
        .post(format!("http://{}{route}", serve.addr))
        .bearer_auth(bearer)
        .json(&body)
        .send()
        .unwrap()
        .json()
        .unwrap()
}

#[test]
fn operator_rules_apply_to_cli_console_start_and_daemon_relay_for_both_roles() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    write_rules(&fixture, VALID);
    let serve = fixture.serve(&[], Some(&aggregator));
    for role in ["coordinator", "domain-coordinator"] {
        let child = json!({"id":role, "agent":"claude"});
        aggregator.reply_json(201, &json!({"ok":true,"data":{"session":child}}));
        let output = fixture.start_via_console(
            &[
                "--agent",
                "claude",
                "--cwd",
                "/work/repo",
                "--role",
                role,
                "--format",
                "json",
            ],
            true,
        );
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        let seen = aggregator.seen();
        let session = &seen.last().unwrap().body.as_ref().unwrap()["session"];
        assert_eq!(session["role"], role);
        assert_eq!(session["lineage"]["parent"]["session_id"], SESSION);

        // Bypass CLI validation: the daemon independently reads the same rule.
        aggregator.reply_json(201, &json!({"ok":true,"data":{"session":child}}));
        let body = post(
            &serve,
            &format!("/sessions/{SESSION}/console-start/v1"),
            CAPABILITY,
            json!({"role":role,"session":{"agent":"claude","cwd":"/work/repo"}}),
        );
        assert_eq!(body["session"]["id"], role, "{body}");
        let seen = aggregator.seen();
        assert_eq!(
            seen.last().unwrap().body.as_ref().unwrap()["session"]["role"],
            role
        );
    }
}

#[test]
fn operator_rules_http_create_resume_and_metadata_follow_reload_and_last_good() {
    let fixture = Fixture::new();
    let serve = fixture.serve(&[], None);
    let body = |role: &str, resume: bool| {
        let mut body = json!({"agent":"claude","cwd":"/work/repo","role":role,"lineage":child_lineage(),
            // This valid import deliberately stops after lineage validation.
            "provider_resume_id":"12345678-1234-1234-1234-123456789abc"});
        if resume {
            body.as_object_mut().unwrap().remove("provider_resume_id");
            body["resume"] = json!("12345678-1234-1234-1234-123456789abc");
        }
        body
    };
    for role in ["coordinator", "domain-coordinator"] {
        for (route, resume) in [("/sessions", false), ("/sessions/resume/v1", true)] {
            let result = post(&serve, route, OPERATOR, body(role, resume));
            assert_eq!(result["error"]["code"], "role-requires-root", "{result}");
        }
    }
    write_rules(&fixture, VALID);
    for raw in [VALID, INVALID] {
        write_rules(&fixture, raw);
        for role in ["coordinator", "domain-coordinator"] {
            let result = post(&serve, "/sessions", OPERATOR, body(role, false));
            assert_eq!(
                result["error"]["code"], "provider-resume-cwd-conflict",
                "{result}"
            );
            let result = post(&serve, "/sessions/resume/v1", OPERATOR, body(role, true));
            assert_eq!(
                result["error"]["code"], "resume-history-missing",
                "{result}"
            );
        }
    }
    let (_, effective) = serve.get("/operator-rules", None);
    assert_eq!(
        effective["data"]["rules"]["coordinator_requires_root"],
        false
    );
    assert_eq!(effective["data"]["source"], "file");
    assert_eq!(effective["data"]["retained_last_good"], true);
    // Metadata updates use the same live rule on an already parented record.
    let path = fixture
        .state_dir
        .join("sessions")
        .join(SESSION)
        .join("session.json");
    let mut original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut lineage = child_lineage();
    lineage["machine"] = json!(MACHINE);
    original["lineage"] = lineage;
    fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
    for (revision, role) in ["coordinator", "domain-coordinator"]
        .into_iter()
        .enumerate()
    {
        let result = post(
            &serve,
            &format!("/sessions/{SESSION}/display-metadata"),
            OPERATOR,
            json!({"expected_session_created_at":original["created_at"],"expected_revision":revision,"role":role}),
        );
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["data"]["session"]["role"], role);
    }
    let stderr = fs::read_to_string(fixture.root.join("serve.stderr")).unwrap();
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.contains("operator rules"))
            .count(),
        1,
        "{stderr}"
    );
    assert!(stderr.contains("rules.json") && stderr.contains("unexpected"));

    // Removing the document deliberately restores defaults, including in serve.
    fs::remove_file(fixture.root.join("rules.json")).unwrap();
    let result = post(&serve, "/sessions", OPERATOR, body("coordinator", false));
    assert_eq!(result["error"]["code"], "role-requires-root");
    let (_, effective) = serve.get("/operator-rules", None);
    assert_eq!(effective["data"]["source"], "built_in");
    assert_eq!(effective["data"]["retained_last_good"], false);
}

#[test]
fn corrupt_operator_rules_leave_mailbox_send_and_read_available() {
    let fixture = Fixture::new();
    write_rules(&fixture, INVALID);
    let _serve = fixture.serve(&[], None);
    let message = fixture.root.join("body.txt");
    fs::write(&message, "fixture message").unwrap();
    let sent = fixture.run(
        "message",
        &[
            "send",
            "--from",
            SESSION,
            "--to",
            SESSION,
            "--body-file",
            message.to_str().unwrap(),
            "--idempotency-key",
            "operator-rules-mail",
            "--format",
            "json",
        ],
        true,
    );
    assert_eq!(sent.code, 0, "{}", sent.stdout_text());
    let id = sent.stdout_json()["data"]["message_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let read = fixture.run(
        "message",
        &[
            "show",
            "--session",
            SESSION,
            "--message",
            &id,
            "--format",
            "json",
        ],
        true,
    );
    assert_eq!(read.code, 0, "{}", read.stdout_text());
    assert!(read.stdout_text().contains("fixture message"));
    for output in [sent, read] {
        assert_eq!(output.stderr_text().matches("warning:").count(), 1);
        assert!(
            output.stderr_text().contains("rules.json")
                && output.stderr_text().contains("unexpected")
        );
    }
    let stderr = fs::read_to_string(fixture.root.join("serve.stderr")).unwrap();
    assert_eq!(
        stderr
            .lines()
            .filter(|line| line.contains("operator rules"))
            .count(),
        1,
        "{stderr}"
    );
}

#[test]
fn operator_rules_check_is_shared_by_cli_and_serve_and_reports_effective_source() {
    let fixture = Fixture::new();
    for command in ["rules", "serve"] {
        let output = fixture.run(command, &["--check", "--format", "json"], false);
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(output.stdout_json()["data"]["source"], "built_in");
        assert_eq!(
            output.stdout_json()["data"]["rules"]["coordinator_requires_root"],
            true
        );
    }
    write_rules(&fixture, VALID);
    let output = fixture.run("rules", &["--format", "json"], false);
    assert_eq!(output.stdout_json()["data"]["source"], "file");
    assert_eq!(
        output.stdout_json()["data"]["rules"]["coordinator_requires_root"],
        false
    );
    assert!(output.stdout_text().contains("rules.json"));
    for (raw, key) in [
        (INVALID, "unexpected"),
        ("{", "$document"),
        (r#"{"schema_version":"unknown"}"#, "schema_version"),
        (
            r#"{"schema_version":"agent-session.operator-rules.v1","coordinator_requires_root":"no"}"#,
            "coordinator_requires_root",
        ),
    ] {
        write_rules(&fixture, raw);
        for command in ["rules", "serve"] {
            let output = fixture.run(command, &["--check", "--format", "json"], false);
            assert_eq!(output.code, 64, "{}", output.stdout_text());
            assert_eq!(output.stdout_json()["error"]["details"]["key"], key);
            assert!(output.stdout_text().contains("rules.json"));
        }
    }
}

#[test]
fn operator_rules_use_the_shared_xdg_or_home_location_without_an_override() {
    let fixture = Fixture::new();
    let config = fixture.root.join("config");
    for (xdg, base) in [
        (Some(config.as_path()), config.clone()),
        (None, fixture.home.join(".config")),
    ] {
        let path = base.join("agent-session/rules.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, VALID).unwrap();
        let options = CmdOptions::new()
            .with_cwd(&fixture.root)
            .without_ambient_managed_session_env()
            .with_env_remove_many(&["AGENT_SESSION_RULES_FILE", "XDG_CONFIG_HOME"])
            .with_env("HOME", fixture.home.to_str().unwrap());
        let options = match xdg {
            Some(path) => options.with_env("XDG_CONFIG_HOME", path.to_str().unwrap()),
            None => options,
        };
        let output = run_resolved("agent-session", &["rules", "--format", "json"], &options);
        assert_eq!(output.code, 0, "{}", output.stderr_text());
        assert_eq!(
            output.stdout_json()["data"]["file"],
            path.to_string_lossy().as_ref()
        );
        assert_eq!(
            output.stdout_json()["data"]["rules"]["coordinator_requires_root"],
            false
        );
    }
}
