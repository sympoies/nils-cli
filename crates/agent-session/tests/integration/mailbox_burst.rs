//! The per-pair mailbox burst limit counts messages stored within one second.

use std::fs;

use pretty_assertions::{assert_eq, assert_ne};

use super::coordination::{capability, data, rewrite_registry, run, seed_brokers};

const BURST_MESSAGES: usize = 10;

/// Ten real sends, then one more inside the same burst window.
///
/// The limit is a wall-clock second that the product reads itself, so a host
/// that needs longer than that to launch eleven processes would let the last
/// send through. After the ten real sends, re-stamp them just ahead of the
/// clock: the window then still holds all ten however long the eleventh
/// process takes to start.
pub(super) fn burst_and_cursor_are_bounded() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            ("alpha", "inc-alpha", "alpha-private-capability-material"),
            ("beta", "inc-beta", "beta-private-capability-material"),
        ],
    );
    let body = tmp.path().join("body.txt");
    fs::write(&body, "body").expect("body");
    let send = |index: usize| {
        let key = format!("round3-burst-{index:04}");
        run(
            tmp.path(),
            &[
                "--state-dir",
                state_dir.to_str().expect("state"),
                "message",
                "send",
                "--from",
                "alpha",
                "--to",
                "beta",
                "--body-file",
                body.to_str().expect("body"),
                "--capability-file",
                &capability(&state_dir, "alpha"),
                "--idempotency-key",
                &key,
                "--format",
                "json",
            ],
        )
    };
    let mut sent_ids = Vec::new();
    for index in 0..BURST_MESSAGES {
        let output = send(index);
        assert_eq!(
            output.code,
            0,
            "index={index} stderr={}",
            output.stderr_text()
        );
        sent_ids.push(
            data(&output)["message_id"]
                .as_str()
                .expect("id")
                .to_string(),
        );
    }
    let ahead_millis = jiff::Timestamp::now().as_millisecond() + 60_000;
    rewrite_registry(&state_dir, |registry| {
        let messages = registry["messages"].as_array_mut().expect("messages");
        assert_eq!(messages.len(), BURST_MESSAGES);
        for (index, message) in messages.iter_mut().enumerate() {
            let millis = ahead_millis + index as i64;
            message["created_at_epoch_millis"] = millis.into();
            message["created_at_epoch"] = (millis / 1000).into();
        }
    });
    let eleventh = send(BURST_MESSAGES);
    assert_ne!(eleventh.code, 0);
    assert_eq!(eleventh.stdout_json()["error"]["code"], "rate-limited");
    let inbox = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "inbox",
            "--session",
            "beta",
            "--capability-file",
            &capability(&state_dir, "beta"),
            "--limit",
            "1",
            "--format",
            "json",
        ],
    );
    assert_eq!(inbox.code, 0, "stderr={}", inbox.stderr_text());
    let inbox_data = data(&inbox);
    let cursor = inbox_data["next_cursor"].as_str().expect("cursor");
    assert!(!sent_ids.iter().any(|message_id| message_id == cursor));
}
