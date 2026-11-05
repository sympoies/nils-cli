use super::*;
use pretty_assertions::{assert_eq, assert_ne};

#[test]
fn message_reminder_claims_each_live_unread_generation_once() {
    // DSH sessions have no serve prompt route; their policy hook claims the
    // fixed reminder through this command at a safe model-step boundary
    // (sympoies/nils-cli#2011).
    let lock = nils_test_support::GlobalStateLock::new();
    let _writers = nils_test_support::EnvGuard::set(&lock, "AGENT_SESSION_NOTICE_WRITERS", "1");
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    // Exercise the hook-only recipient, whose plain route is explicit.
    let path = state_dir.join("sessions/beta/session.json");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["agent"] = json!("dsh");
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    let body = tmp.path().join("body.txt");
    fs::write(&body, "REMINDER-BODY-CANARY").expect("body");
    let state = state_dir.to_string_lossy().to_string();
    let alpha_cap = capability(&state_dir, "alpha");
    let beta_cap = capability(&state_dir, "beta");
    let send = |key: &str| {
        let sent = run(
            tmp.path(),
            &[
                "--state-dir",
                &state,
                "message",
                "send",
                "--from",
                "alpha",
                "--to",
                "beta",
                "--body-file",
                body.to_str().expect("body"),
                "--capability-file",
                &alpha_cap,
                "--idempotency-key",
                key,
                "--format",
                "json",
            ],
        );
        assert_eq!(sent.code, 0, "stderr={}", sent.stderr_text());
    };
    let reminder = |session: &str, capability: &str| {
        run(
            tmp.path(),
            &[
                "--state-dir",
                &state,
                "message",
                "reminder",
                "--session",
                session,
                "--capability-file",
                capability,
                "--format",
                "json",
            ],
        )
    };

    // Nothing is pending before any mail arrives.
    let empty = reminder("beta", &beta_cap);
    assert_eq!(
        empty.code,
        0,
        "stdout={} stderr={}",
        empty.stdout_text(),
        empty.stderr_text()
    );
    assert_eq!(
        empty.stdout_json()["schema_version"],
        "cli.agent-session.message-reminder.v1"
    );
    assert_eq!(data(&empty)["reminder"], serde_json::Value::Null);

    send("reminder-send-0001");
    let first = reminder("beta", &beta_cap);
    assert_eq!(first.code, 0, "stderr={}", first.stderr_text());
    let text = data(&first)["reminder"]
        .as_str()
        .expect("reminder text")
        .to_string();
    assert!(
        text.starts_with(
            "<!-- agent-notice:v1 kind=mailbox.wake source=agent-session -->\n\nMailbox: "
        ),
        "{text}"
    );
    let body = text.split_once("\n\n").unwrap().1;
    assert!(
        body.lines()
            .next()
            .unwrap()
            .ends_with("agent-session message inbox")
    );
    assert!(text.is_ascii());
    assert!(!text.contains("beta"));
    assert!(!first.stdout_text().contains("REMINDER-BODY-CANARY"));
    assert_eq!(data(&first)["generation"], 1);
    let receipt = coordination_registry(&state_dir)["notifications"]
        .as_object()
        .expect("notifications")
        .values()
        .find(|receipt| receipt["target_session_id"] == "beta")
        .expect("beta receipt")
        .clone();
    assert_eq!(receipt["state"], "prompt_submitted");
    assert_eq!(receipt["notified_generation"], 1);

    // One generation is delivered once.
    let repeat = reminder("beta", &beta_cap);
    assert_eq!(repeat.code, 0, "stderr={}", repeat.stderr_text());
    assert_eq!(data(&repeat)["reminder"], serde_json::Value::Null);

    // A later send advances the generation, including after serve recorded
    // that the hook owns delivery.
    rewrite_registry(&state_dir, |registry| {
        for receipt in registry["notifications"]
            .as_object_mut()
            .expect("notifications")
            .values_mut()
        {
            receipt["state"] = json!("undeliverable");
            receipt["last_reason"] = json!("hook-delivered");
        }
    });
    send("reminder-send-0002");
    rewrite_registry(&state_dir, |registry| {
        for receipt in registry["notifications"]
            .as_object_mut()
            .expect("notifications")
            .values_mut()
        {
            receipt["state"] = json!("undeliverable");
            receipt["last_reason"] = json!("hook-delivered");
        }
    });
    let second = reminder("beta", &beta_cap);
    assert_eq!(second.code, 0, "stderr={}", second.stderr_text());
    assert_eq!(data(&second)["generation"], 2);
    assert!(data(&second)["reminder"].is_string());

    // A drained inbox has nothing to announce, even for a new generation.
    send("reminder-send-0003");
    rewrite_registry(&state_dir, |registry| {
        for message in registry["messages"].as_array_mut().expect("messages") {
            message["state"] = json!("acknowledged");
        }
    });
    let drained = reminder("beta", &beta_cap);
    assert_eq!(drained.code, 0, "stderr={}", drained.stderr_text());
    assert_eq!(data(&drained)["reminder"], serde_json::Value::Null);

    // Only the authenticated recipient may claim its reminder.
    send("reminder-send-0004");
    let impersonation = reminder("beta", &alpha_cap);
    assert_ne!(impersonation.code, 0);
    assert_eq!(
        impersonation.stdout_json()["error"]["code"],
        "coordination-unauthorized"
    );
    let claimed = reminder("beta", &beta_cap);
    assert!(data(&claimed)["reminder"].is_string());
}

#[test]
fn message_reminder_gives_up_under_lock_contention_without_claiming() {
    let lock = nils_test_support::GlobalStateLock::new();
    let _writers = nils_test_support::EnvGuard::set(&lock, "AGENT_SESSION_NOTICE_WRITERS", "1");
    // The hook that runs `message reminder` has a 5 s child deadline. The
    // claim must finish well inside it or give up unclaimed, so a reminder is
    // never persisted as delivered by a child the hook already abandoned.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let path = state_dir.join("sessions/beta/session.json");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["agent"] = json!("dsh");
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    let body = tmp.path().join("body.txt");
    fs::write(&body, "contended reminder body").expect("body");
    let state = state_dir.to_string_lossy().to_string();
    let alpha_cap = capability(&state_dir, "alpha");
    let beta_cap = capability(&state_dir, "beta");
    let sent = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "message",
            "send",
            "--from",
            "alpha",
            "--to",
            "beta",
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "contended-reminder-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(sent.code, 0, "stderr={}", sent.stderr_text());
    let reminder = || {
        run(
            tmp.path(),
            &[
                "--state-dir",
                &state,
                "message",
                "reminder",
                "--session",
                "beta",
                "--capability-file",
                &beta_cap,
                "--format",
                "json",
            ],
        )
    };

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(state_dir.join("coordination/registry.lock"))
        .expect("registry lock");
    lock.lock().expect("hold the registry lock");
    let started = Instant::now();
    let contended = reminder();
    let elapsed = started.elapsed();
    lock.unlock().expect("release the registry lock");
    assert_ne!(contended.code, 0, "stdout={}", contended.stdout_text());
    assert_eq!(
        contended.stdout_json()["error"]["code"],
        "coordination-lock-timeout"
    );
    assert!(
        elapsed < Duration::from_millis(1_800),
        "the reminder claim must give up well inside the hook deadline: {elapsed:?}"
    );
    let receipt = coordination_registry(&state_dir)["notifications"]
        .as_object()
        .expect("notifications")
        .values()
        .find(|receipt| receipt["target_session_id"] == "beta")
        .expect("beta receipt")
        .clone();
    assert_eq!(receipt["notified_generation"], 0);

    // The generation stays claimable once the lock is free.
    let claimed = reminder();
    assert_eq!(claimed.code, 0, "stderr={}", claimed.stderr_text());
    assert_eq!(data(&claimed)["generation"], 1);
    assert!(data(&claimed)["reminder"].is_string());
}
