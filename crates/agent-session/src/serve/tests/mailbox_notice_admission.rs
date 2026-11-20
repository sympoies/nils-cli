use super::*;
use pretty_assertions::assert_eq;

fn update_json(path: &Path, update: impl FnOnce(&mut Value)) {
    let mut document: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    update(&mut document);
    fs::write(path, serde_json::to_vec(&document).unwrap()).unwrap();
}

fn refuse_before_authorization_release(
    dir: PathBuf,
    socket: PathBuf,
    binding: Value,
    nonce: String,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use std::io::{Read, Write};

        let deadline = Instant::now() + Duration::from_secs(10);
        let offer: Value = loop {
            let offer: Value =
                serde_json::from_slice(&fs::read(dir.join("offer.json")).unwrap()).unwrap();
            if offer["op"].is_string() {
                break offer;
            }
            assert!(Instant::now() < deadline, "receiver did not get an offer");
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut stream = std::os::unix::net::UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let body = json!({
            "schema_version":"agent-session.claude-notice-authorize.v1",
            "op":offer["op"], "binding":binding, "nonce":nonce,
            "terminal":true, "box_empty":true, "cursor":0
        })
        .to_string();
        write!(
            stream,
            "POST /authorize HTTP/1.1\r\nHost: notice.invalid\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(
            response.is_empty(),
            "a refused wake must release no notice text"
        );
    })
}

#[tokio::test]
async fn mailbox_notice_claude_native_admission_records_the_refusing_guard() {
    let lock = GlobalStateLock::new();
    let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
    let provider_not_ready = "provider-not-ready";
    for (reason, detail) in [
        (provider_not_ready, "notice-module-binding-mismatch"),
        (provider_not_ready, "notice-baseline-unprovable"),
        (provider_not_ready, "send-session-not-idle"),
        (provider_not_ready, "notice-recipient-attached"),
        (
            "recipient-coordination-not-quiescent",
            "broker-not-authoritative",
        ),
    ] {
        let fixture = WakeFixture::new(AgentKind::Claude).await;
        configuration_fallback::bind_idle(&fixture);
        let (_socket_root, dir, socket, binding, nonce) = bind_native_receiver(&fixture);
        let root = fixture.temp.path();
        let peer = match detail {
            "notice-module-binding-mismatch" => {
                update_json(&dir.join("ready.json"), |ready| {
                    ready["binding"]["provider_session_id"] = json!("other-provider-conversation");
                });
                None
            }
            "notice-baseline-unprovable" => {
                let transcript =
                    root.join("profile-config/projects/project-27/provider-beta.jsonl");
                let mut bytes = fs::read(&transcript).unwrap();
                bytes.extend_from_slice(b"{");
                fs::write(transcript, bytes).unwrap();
                None
            }
            "send-session-not-idle" => {
                update_json(&root.join("sessions/beta/activity.json"), |activity| {
                    activity["state"]["phase"] = json!("working");
                });
                None
            }
            "notice-recipient-attached" => {
                fs::write(root.join("attached-client"), "").unwrap();
                Some(refuse_before_authorization_release(
                    dir, socket, binding, nonce,
                ))
            }
            "broker-not-authoritative" => {
                let heartbeat = crate::coordination::heartbeat_path(root, "beta");
                fs::write(
                    heartbeat,
                    format!("{}:1\n", fixture.candidate.target_incarnation),
                )
                .unwrap();
                Some(refuse_before_authorization_release(
                    dir, socket, binding, nonce,
                ))
            }
            _ => unreachable!(),
        };
        crate::serve::terminal_notification::dispatch_coordination_notification(
            fixture.state.clone(),
            fixture.candidate.clone(),
        )
        .await;
        if let Some(peer) = peer {
            peer.join().unwrap();
        }
        fixture.assert_no_input("");
        let receipt = notification_fixture(root);
        assert_eq!(receipt["state"], "queued", "{receipt}");
        assert_eq!(receipt["attempted_generation"], 0, "{receipt}");
        assert_eq!(receipt["last_reason"], reason, "{receipt}");
        assert_eq!(receipt["last_reason_detail"], detail, "{receipt}");
    }
}
