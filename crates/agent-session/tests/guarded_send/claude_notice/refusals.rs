//! Pre-claim protocol diagnostics through the public JSON command boundary.
use super::*;
use pretty_assertions::assert_eq;

pub(super) fn no_claim(fixture: &super::super::Fixture) {
    let session = fixture.root().join("state/sessions/guarded");
    assert!(!fs::read_dir(session).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("send-key-")
    }));
    assert!(
        !fixture
            .root()
            .join("state/history/claude-notices/ledger.jsonl")
            .exists()
    );
    fixture.no_delivery();
}

#[test]
fn preclaim_authorization_refusals_identify_the_failed_check() {
    for (field, value, reason) in [
        ("path", json!("/result"), "authorization-path"),
        ("schema_version", json!("unknown"), "authorization-schema"),
        ("op", json!("wrong-operation"), "authorization-operation"),
        ("binding", Value::Null, "authorization-binding"),
        ("nonce", json!("wrong-nonce"), "authorization-nonce"),
        ("module", Value::Null, "authorization-binding"),
        ("runtime", Value::Null, "authorization-binding"),
        ("terminal", json!(false), "authorization-surface"),
        (
            "box_empty",
            json!(false),
            "authorization-composer-not-empty",
        ),
        ("cursor", json!(1), "authorization-cursor"),
        ("malformed", Value::Null, "authorization-payload"),
        ("readiness-missing", Value::Null, "readiness-unavailable"),
        ("readiness", Value::Null, "readiness-changed"),
    ] {
        let fixture = super::super::Fixture::new("claude");
        let (dir, boot) = bind_modeled_peer(&fixture);
        let thread = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let offer = loop {
                if let Ok(bytes) = fs::read(dir.join("offer.json"))
                    && let Ok(offer) = serde_json::from_slice::<Value>(&bytes)
                    && offer["op"].is_string()
                {
                    break offer;
                }
                assert!(Instant::now() < deadline, "offer not published");
                std::thread::sleep(Duration::from_millis(2));
            };
            let mut request = json!({"schema_version":"agent-session.claude-notice-authorize.v1",
                "op":offer["op"], "binding":offer["binding"], "nonce":boot["nonce"],
                "terminal":true, "box_empty":true, "cursor":0});
            let mut path = "/authorize";
            match field {
                "path" => path = "/result",
                "binding" => request["binding"]["provider_session_id"] = json!("other-provider"),
                "module" => request["binding"]["module_sha256"] = json!("a".repeat(64)),
                "runtime" => request["binding"]["launch"]["incarnation"] = json!("other-runtime"),
                "malformed" => {
                    request.as_object_mut().unwrap().remove("nonce");
                }
                "readiness-missing" => fs::remove_file(dir.join("ready.json")).unwrap(),
                "readiness" => {
                    for name in ["bootstrap.json", "ready.json"] {
                        let path = dir.join(name);
                        let mut value: Value =
                            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                        value["nonce"] = json!("a".repeat(64));
                        fs::write(path, value.to_string()).unwrap();
                    }
                }
                _ => request[field] = value,
            }
            assert_eq!(
                post(
                    Path::new(boot["socket_path"].as_str().unwrap()),
                    path,
                    &request
                ),
                None,
                "a refused request must not receive notice text"
            );
        });
        let output = send_notice(&fixture, "synthetic\n");
        thread.join().unwrap();
        let result = fixture.refused(&output, "notice-native-unavailable");
        assert_eq!(result["error"]["details"]["reason"], reason, "{field}");
        assert!(
            !output
                .stdout_text()
                .contains(fixture.root().to_str().unwrap())
        );
        no_claim(&fixture);
    }
}

#[test]
fn preclaim_receive_timeout_and_protocol_failures_have_distinct_reasons() {
    for (case, reason) in [
        ("timeout", "authorization-timeout"),
        ("protocol", "authorization-protocol"),
        ("closed", "authorization-connection"),
    ] {
        let fixture = super::super::Fixture::new("claude");
        let (dir, boot) = bind_modeled_peer(&fixture);
        let thread = std::thread::spawn(move || {
            if case == "timeout" {
                return;
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Ok(bytes) = fs::read(dir.join("offer.json"))
                    && let Ok(offer) = serde_json::from_slice::<Value>(&bytes)
                    && offer["op"].is_string()
                {
                    break;
                }
                assert!(Instant::now() < deadline, "offer not published");
                std::thread::sleep(Duration::from_millis(2));
            }
            let mut stream = UnixStream::connect(boot["socket_path"].as_str().unwrap()).unwrap();
            if case == "protocol" {
                stream
                    .write_all(b"GET /authorize HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}")
                    .unwrap();
                // Keep the read side alive until rejection. Dropping a socket
                // with unread bytes can reset it on macOS before the server
                // parses the malformed header, testing transport failure instead.
                stream.shutdown(std::net::Shutdown::Write).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let mut response = Vec::new();
                match stream.read_to_end(&mut response) {
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                    Err(error) => panic!("protocol rejection was not received: {error}"),
                }
                assert!(response.is_empty(), "malformed request received a response");
            }
        });
        let result = fixture.refused(
            &send_notice(&fixture, "synthetic\n"),
            "notice-native-unavailable",
        );
        thread.join().unwrap();
        assert_eq!(result["error"]["details"]["reason"], reason);
        no_claim(&fixture);
    }
}

#[test]
fn preclaim_listener_open_refusal_identifies_the_failed_check() {
    let fixture = super::super::Fixture::new("claude");
    let (_dir, boot) = bind_modeled_peer(&fixture);
    fs::write(boot["socket_path"].as_str().unwrap(), "occupied").unwrap();
    let result = fixture.refused(
        &send_notice(&fixture, "synthetic\n"),
        "notice-native-unavailable",
    );
    assert_eq!(result["error"]["details"]["reason"], "listener-open");
    no_claim(&fixture);
}
