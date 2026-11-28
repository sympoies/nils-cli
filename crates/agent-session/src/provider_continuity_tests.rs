use super::*;
use pretty_assertions::assert_eq;

const NEXT: &str = "cleared-conversation";

#[test]
fn unowned_observation_and_noop_resume_do_not_mint_continuity() {
    let tmp = tempfile::tempdir().unwrap();
    let context = context(tmp.path());
    let fresh = seed_exited_session(&context, tmp.path());
    assert!(!fresh.extra.contains_key("provider_continuity"));
    let tmux = fake_tmux(tmp.path());
    let launch = running_session(&context, &tmux);
    session_ready(&context, &launch);
    let before = snapshot(&context, &tmux);
    crate::resume_session_by_id(&context, ID, &tmux).unwrap();
    assert_eq!(
        snapshot(&context, &tmux)["provider_continuity"],
        before["provider_continuity"]
    );
    // A clear in the provider UI is observable, but was not performed by this
    // lifecycle tool and therefore cannot extend an ownership proof.
    claude_hook(
        &context,
        &launch,
        json!({"hook_event_name":"SessionStart","source":"clear","session_id":NEXT}),
    );
    assert!(
        snapshot(&context, &tmux)
            .get("provider_continuity")
            .is_none()
    );
}

#[test]
fn failed_relaunch_restores_the_previous_continuity_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let context = context(tmp.path());
    seed_exited_session(&context, tmp.path());
    let tmux = fake_tmux(tmp.path());
    let launch = running_session(&context, &tmux);
    session_ready(&context, &launch);
    let before = snapshot(&context, &tmux);
    fs::write(tmp.path().join("fail-launch"), "").unwrap();
    assert_eq!(
        restart(&context, &restart_args(&tmux, false, 1))
            .unwrap_err()
            .code(),
        "restart-relaunch-failed"
    );
    let record = load_session_record(&context, ID).unwrap();
    assert_eq!(
        record.extra["provider_continuity"],
        before["provider_continuity"]
    );
    assert_eq!(record.runtime.unwrap().launch_id, launch);
}

fn snapshot(context: &CliContext, tmux: &Path) -> Value {
    let record = load_session_record(context, ID).unwrap();
    serde_json::to_value(crate::session_view(
        context,
        &record,
        Some("running".into()),
        Some(tmux),
    ))
    .unwrap()
}

#[test]
fn two_root_clear_restart_snapshot_carries_owned_provider_continuity() {
    let recipient = tempfile::tempdir().unwrap();
    let sender = tempfile::tempdir().unwrap();
    let recipient_context = context(recipient.path());
    let sender_context = context(sender.path());
    seed_exited_session(&recipient_context, recipient.path());
    seed_exited_session(&sender_context, sender.path());
    let tmux = fake_tmux(recipient.path());
    let original_launch = running_session(&recipient_context, &tmux);
    session_ready(&recipient_context, &original_launch);
    let before = snapshot(&recipient_context, &tmux);

    // Only this synthetic recipient receives the clear; the other root has
    // the same managed id and must not acquire its continuity evidence.
    fs::write(recipient.path().join("clear-requested"), "").unwrap();
    let observer = test_control::on(
        &recipient_context,
        test_control::Phase::ClearSubmitted,
        "",
        move |context, launch| {
            claude_hook(
                context,
                launch,
                json!({"hook_event_name":"SessionStart","source":"clear","session_id":NEXT}),
            );
        },
    );
    let clear_status = crate::conversation::run(
        &recipient_context,
        crate::cli::ConversationArgs {
            id: ID.into(),
            expect_idle: true,
            timeout: 10,
            tmux_bin: Some((*tmux).clone()),
            format: nils_common::cli_contract::OutputFormat::Json,
        },
        crate::conversation::Operation::Clear(Default::default()),
    );
    observer.join().unwrap();
    assert_eq!(clear_status, 0);
    let after_clear = snapshot(&recipient_context, &tmux);

    let ready = on_relaunch(&recipient_context, &original_launch, |context, launch| {
        claude_hook(
            context,
            launch,
            json!({"hook_event_name":"SessionStart","source":"resume","session_id":NEXT}),
        );
    });
    let restarted = restart(&recipient_context, &restart_args(&tmux, false, 30));
    ready.join().unwrap();
    restarted.unwrap();
    let after = snapshot(&recipient_context, &tmux);
    let chain = &after["provider_continuity"];
    assert_eq!(
        chain["schema_version"],
        "agent-session.provider-continuity.v1"
    );
    assert_eq!(chain["provider"], "claude");
    let links = chain["links"].as_array().unwrap();
    assert_eq!(links.len(), 3);
    assert_eq!(links[0]["kind"], "resume");
    assert_eq!(links[1]["kind"], "clear");
    assert_eq!(links[1]["from_incarnation"], before["session_incarnation"]);
    assert_eq!(
        links[1]["to_incarnation"],
        after_clear["session_incarnation"]
    );
    assert_eq!(links[1]["from_conversation"], CONVERSATION);
    assert_eq!(links[1]["to_conversation"], NEXT);
    assert_eq!(links[2]["kind"], "restart");
    assert_eq!(
        links[2]["from_incarnation"],
        after_clear["session_incarnation"]
    );
    assert_eq!(links[2]["to_incarnation"], after["session_incarnation"]);
    assert_eq!(links[2]["from_conversation"], NEXT);
    assert_eq!(
        links[2]["to_conversation"],
        after["provider_resume"]["session_id"]
    );
    assert!(
        links
            .windows(2)
            .all(|pair| pair[0]["at_epoch"].as_i64() <= pair[1]["at_epoch"].as_i64())
    );
    assert!(
        !load_session_record(&sender_context, ID)
            .unwrap()
            .extra
            .contains_key("provider_continuity")
    );
    // Evidence survives re-reading the durable record; a stopped snapshot does
    // not offer a proof to the relay.
    let persisted = load_session_record(&recipient_context, ID).unwrap();
    assert_eq!(persisted.extra["provider_continuity"], *chain);
    let stopped = serde_json::to_value(crate::session_view(
        &recipient_context,
        &persisted,
        Some("stopped".into()),
        Some(&tmux),
    ))
    .unwrap();
    assert!(stopped.get("provider_continuity").is_none());
}
