use super::*;
use pretty_assertions::assert_eq;

#[derive(Clone, Copy)]
enum AttemptPrompt { Notice, MissingNotice, Current, Retired, UnknownFormat }

#[tokio::test]
async fn coordination_notification_reconcile_finds_delivered_envelope_before_later_prompt() {
    check_attempt(AttemptPrompt::Current, "2030-01-01T00:01:40.800000000Z", "prompt_submitted").await;
}

#[tokio::test]
async fn mailbox_notice_does_not_replay_after_absence_or_ambiguous_timestamp() {
    check_attempt(AttemptPrompt::Current, "2030-01-01T00:01:40.600000000Z", "attempt_unknown").await;
    check_attempt(AttemptPrompt::Current, "", "attempt_unknown").await;
}

#[tokio::test]
async fn mailbox_notice_rejects_retired_and_unknown_attempt_formats() {
    check_attempt(AttemptPrompt::Retired, "2030-01-01T00:01:40.800000000Z", "attempt_unknown").await;
    check_attempt(AttemptPrompt::UnknownFormat, "2030-01-01T00:01:40.800000000Z", "attempt_unknown").await;
}

async fn check_attempt(form: AttemptPrompt, submitted_at: &str, expected_state: &str) {
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let codex_home = tmp.path().join("codex-home");
        let transcript = codex_home.join("sessions/2026/07/session.jsonl");
        fs::create_dir_all(transcript.parent().expect("transcript parent"))
            .expect("transcript dir");
        let provider_id = "notification-reconcile-delivered";
        write_codex_notification_transcript(&transcript, provider_id, &[]);
        let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().unwrap());

        seed_session_with_runtime(tmp.path(), "alpha", "codex", "hs-codex-alpha");
        seed_fresh_provider_session(
            tmp.path(),
            "beta",
            "codex",
            "hs-codex-beta",
            tmp.path(),
            Some((provider_id, "codex-explicit-session-id")),
        );
        let state = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
        let alpha = load_session_record(&state.context, "alpha").expect("alpha");
        let beta = load_session_record(&state.context, "beta").expect("beta");
        let alpha_capability = provision_ready_coordination_fixture(&state.context, &alpha);
        provision_ready_coordination_fixture(&state.context, &beta);

        send_notification_fixture(&state, &alpha_capability, "notification-delivered-gen").await;
        let mut candidate = crate::coordination::pending_notifications(&state.context)
            .expect("pending notification")
            .pop()
            .expect("notification candidate");
        if matches!(form, AttemptPrompt::Notice | AttemptPrompt::MissingNotice | AttemptPrompt::Current) {
            candidate.prompt = Some(crate::notice::NoticeEnvelope::new("mailbox.wake", "agent-session", None)
                .unwrap().encode("Mailbox: 1 uncategorized; agent-session message inbox").unwrap());
        }
        assert!(
            crate::coordination::begin_notification_attempt(&state.context, &candidate)
                .expect("begin notification")
        );
        assert!(
            crate::coordination::mark_notification_unknown(
                &state.context,
                &candidate,
                "submission-outcome-unknown",
            )
            .expect("unknown notification")
        );
        let path = tmp.path().join("coordination/registry.json");
        let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let receipt = registry["notifications"].as_object_mut().unwrap().values_mut().next().unwrap();
        assert!(receipt.get("composer_recovery_prompt").is_none());
        if matches!(form, AttemptPrompt::Retired) {
            receipt["attempted_prompt_format"] = json!("mailbox-short.v2");
            receipt["attempted_prompt"] = json!("Mailbox: agent-session message inbox");
        }
        if matches!(form, AttemptPrompt::UnknownFormat) {
            receipt["attempted_prompt_format"] = json!("unrecognized-format");
        }
        fs::write(path, serde_json::to_vec_pretty(&registry).unwrap()).unwrap();
        let prompt = match form {
            AttemptPrompt::MissingNotice => "unrelated provider input".to_string(),
            AttemptPrompt::Retired => "Mailbox: agent-session message inbox".into(),
            _ => crate::coordination::notification_prompt(&candidate),
        };
        write_codex_notification_transcript(
            &transcript,
            provider_id,
            &[
                (
                    submitted_at,
                    prompt,
                ),
                (
                    "2030-01-01T00:01:41.100000000Z",
                    "later unrelated recipient prompt".to_string(),
                ),
            ],
        );
        set_notification_attempted_at(tmp.path(), "2030-01-01T00:01:40.700000000Z");

        reconcile_coordination_notifications(state.clone()).await;

        let notification = notification_fixture(tmp.path());
        assert_eq!(notification["state"], expected_state);
        assert_eq!(notification["notified_generation"], if expected_state == "prompt_submitted" { 1 } else { 0 });
        assert_eq!(
            crate::coordination::pending_notifications(&state.context)
                .expect("pending notifications").len(),
            if expected_state == "queued" { 1 } else { 0 },
            "only proven absence may queue another delivery"
        );
}

/// The exact prompt the fixture's current notification generation carries.
pub(super) fn fixture_notification_prompt(state_dir: &Path) -> String {
    let notification = notification_fixture(state_dir);
    if notification["attempted_generation"] == notification["generation"]
        && let Some(prompt) = notification["attempted_prompt"].as_str()
    {
        return prompt.to_owned();
    }
    let context = CliContext { state_dir: state_dir.into(), host: None };
    let session = notification["target_session_id"].as_str().unwrap();
    let mut candidate = crate::coordination::NotificationCandidate {
        prompt: None,
        target_session_id: session.into(),
        target_incarnation: notification["target_incarnation"].as_str().unwrap().into(),
        generation: notification["generation"].as_u64().unwrap(),
        queued_at_epoch: notification["queued_at_epoch"].as_i64().unwrap(),
        attempted_at_epoch: 0,
        attempted_at: None,
    };
    let record = load_session_record(&context, session).unwrap();
    candidate.prepare_notice(&context, &record).unwrap();
    crate::coordination::notification_prompt(&candidate)
}

#[tokio::test]
async fn mailbox_notice_reconciliation_requires_positive_exact_observation() {
    check_attempt(AttemptPrompt::Notice, "2030-01-01T00:01:40.800000000Z", "prompt_submitted").await;
    check_attempt(AttemptPrompt::MissingNotice, "2030-01-01T00:01:40.800000000Z", "attempt_unknown").await;
}
