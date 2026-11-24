use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::io::Write;

const READ_BUDGET: u64 = 128 * 1024;

fn read_bytes() -> u64 {
    fs::read_to_string("/proc/thread-self/io")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("rchar: "))
        .unwrap()
        .parse()
        .unwrap()
}

fn context(temp: &tempfile::TempDir) -> CliContext {
    CliContext {
        state_dir: temp.path().join("state"),
        host: None,
    }
}

#[test]
fn registry_poll_reads_are_bounded_across_snapshot_and_locked_readers() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(&temp);
    let mut locked = lock_registry_observational(&context).unwrap();
    locked.registry.fingerprint_key = "fixture-before".into();
    for index in 0..512 {
        locked.registry.receipts.insert(
            format!("receipt-{index}"),
            IdempotencyReceipt {
                principal: "fixture".into(),
                incarnation: "fixture-runtime".into(),
                operation: "message-send".into(),
                digest: "fixture-digest".into(),
                outcome: json!({"body": "x".repeat(2048)}),
                expires_at_epoch: now_epoch() + RECEIPT_TTL_SECS,
            },
        );
    }
    locked.save().unwrap();
    let path = locked.path.clone();
    drop(locked);

    assert!(fs::metadata(&path).unwrap().len() > 1024 * 1024);
    let initial = public_summary_registry_snapshot(&context).unwrap();
    assert_eq!(initial.receipts.len(), 512);
    let before = read_bytes();
    for _ in 0..4 {
        let snapshot = public_summary_registry_snapshot(&context).unwrap();
        assert_eq!(snapshot.fingerprint_key, "fixture-before");
        let locked = lock_registry_observational(&context).unwrap();
        assert_eq!(locked.registry.receipts.len(), 512);
    }
    let unchanged = read_bytes() - before;
    assert!(
        unchanged < READ_BUDGET,
        "unchanged registry polls read {unchanged} bytes (budget {READ_BUDGET})"
    );

    let mut locked = lock_registry_observational(&context).unwrap();
    locked.registry.fingerprint_key = "fixture-after".into();
    locked.save().unwrap();
    drop(locked);
    let current = public_summary_registry_snapshot(&context).unwrap();
    assert_eq!(current.fingerprint_key, "fixture-after");

    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(public_summary_registry_snapshot(&context).is_none());
    assert!(lock_registry_observational(&context).is_err());
}

#[test]
fn codex_notification_poll_reads_are_bounded_and_observe_appends() {
    let temp = tempfile::tempdir().unwrap();
    let context = context(&temp);
    let session = "fixture";
    let incarnation = "fixture-runtime";
    let provider_session = "11111111-1111-4111-8111-111111111111";
    let provider_root = temp.path().join("codex");
    let timestamp = "2026-01-01T00:00:00Z";
    let attempted_at = "2026-01-01T00:00:01Z".parse::<jiff::Timestamp>().unwrap();
    let record: crate::SessionRecord = serde_json::from_value(json!({
        "schema_version": "agent-session.session.v1",
        "id": session, "agent": "codex", "mode": "interactive", "cwd": temp.path(),
        "tmux_session": "hs-fixture", "created_at": timestamp, "updated_at": timestamp,
        "runtime": {
            "kind": "tmux", "tmux_session": "hs-fixture", "launch_id": incarnation,
            "generation": 1, "started_at": timestamp,
            "agent_profile_provider_config_dir": provider_root,
        },
        "provider_resume": {
            "provider": "codex", "session_id": provider_session, "captured_at": timestamp,
            "capture_method": "fixture", "resume_args": [],
        },
    }))
    .unwrap();
    fs::create_dir_all(crate::session_dir(&context, session)).unwrap();
    crate::write_session_record(&context, &record).unwrap();

    let rollout = provider_root.join(format!(
        "sessions/2026/01/01/rollout-{provider_session}.jsonl"
    ));
    fs::create_dir_all(rollout.parent().unwrap()).unwrap();
    let header = json!({
        "timestamp": timestamp, "type": "session_meta",
        "payload": {"id": provider_session, "cwd": temp.path(), "source": "cli", "timestamp": timestamp},
    });
    let filler = format!(
        "{}\n",
        json!({"type": "response_item", "payload": {"output": "x".repeat(4096)}})
    );
    fs::write(&rollout, format!("{header}\n{}", filler.repeat(512))).unwrap();

    let prompt = "<!-- agent-notice:v1 kind=mailbox.alert source=agent-session -->\n\nSynthetic mailbox alert.";
    let mut locked = lock_registry_observational(&context).unwrap();
    let key = format!("recipient-{}", digest_bytes(b"fixture\0fixture-runtime"));
    locked.registry.notifications.insert(
        key,
        notification::NotificationReceipt {
            target_session_id: session.into(),
            target_incarnation: incarnation.into(),
            generation: 1,
            attempted_generation: 1,
            attempted_at_epoch: attempted_at.as_second(),
            attempted_at: Some(attempted_at.to_string()),
            attempted_prompt_format: Some("mailbox-notice.v1".into()),
            attempted_prompt: Some(prompt.into()),
            ..Default::default()
        },
    );
    locked.save().unwrap();
    drop(locked);
    let candidate = NotificationCandidate {
        target_session_id: session.into(),
        target_incarnation: incarnation.into(),
        generation: 1,
        queued_at_epoch: 0,
        attempted_at_epoch: attempted_at.as_second(),
        attempted_at: Some(attempted_at.to_string()),
        prompt: Some(prompt.into()),
    };
    let observe = || notification::observe_attempt(&context, &candidate);
    assert_eq!(observe(), None);

    let before = read_bytes();
    for _ in 0..4 {
        assert_eq!(observe(), None);
    }
    let unchanged = read_bytes() - before;
    assert!(
        unchanged < READ_BUDGET,
        "unchanged Codex rollout polls read {unchanged} bytes (budget {READ_BUDGET})"
    );

    let match_row = json!({
        "timestamp": "2026-01-01T00:00:02Z", "type": "event_msg",
        "payload": {"type": "user_message", "message": prompt},
    })
    .to_string();
    let mut writer = fs::OpenOptions::new().append(true).open(&rollout).unwrap();
    let before = read_bytes();
    let (first, rest) = match_row.split_at(match_row.len() / 2);
    write!(writer, "{first}").unwrap();
    assert_eq!(observe(), None);
    writeln!(writer, "{rest}").unwrap();
    assert_eq!(observe(), Some(()));
    let appended = read_bytes() - before;
    assert!(
        appended < READ_BUDGET,
        "appended Codex rollout polls read {appended} bytes (budget {READ_BUDGET})"
    );

    let replacement = rollout.with_extension("replacement");
    fs::write(&replacement, format!("{header}\n{filler}")).unwrap();
    fs::rename(replacement, &rollout).unwrap();
    assert_eq!(observe(), None);
}
