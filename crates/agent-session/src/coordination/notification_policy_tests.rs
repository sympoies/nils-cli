//! Destination-owned notification policy admission regressions.
use super::*;
use crate::cli::{MessageCategory, MessageSendArgs};
use nils_common::cli_contract::OutputFormat;
use pretty_assertions::assert_eq;

fn configure(context: &CliContext, recipient: &str) {
    write_atomic(
        &context
            .state_dir
            .join("coordination/notification-policy.json"),
        &serde_json::to_vec(&json!({
            "schema_version": "agent-session.notification-policy.v1",
            "recipients": [{"session_id": recipient, "suppress_categories": ["progress", "report"]}]
        }))
        .unwrap(),
        SECRET_FILE_MODE,
    )
    .unwrap();
}

#[test]
fn remote_notification_policy_stores_progress_without_a_generation() {
    let (_temp, context) = tests::fixture();
    configure(&context, "recipient");
    let mut envelope = tests::envelope();
    envelope.category = Some(MessageCategory::Progress);
    envelope.schema_version = ENVELOPE_V2.into();
    let receipt = receive(&context, "destination", envelope).unwrap();
    assert_eq!(receipt["state"], "delivered");
    assert!(
        super::super::notification::pending(&context)
            .unwrap()
            .is_empty(),
        "stored progress must not wake the configured recipient"
    );
    assert_eq!(lock_registry(&context).unwrap().registry.messages.len(), 1);
}

#[test]
fn local_notification_policy_stores_progress_without_a_generation() {
    let (_temp, context) = tests::fixture();
    configure(&context, "recipient");
    let body = context.state_dir.join("body.txt");
    std::fs::write(&body, "bounded fixture body").unwrap();
    let outcome = super::super::mailbox::send(
        &context,
        MessageSendArgs {
            from_session: "recipient".into(),
            to_session: "recipient".into(),
            to_machine: None,
            body_file: body,
            capability_file: Some(super::super::capability_path(
                &context,
                "recipient",
                "recipient-incarnation",
            )),
            idempotency_key: "local-policy-progress".into(),
            reply_to: None,
            expires_in: None,
            category: Some(MessageCategory::Progress),
            format: OutputFormat::Json,
        },
    )
    .unwrap();
    assert_eq!(outcome["state"], "unread");
    assert!(
        super::super::notification::pending(&context)
            .unwrap()
            .is_empty(),
        "stored progress must not wake the configured recipient"
    );
}

fn ingress(context: &CliContext, category: Option<MessageCategory>) -> Value {
    let mut envelope = tests::envelope();
    envelope.category = category;
    if category.is_some() {
        envelope.schema_version = ENVELOPE_V2.into();
    }
    receive(context, "destination", envelope).unwrap()
}

#[test]
fn notification_policy_category_matrix_keeps_defaults_and_urgent_mail() {
    for selected_recipient in [None, Some("other-recipient"), Some("recipient")] {
        for category in [
            None,
            Some(MessageCategory::Progress),
            Some(MessageCategory::Report),
            Some(MessageCategory::Blocker),
            Some(MessageCategory::Decision),
            Some(MessageCategory::Handoff),
        ] {
            let (_temp, context) = tests::fixture();
            if let Some(recipient) = selected_recipient {
                configure(&context, recipient);
            }
            let receipt = ingress(&context, category);
            let suppressed = selected_recipient == Some("recipient")
                && matches!(
                    category,
                    Some(MessageCategory::Progress | MessageCategory::Report)
                );
            assert_eq!(receipt["state"], "delivered");
            assert_eq!(
                receipt["notification_suppressed"]
                    .as_bool()
                    .unwrap_or(false),
                suppressed
            );
            let pending = super::super::notification::pending(&context).unwrap();
            assert_eq!(pending.is_empty(), suppressed);
            let locked = lock_registry(&context).unwrap();
            let metadata = super::super::mailbox::message_projection(&locked.registry.messages[0]);
            assert_eq!(metadata["notification_suppressed"], suppressed);
            assert_eq!(metadata["recipient_session_id"], "recipient");
            assert_eq!(
                metadata["created_at"],
                timestamp(receipt["persisted_at_epoch"].as_i64().unwrap())
            );
        }
    }
}

#[test]
fn notification_policy_suppressed_mail_remains_readable_acknowledgeable_and_replayable() {
    use crate::cli::{MessageAckArgs, MessageShowArgs};
    let (_temp, context) = tests::fixture();
    configure(&context, "recipient");
    let mut envelope = tests::envelope();
    envelope.schema_version = ENVELOPE_V2.into();
    envelope.category = Some(MessageCategory::Report);
    let receipt = receive(&context, "destination", envelope.clone()).unwrap();
    let id = receipt["message_id"].as_str().unwrap();
    let capability = super::super::capability_path(&context, "recipient", "recipient-incarnation");
    let shown = super::super::mailbox::show(
        &context,
        MessageShowArgs {
            session: "recipient".into(),
            message: id.into(),
            capability_file: Some(capability.clone()),
            format: OutputFormat::Json,
        },
    )
    .unwrap();
    assert_eq!(shown["notification_suppressed"], true);
    assert_eq!(shown["body"]["text"], envelope.body);
    assert_eq!(shown["body"]["classification"], "untrusted_peer_data");
    let ack = |revision| {
        super::super::mailbox::ack(
            &context,
            MessageAckArgs {
                session: "recipient".into(),
                message: id.into(),
                capability_file: Some(capability.clone()),
                if_revision: revision,
                idempotency_key: format!("policy-ack-{revision}"),
                format: OutputFormat::Json,
            },
        )
    };
    assert_eq!(ack(1).unwrap_err().code(), "message-revision-conflict");
    assert_eq!(ack(2).unwrap()["state"], "acknowledged");
    // Admission replay wins even if policy subsequently becomes invalid.
    std::fs::write(
        context
            .state_dir
            .join("coordination/notification-policy.json"),
        "invalid",
    )
    .unwrap();
    assert_eq!(receive(&context, "destination", envelope).unwrap(), receipt);
}

#[test]
fn notification_policy_mixed_generations_ignore_suppressed_unread_mail() {
    let (_temp, context) = tests::fixture();
    configure(&context, "recipient");
    ingress(&context, Some(MessageCategory::Progress));
    ingress(&context, Some(MessageCategory::Handoff));
    let before = super::super::notification::pending(&context)
        .unwrap()
        .pop()
        .unwrap();
    ingress(&context, Some(MessageCategory::Report));
    let after = super::super::notification::pending(&context)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(after.generation, before.generation);
    assert_eq!(after.queued_at_epoch, before.queued_at_epoch);
    let mut locked = lock_registry(&context).unwrap();
    // Future recovery must not date an urgent wake from suppressed records.
    locked.registry.messages[2].created_at_epoch = now_epoch() + 10;
    super::super::notification::normalize_registry(&mut locked.registry, now_epoch());
    assert_eq!(
        locked
            .registry
            .notifications
            .values()
            .next()
            .unwrap()
            .queued_at_epoch,
        before.queued_at_epoch
    );
    locked.registry.messages[1].state = "read".into();
    assert!(
        super::super::notification::deliverable_candidates(&mut locked.registry, now_epoch())
            .is_empty()
    );
    assert!(
        super::super::notification::claim_hook_reminder(
            &mut locked.registry,
            "recipient",
            "recipient-incarnation",
            now_epoch()
        )
        .is_none()
    );
    assert_eq!(
        locked
            .registry
            .messages
            .iter()
            .filter(|m| m.state == "unread")
            .count(),
        2
    );
}

#[test]
fn notification_policy_verified_resume_does_not_wake_for_suppressed_mail() {
    let (_temp, context) = tests::fixture();
    configure(&context, "recipient");
    ingress(&context, Some(MessageCategory::Report));
    let mut locked = lock_registry(&context).unwrap();
    let carried = super::super::mailbox::carry_unread_after_verified_resume(
        &context,
        &mut locked.registry,
        "recipient",
        "2020-01-01T00:00:00Z",
        "recipient-incarnation",
        "replacement",
        now_epoch(),
    );
    assert_eq!(carried, 1);
    assert!(locked.registry.notifications.is_empty());
    assert_eq!(
        locked.registry.messages[0].recipient_incarnation,
        "replacement"
    );
    assert!(super::super::mailbox::message_projection(&locked.registry.messages[0])["notification_suppressed"].as_bool().unwrap());
}

#[test]
fn notification_policy_rejects_invalid_or_unsafe_configuration_without_admission() {
    use std::os::unix::fs::PermissionsExt;
    for input in [
        "invalid-private-policy-canary",
        r#"{"schema_version":"unsupported","recipients":[]}"#,
        r#"{"schema_version":"agent-session.notification-policy.v1","recipients":[{"session_id":"recipient","suppress_categories":["blocker"]}]}"#,
        r#"{"schema_version":"agent-session.notification-policy.v1","recipients":[],"sender_override":true}"#,
        r#"{"schema_version":"agent-session.notification-policy.v1","recipients":[{"session_id":"recipient","suppress_categories":[]},{"session_id":"recipient","suppress_categories":[]}]}"#,
    ] {
        let (_temp, context) = tests::fixture();
        configure(&context, "recipient");
        std::fs::write(
            context
                .state_dir
                .join("coordination/notification-policy.json"),
            input,
        )
        .unwrap();
        let error = receive(&context, "destination", tests::envelope()).unwrap_err();
        assert_eq!(error.code(), "notification-policy-invalid");
        assert!(!error.message().contains("canary"));
        assert!(
            lock_registry(&context)
                .unwrap()
                .registry
                .messages
                .is_empty()
        );
    }
    let (_temp, context) = tests::fixture();
    configure(&context, "recipient");
    let path = context
        .state_dir
        .join("coordination/notification-policy.json");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        receive(&context, "destination", tests::envelope())
            .unwrap_err()
            .code(),
        "notification-policy-invalid"
    );
    std::fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(context.state_dir.join("absent"), &path).unwrap();
    assert_eq!(
        receive(&context, "destination", tests::envelope())
            .unwrap_err()
            .code(),
        "notification-policy-invalid"
    );
}

#[test]
fn notification_policy_sender_fields_cannot_suppress_unconfigured_mail() {
    let (_temp, context) = tests::fixture();
    let mut raw = serde_json::to_value(tests::envelope()).unwrap();
    raw["notification_suppressed"] = json!(true);
    raw["notification_policy"] = json!({"suppress_categories":["uncategorized"]});
    assert!(
        serde_json::from_value::<Envelope>(raw).is_err(),
        "wire policy fields are rejected"
    );
    let mut envelope = tests::envelope();
    envelope.body = r#"{"notification_policy":{"suppress_categories":["uncategorized"]}}"#.into();
    envelope.body_sha256 = digest_bytes(envelope.body.as_bytes());
    let receipt = receive(&context, "destination", envelope).unwrap();
    assert!(receipt.get("notification_suppressed").is_none());
    assert_eq!(
        super::super::notification::pending(&context).unwrap().len(),
        1
    );
}

#[test]
fn notification_policy_terminal_outbox_compaction_preserves_suppression_receipt() {
    let mut item = tests::own_outbox("suppressed-receipt", "delivered");
    item.receipt.as_mut().unwrap()["notification_suppressed"] = json!(true);
    let compacted = serde_json::to_value(item.identity()).unwrap();
    let retained: Retained = serde_json::from_value(compacted).unwrap();
    assert_eq!(retained.receipt(), item.receipt);
}

#[test]
fn notification_policy_forwarding_recomputes_destination_policy_without_elevating_authority() {
    use crate::cli::MessageForwardArgs;
    use std::os::unix::fs::PermissionsExt;
    let (_temp, context) = tests::fixture();
    // The original recipient is unconfigured, while the forwarded copy is quiet.
    configure(&context, "digest");
    let original = ingress(&context, Some(MessageCategory::Progress));
    let directory = context.state_dir.join("sessions/digest/coordination");
    std::fs::create_dir_all(&directory).unwrap();
    for path in [context.state_dir.join("sessions/digest"), directory.clone()] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut record: Value = serde_json::from_slice(
        &std::fs::read(context.state_dir.join("sessions/recipient/session.json")).unwrap(),
    )
    .unwrap();
    record["id"] = json!("digest");
    record["runtime"]["launch_id"] = json!("digest-incarnation");
    write_atomic(
        &context.state_dir.join("sessions/digest/session.json"),
        &serde_json::to_vec(&record).unwrap(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    let source_capability =
        super::super::capability_path(&context, "recipient", "recipient-incarnation");
    write_atomic(
        &super::super::capability_path(&context, "digest", "digest-incarnation"),
        &std::fs::read(&source_capability).unwrap(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    write_atomic(
        &directory.join("heartbeat"),
        format!("digest-incarnation:{}\n", now_epoch()).as_bytes(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    let mut locked = lock_registry(&context).unwrap();
    let mut broker = locked.registry.brokers["recipient"].clone();
    broker.session_id = "digest".into();
    broker.incarnation = "digest-incarnation".into();
    locked.registry.brokers.insert("digest".into(), broker);
    locked.save().unwrap();
    drop(locked);
    let forwarded = super::super::mailbox::forward(
        &context,
        MessageForwardArgs {
            session: "recipient".into(),
            message: original["message_id"].as_str().unwrap().into(),
            if_revision: 1,
            to_session: "digest".into(),
            to_machine: None,
            categories: vec![MessageCategory::Progress],
            capability_file: Some(source_capability),
            idempotency_key: "policy-forward".into(),
            format: OutputFormat::Json,
        },
    )
    .unwrap();
    assert_eq!(forwarded["notification_suppressed"], true);
    assert_eq!(forwarded["category"], "progress");
    assert_eq!(
        forwarded["forwarding"]["original_message_id"],
        original["message_id"]
    );
    let locked = lock_registry(&context).unwrap();
    assert_eq!(locked.registry.messages[0].state, "unread");
    assert_eq!(locked.registry.messages[0].revision, 1);
    assert_eq!(
        super::super::mailbox::message_projection(&locked.registry.messages[1])["forwarding"],
        forwarded["forwarding"]
    );
    assert_eq!(
        locked.registry.notifications.len(),
        1,
        "only original recipient has a wake"
    );
}

#[test]
fn notification_policy_journal_fences_suppression_and_keeps_the_fence() {
    let (_temp, context) = tests::fixture();
    let mut item = tests::own_outbox("quiet-fence", "delivered");
    item.receipt.as_mut().unwrap()["notification_suppressed"] = json!(true);
    let mut locked = lock_journal(&context).unwrap();
    locked.registry.remote_outbox.push(item);
    locked.save().unwrap();
    assert_eq!(
        locked.registry.schema_version,
        "agent-session.federation-journal.v6"
    );
    drop(locked);
    let mut locked = lock_journal(&context).unwrap();
    assert_eq!(
        locked.registry.retained[0].receipt().unwrap()["notification_suppressed"],
        true
    );
    locked.registry.retained.clear();
    locked.save().unwrap();
    assert_eq!(
        locked.registry.schema_version,
        "agent-session.federation-journal.v6"
    );
}

#[test]
fn notification_policy_registry_fences_old_writers_and_keeps_the_fence() {
    let (_temp, context) = tests::fixture();
    configure(&context, "recipient");
    ingress(&context, Some(MessageCategory::Progress));
    let mut locked = lock_registry(&context).unwrap();
    assert_eq!(
        locked.registry.schema_version,
        "agent-session.coordination-registry.v3"
    );
    assert!(locked.registry.messages[0].notification_suppressed);
    locked.registry.messages.clear();
    locked.save().unwrap();
    drop(locked);
    assert_eq!(
        lock_registry(&context).unwrap().registry.schema_version,
        "agent-session.coordination-registry.v3"
    );
    assert!(super::super::public_summary_registry_snapshot(&context).is_some());
    assert!(
        nils_common::coordination_projection::load(&context.state_dir)
            .unwrap()
            .is_some()
    );
}

#[test]
fn notification_policy_broker_inspection_resolves_wake_but_counts_quiet_mail() {
    let (_temp, context) = tests::fixture();
    configure(&context, "recipient");
    ingress(&context, Some(MessageCategory::Progress));
    ingress(&context, Some(MessageCategory::Handoff));
    let mut locked = lock_registry(&context).unwrap();
    locked.registry.messages[1].state = "read".into();
    locked.save().unwrap();
    drop(locked);
    let status = super::super::broker::status(
        &context,
        crate::cli::BrokerStatusArgs {
            session: "recipient".into(),
            capability_file: None,
            authenticated: false,
            format: OutputFormat::Json,
        },
    )
    .unwrap();
    assert_eq!(status["notification"]["live_unread_count"], 1);
    assert_eq!(status["notification"]["delivery_state"], "resolved");
    assert_eq!(status["notification"]["resolved"], true);
}

#[test]
fn notification_policy_remote_refusal_is_terminal_without_erasing_uncertainty() {
    use std::io::{Read, Write};
    for (attempts, expected) in [(0, "rejected"), (1, "delivery-unknown")] {
        let (_temp, context) = tests::fixture();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut config = tests::config();
        config.url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            loop {
                let mut bytes = [0; 1024];
                let count = stream.read(&mut bytes).unwrap();
                assert!(count > 0, "request must finish before EOF");
                request.extend_from_slice(&bytes[..count]);
                assert!(request.len() <= 8192, "bounded fixture request");
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    let header = std::str::from_utf8(&request[..end]).unwrap();
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = r#"{"error":{"code":"notification-policy-invalid","message":"private-policy-canary","details":{"path":"private-policy-canary"}}}"#;
            write!(
                stream,
                "HTTP/1.1 409 Conflict\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let mut item = tests::own_outbox("invalid-policy", "queued");
        item.attempts = attempts;
        let mut locked = lock_journal(&context).unwrap();
        locked.registry.remote_outbox.push(item);
        locked.save().unwrap();
        drop(locked);
        drain(&context, &config).unwrap();
        server.join().unwrap();
        let locked = lock_journal(&context).unwrap();
        assert!(
            locked.registry.remote_outbox.is_empty(),
            "policy refusal must stop retries"
        );
        let retained = &locked.registry.retained[0];
        assert_eq!(retained.state, expected);
        assert_eq!(
            retained.reason.as_deref(),
            Some("notification-policy-invalid")
        );
        assert!(!projection(retained).to_string().contains("canary"));
    }
}

#[test]
fn routine_coalescing_is_opt_in_and_urgent_mail_bypasses_the_window() {
    let (_temp, context) = tests::fixture();
    write_atomic(
        &context.state_dir.join("coordination/notification-policy.json"),
        &serde_json::to_vec(&json!({
            "schema_version": "agent-session.notification-policy.v1",
            "recipients": [{"session_id":"recipient", "suppress_categories":[], "routine_interval_seconds":600}]
        })).unwrap(), SECRET_FILE_MODE,
    ).unwrap();
    ingress(&context, Some(MessageCategory::Progress));
    let first = super::super::notification::pending(&context)
        .unwrap()
        .pop()
        .unwrap();
    assert!(super::super::notification::begin_attempt(&context, &first).unwrap());
    assert!(super::super::notification::mark_submitted(&context, &first).unwrap());
    ingress(&context, Some(MessageCategory::Handoff));
    assert!(
        super::super::notification::pending(&context)
            .unwrap()
            .is_empty()
    );
    ingress(&context, Some(MessageCategory::Blocker));
    let urgent = super::super::notification::pending(&context)
        .unwrap()
        .pop()
        .unwrap();
    let prompt = super::super::notification_prompt(&urgent);
    assert!(prompt.contains("p=1") && prompt.contains("h=1") && prompt.contains("b=1"));
    assert!(!prompt.contains("bounded fixture body"));
}

#[test]
fn coalescing_window_survives_restart_and_hook_claim_until_ten_minutes() {
    let (_temp, context) = tests::fixture();
    write_atomic(&context.state_dir.join("coordination/notification-policy.json"), &serde_json::to_vec(&json!({
        "schema_version":"agent-session.notification-policy.v1", "recipients":[{"session_id":"recipient", "suppress_categories":[], "routine_interval_seconds":600}]
    })).unwrap(), SECRET_FILE_MODE).unwrap();
    ingress(&context, Some(MessageCategory::Report));
    let first = super::super::notification::pending(&context)
        .unwrap()
        .pop()
        .unwrap();
    assert!(super::super::notification::begin_attempt(&context, &first).unwrap());
    assert!(super::super::notification::mark_submitted(&context, &first).unwrap());
    ingress(&context, None);
    let mut locked = lock_registry(&context).unwrap();
    for message in &mut locked.registry.messages {
        message.expires_at_epoch += 3600;
    }
    locked.save().unwrap();
    drop(locked);
    let mut registry = lock_registry(&context).unwrap().registry.clone();
    let receipt = registry.notifications.values().next().unwrap();
    let at = receipt.routine_last_submitted_at_epoch;
    assert!(
        super::super::notification::claim_hook_reminder(
            &mut registry,
            "recipient",
            "recipient-incarnation",
            at + 599
        )
        .is_none()
    );
    let next = super::super::notification::claim_hook_reminder(
        &mut registry,
        "recipient",
        "recipient-incarnation",
        at + 600,
    )
    .unwrap();
    assert!(super::super::notification_prompt(&next).contains("u=1"));
}
