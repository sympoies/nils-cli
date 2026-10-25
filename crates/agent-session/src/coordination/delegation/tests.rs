use super::cli::{Mint, Operation, Revoke};
use super::*;
use nils_common::{
    cli_contract::OutputFormat,
    fs::{SECRET_FILE_MODE, write_atomic},
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture {
    _temp: tempfile::TempDir,
    context: CliContext,
    capability: PathBuf,
}
impl Fixture {
    fn new(agent: &str) -> Self {
        let temp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: temp.path().to_owned(),
            host: Some("fixture-machine".into()),
        };
        let mut registry = super::super::lock_registry(&context).unwrap();
        super::super::ensure_fingerprint_key(&mut registry.registry);
        for id in ["caller", "recipient", "a-recipient"] {
            let dir = context
                .state_dir
                .join("sessions")
                .join(id)
                .join("coordination");
            fs::create_dir_all(&dir).unwrap();
            for path in [
                context.state_dir.join("sessions"),
                dir.parent().unwrap().to_owned(),
                dir.clone(),
            ] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            let record = json!({"schema_version":"agent-session.session.v1", "id":id, "agent":agent, "mode":"interactive", "title":null, "cwd":".", "tmux_session":"fixture", "prompt_file":null, "log_file":null, "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z", "runtime":{"kind":"tmux", "tmux_session":"fixture", "generation":1, "started_at":"2030-01-01T00:00:00Z", "launch_id":"current"}});
            write_atomic(
                &dir.parent().unwrap().join("session.json"),
                &serde_json::to_vec(&record).unwrap(),
                SECRET_FILE_MODE,
            )
            .unwrap();
            write_atomic(
                &super::super::capability_path(&context, id, "current"),
                b"synthetic-runtime-capability-000000000000001",
                SECRET_FILE_MODE,
            )
            .unwrap();
            write_atomic(
                &dir.join("heartbeat"),
                format!("current:{}\n", now_epoch()).as_bytes(),
                SECRET_FILE_MODE,
            )
            .unwrap();
            registry.registry.brokers.insert(id.into(), serde_json::from_value(json!({"session_id":id, "incarnation":"current", "capability_digest":digest_bytes(b"synthetic-runtime-capability-000000000000001"), "generation":1, "state":"ready", "heartbeat_at":"2030-01-01T00:00:00Z", "heartbeat_epoch":now_epoch()})).unwrap());
        }
        registry.save().unwrap();
        drop(registry);
        let capability = super::super::capability_path(&context, "caller", "current");
        Self {
            _temp: temp,
            context,
            capability,
        }
    }
    fn mint(
        &self,
        operations: Vec<Operation>,
        recipients: Vec<String>,
    ) -> (Value, String, PathBuf) {
        let result = cli::mint(
            &self.context,
            Mint {
                session: "caller".into(),
                capability_file: Some(self.capability.clone()),
                operations,
                recipients,
                expires_in: 3600,
                format: OutputFormat::Json,
            },
        )
        .unwrap();
        let path = PathBuf::from(result["credential_file"].as_str().unwrap());
        let token = fs::read_to_string(&path).unwrap();
        (result, token, path)
    }
    fn machine(&self) -> String {
        crate::board::machine_identity(None, &self.context)
    }
}
fn allowed(
    context: &CliContext,
    token: &str,
    usage: Use<'_>,
) -> Result<(SessionRecord, String), CliError> {
    authenticate_token(context, Some("caller"), token, usage, true)
}

#[test]
fn delegation_scope_and_use_audit_work_for_supported_harnesses() {
    for agent in ["codex", "claude"] {
        let fixture = Fixture::new(agent);
        let (result, token, path) = fixture.mint(
            vec![Operation::MessageSend],
            vec![format!("{}/recipient", fixture.machine())],
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let body = fixture.context.state_dir.join("body.txt");
        fs::write(&body, "synthetic worker receipt").unwrap();
        let send = |recipient: &str, key: &str| {
            super::super::mailbox::send(
                &fixture.context,
                crate::cli::MessageSendArgs {
                    from_session: "caller".into(),
                    to_session: recipient.into(),
                    to_machine: None,
                    body_file: body.clone(),
                    reply_to: None,
                    expires_in: None,
                    capability_file: Some(path.clone()),
                    idempotency_key: key.into(),
                    format: OutputFormat::Json,
                    category: None,
                },
            )
        };
        send("recipient", "allowed-receipt").expect("in-scope receipt must be delivered");
        {
            let mut registry = super::super::lock_registry(&fixture.context).unwrap();
            registry.registry.messages[0].notification_suppressed = true;
            registry.save().unwrap();
            assert_eq!(
                registry.registry.schema_version,
                "agent-session.coordination-registry.v4"
            );
        }

        assert_eq!(
            send("caller", "denied").unwrap_err().code(),
            "delegation-scope-denied"
        );
        assert_eq!(
            allowed(&fixture.context, &token, Use::StartViaConsole)
                .unwrap_err()
                .code(),
            "delegation-scope-denied"
        );
        let wrong_machine = Use::MessageSend {
            machine: "other-machine",
            recipient: "recipient",
        };
        assert_eq!(
            allowed(&fixture.context, &token, wrong_machine)
                .unwrap_err()
                .code(),
            "delegation-scope-denied"
        );
        assert_eq!(
            super::super::authenticate_token(&fixture.context, "caller", &token)
                .unwrap_err()
                .code(),
            "delegation-scope-denied"
        );
        let registry = super::super::lock_registry_observational(&fixture.context).unwrap();
        let audit = &registry.registry.delegation_audit;
        assert_eq!(audit.len(), 5);
        assert_eq!(audit[0].outcome, "allowed");
        assert_eq!(audit[1].outcome, "delegation-scope-denied");
        assert_eq!(audit[0].delegation_id, result["id"].as_str().unwrap());
        let stored = serde_json::to_string(&registry.registry).unwrap();
        assert!(!stored.contains(&token));
        assert!(!stored.contains("synthetic-runtime-capability-000000000000001"));
        assert!(
            !serde_json::to_string(audit)
                .unwrap()
                .contains("synthetic worker receipt")
        );
    }
}

#[test]
fn expired_revoked_and_replaced_incarnations_have_typed_refusals() {
    for change in [
        "expiry",
        "explicit",
        "stop",
        "replacement",
        "credential-rotation",
        "stale-heartbeat",
    ] {
        let fixture = Fixture::new("codex");
        let (result, token, _) = fixture.mint(vec![Operation::StartViaConsole], vec![]);
        assert!(allowed(&fixture.context, &token, Use::StartViaConsole).is_ok());
        let id = result["id"].as_str().unwrap();
        let expected = match change {
            "explicit" => {
                cli::revoke(
                    &fixture.context,
                    Revoke {
                        session: "caller".into(),
                        id: id.into(),
                        capability_file: Some(fixture.capability.clone()),
                        format: OutputFormat::Json,
                    },
                )
                .unwrap();
                "delegation-revoked"
            }
            "replacement" => {
                let path = fixture
                    .context
                    .state_dir
                    .join("sessions/caller/session.json");
                let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                record["runtime"]["launch_id"] = json!("replacement");
                write_atomic(
                    &path,
                    &serde_json::to_vec(&record).unwrap(),
                    SECRET_FILE_MODE,
                )
                .unwrap();
                "delegation-incarnation-replaced"
            }
            "credential-rotation" => {
                write_atomic(
                    &fixture.capability,
                    b"rotated-runtime-capability-000000000000001",
                    SECRET_FILE_MODE,
                )
                .unwrap();
                "delegation-revoked"
            }
            "stale-heartbeat" => {
                write_atomic(
                    &fixture
                        .context
                        .state_dir
                        .join("sessions/caller/coordination/heartbeat"),
                    b"current:0\n",
                    SECRET_FILE_MODE,
                )
                .unwrap();
                "delegation-revoked"
            }
            _ => {
                let mut registry = super::super::lock_registry(&fixture.context).unwrap();
                if change == "expiry" {
                    registry
                        .registry
                        .delegations
                        .get_mut(id)
                        .unwrap()
                        .expires_at_epoch = now_epoch();
                } else {
                    registry.registry.brokers.get_mut("caller").unwrap().state = "revoked".into();
                }
                registry.save().unwrap();
                if change == "expiry" {
                    "delegation-expired"
                } else {
                    "delegation-revoked"
                }
            }
        };
        assert_eq!(
            allowed(&fixture.context, &token, Use::StartViaConsole)
                .unwrap_err()
                .code(),
            expected,
            "{change}"
        );
        let registry = super::super::lock_registry_observational(&fixture.context).unwrap();
        assert_eq!(
            registry.registry.delegation_audit.last().unwrap().outcome,
            expected
        );
        assert_eq!(
            revalidate(
                &fixture.context,
                &registry.registry,
                "caller",
                "current",
                &token,
                Use::StartViaConsole
            )
            .unwrap_err()
            .code(),
            expected
        );
    }
}

#[test]
fn scoped_console_starts_relay_without_exposing_the_worker_credential() {
    use nils_test_support::http::{HttpResponse, LoopbackServer};
    for agent in ["codex", "claude"] {
        let fixture = Fixture::new(agent);
        let (_, token, _) = fixture.mint(vec![Operation::StartViaConsole], vec![]);
        let server = LoopbackServer::new().unwrap();
        server.add_route(
            "POST",
            "/api/coordination/sessions/v1",
            HttpResponse::new(
                201,
                json!({"ok":true,"data":{"session":{"id":"child"}}}).to_string(),
            ),
        );
        let config = super::super::remote::Config {
            machine: "fixture-machine".into(),
            url: server.url(),
            token: "synthetic-relay-token".into(),
            ingress_token: "synthetic-ingress-token".into(),
        };
        let request = json!({"session":{"agent":agent,"cwd":"."}});
        let result = super::super::console_start::relay_route(
            &fixture.context,
            Some(&config),
            "caller",
            &token,
            &request,
        )
        .unwrap();
        assert_eq!(result["session"]["id"], "child");
        let requests = server.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].header_value("authorization").as_deref(),
            Some("Bearer synthetic-relay-token")
        );
        assert!(!requests[0].body_text().contains(&token));
        assert_eq!(
            super::super::console_start::relay_route(
                &fixture.context,
                Some(&config),
                "recipient",
                &token,
                &request
            )
            .unwrap_err()
            .code(),
            "delegation-scope-denied"
        );
    }
}

#[test]
fn forged_credentials_and_unscoped_minting_are_refused() {
    let fixture = Fixture::new("codex");
    let (_, token, path) = fixture.mint(vec![Operation::StartViaConsole], vec![]);
    assert_eq!(
        allowed(
            &fixture.context,
            &(token + "tampered"),
            Use::StartViaConsole
        )
        .unwrap_err()
        .code(),
        "delegation-unauthorized"
    );
    assert_eq!(
        cli::mint(
            &fixture.context,
            Mint {
                session: "caller".into(),
                capability_file: Some(path),
                operations: vec![Operation::StartViaConsole],
                recipients: vec![],
                expires_in: 3600,
                format: OutputFormat::Json
            }
        )
        .unwrap_err()
        .code(),
        "delegation-scope-denied"
    );
    assert_eq!(
        cli::mint(
            &fixture.context,
            Mint {
                session: "caller".into(),
                capability_file: Some(fixture.capability.clone()),
                operations: vec![Operation::MessageSend],
                recipients: vec![],
                expires_in: 3600,
                format: OutputFormat::Json
            }
        )
        .unwrap_err()
        .code(),
        "delegation-invalid"
    );
}

#[test]
fn local_receipt_rechecks_expiry_after_preflight_before_commit() {
    use std::{
        thread,
        time::{Duration, Instant},
    };
    let fixture = Fixture::new("codex");
    let (result, _, path) = fixture.mint(
        vec![Operation::MessageSend],
        vec![format!("{}/a-recipient", fixture.machine())],
    );
    let body = fixture.context.state_dir.join("receipt.txt");
    fs::write(&body, "worker receipt").unwrap();
    // Keep the final sorted lifecycle-lock acquisition behind a-recipient.
    let recipient_guard =
        crate::acquire_session_record_lock(&fixture.context, "a-recipient").unwrap();
    let caller_guard = crate::acquire_session_record_lock(&fixture.context, "caller").unwrap();
    let context = fixture.context.clone();
    let sender = thread::spawn(move || {
        super::super::mailbox::send(
            &context,
            crate::cli::MessageSendArgs {
                from_session: "caller".into(),
                to_session: "a-recipient".into(),
                to_machine: None,
                body_file: body,
                reply_to: None,
                expires_in: None,
                capability_file: Some(path),
                idempotency_key: "race-receipt".into(),
                format: OutputFormat::Json,
                category: None,
            },
        )
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let registry = super::super::lock_registry(&fixture.context).unwrap();
        if !registry.registry.delegation_audit.is_empty() {
            break;
        }
        drop(registry);
        assert!(
            Instant::now() < deadline,
            "sender never reached authentication"
        );
        thread::sleep(Duration::from_millis(1));
    }
    // Hold the registry while releasing caller: the sender must take caller
    // for preflight and then wait on this registry lock. Observe that lock
    // before letting preflight finish, so a fast lock cycle cannot be missed.
    let registry = super::super::lock_registry(&fixture.context).unwrap();
    drop(caller_guard);
    loop {
        if crate::try_acquire_session_record_lock(&fixture.context, "caller")
            .unwrap()
            .is_none()
        {
            break;
        }
        assert!(Instant::now() < deadline, "sender never entered preflight");
        thread::sleep(Duration::from_millis(1));
    }
    drop(registry);
    let caller_guard = loop {
        if let Some(guard) =
            crate::try_acquire_session_record_lock(&fixture.context, "caller").unwrap()
        {
            break guard;
        }
        assert!(Instant::now() < deadline, "sender never finished preflight");
        thread::sleep(Duration::from_millis(1));
    };
    // Preflight released caller; final commit is still behind a-recipient.
    let mut registry = super::super::lock_registry(&fixture.context).unwrap();
    registry
        .registry
        .delegations
        .get_mut(result["id"].as_str().unwrap())
        .unwrap()
        .expires_at_epoch = now_epoch();
    registry.save().unwrap();
    drop(registry);
    drop(caller_guard);
    drop(recipient_guard);
    assert_eq!(
        sender.join().unwrap().unwrap_err().code(),
        "delegation-expired"
    );
    let registry = super::super::lock_registry_observational(&fixture.context).unwrap();
    assert!(registry.registry.messages.is_empty());
    assert!(registry.registry.receipts.is_empty());
}

#[test]
fn remote_receipt_rechecks_revocation_after_discovery_before_persistence() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };
    let fixture = Fixture::new("claude");
    let (result, token, _) = fixture.mint(
        vec![Operation::MessageSend],
        vec!["remote-machine/recipient".into()],
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        ready_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let body = json!({"schema_version":"agent-session.remote-peers.v1", "peers":[{"machine":"remote-machine", "session_id":"recipient", "session_incarnation":"target-current", "messaging_supported":true}]}).to_string();
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
    });
    let context = fixture.context.clone();
    let sender = thread::spawn(move || {
        let config = super::super::remote::Config {
            machine: "fixture-machine".into(),
            url: format!("http://{address}"),
            token: "synthetic-relay-token".into(),
            ingress_token: "synthetic-ingress-token".into(),
        };
        let submission = serde_json::from_value(json!({"to_machine":"remote-machine", "to_session":"recipient", "body":"worker receipt", "idempotency_key":"race-receipt"})).unwrap();
        super::super::remote::submit(&context, &config, "caller", &token, submission)
    });
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    cli::revoke(
        &fixture.context,
        Revoke {
            session: "caller".into(),
            id: result["id"].as_str().unwrap().into(),
            capability_file: Some(fixture.capability.clone()),
            format: OutputFormat::Json,
        },
    )
    .unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(
        sender.join().unwrap().unwrap_err().code(),
        "delegation-revoked"
    );
    server.join().unwrap();
    let path = fixture
        .context
        .state_dir
        .join("coordination/federation-journal.json");
    if path.exists() {
        let journal: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert!(journal["remote_outbox"].as_array().unwrap().is_empty());
    }
}

#[test]
fn local_message_scope_cannot_relabel_a_remote_destination() {
    let fixture = Fixture::new("codex");
    let (_, token, _) = fixture.mint(
        vec![Operation::MessageSend],
        vec!["remote-machine/recipient".into()],
    );
    // A listed remote address is valid for remote submission, but even an
    // identical caller-supplied machine label cannot turn it into local mail.
    assert!(
        allowed(
            &fixture.context,
            &token,
            Use::MessageSend {
                machine: "remote-machine",
                recipient: "recipient"
            }
        )
        .is_ok()
    );
    assert_eq!(
        allowed(
            &fixture.context,
            &token,
            Use::LocalMessageSend {
                machine: "remote-machine",
                recipient: "recipient"
            }
        )
        .unwrap_err()
        .code(),
        "delegation-scope-denied"
    );
}

#[test]
fn delegation_registry_fences_older_writers_and_preserves_its_version() {
    let fixture = Fixture::new("codex");
    fixture.mint(vec![Operation::StartViaConsole], vec![]);
    let path = fixture.context.state_dir.join("coordination/registry.json");
    let bytes = fs::read(&path).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["schema_version"],
        "agent-session.coordination-registry.v4"
    );
    // The pre-delegation writer's save admission recognizes only v1-v3.
    let older_writer_admits = matches!(
        value["schema_version"].as_str().unwrap(),
        "agent-session.coordination-registry.v1"
            | "agent-session.coordination-registry.v2"
            | "agent-session.coordination-registry.v3"
    );
    assert!(
        !older_writer_admits,
        "an older writer must refuse before dropping fields"
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(nils_common::coordination_projection::load(&fixture.context.state_dir).is_ok());
    let audit = super::super::audit::snapshot(
        &fixture.context,
        &fixture.machine(),
        super::super::audit::Query::default(),
    )
    .unwrap();
    assert_eq!(audit["sources"]["inbox"], "available");
    assert_eq!(audit["sources"]["notifications"], "available");

    let mut registry = super::super::lock_registry(&fixture.context).unwrap();
    registry.registry.delegations.clear();
    registry.registry.delegation_audit.clear();
    registry.save().unwrap();
    drop(registry);
    assert_eq!(
        super::super::lock_registry(&fixture.context)
            .unwrap()
            .registry
            .schema_version,
        "agent-session.coordination-registry.v4"
    );
}

#[test]
fn notification_history_registry_fences_older_writers_and_preserves_its_version() {
    let fixture = Fixture::new("codex");
    let mut registry = super::super::lock_registry(&fixture.context).unwrap();
    let receipt = super::super::notification::NotificationReceipt {
        schema_version: "agent-session.notification-generation.v1".into(),
        target_session_id: "recipient".into(),
        target_incarnation: "current".into(),
        generation: 1,
        attempt_history: vec![super::super::notification::NotificationAttempt {
            generation: 1,
            recorded_at_epoch: super::super::now_epoch(),
            reason: "recipient-attached".into(),
        }],
        ..super::super::notification::NotificationReceipt::default()
    };
    registry
        .registry
        .notifications
        .insert("synthetic-notification".into(), receipt);
    registry.save().unwrap();

    let path = fixture.context.state_dir.join("coordination/registry.json");
    let bytes = fs::read(&path).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    let version = value["schema_version"].as_str().unwrap();
    assert_eq!(version, "agent-session.coordination-registry.v5");
    let older_writer_admits = matches!(
        version,
        "agent-session.coordination-registry.v1"
            | "agent-session.coordination-registry.v2"
            | "agent-session.coordination-registry.v3"
            | "agent-session.coordination-registry.v4"
    );
    assert!(
        !older_writer_admits,
        "a v1-v4 writer must refuse before dropping notification history"
    );
    assert!(nils_common::coordination_projection::load(&fixture.context.state_dir).is_ok());
    assert_eq!(fs::read(&path).unwrap(), bytes);

    registry
        .registry
        .notifications
        .values_mut()
        .next()
        .unwrap()
        .attempt_history
        .clear();
    registry.save().unwrap();
    drop(registry);
    assert_eq!(
        super::super::lock_registry(&fixture.context)
            .unwrap()
            .registry
            .schema_version,
        "agent-session.coordination-registry.v5"
    );
}
