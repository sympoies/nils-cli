use std::{sync::mpsc, thread, time::Duration};

use super::Fixture;
use crate::cli::MessageSendArgs;
use crate::coordination::{lock_registry, mailbox};
use crate::metadata::send_labels::{Label, set};
use nils_common::cli_contract::OutputFormat;
use pretty_assertions::assert_eq;

#[test]
fn send_labels_rotation_cannot_interleave_comparison_and_commit() {
    let fixture = Fixture::new("codex");
    let context = &fixture.context;
    let recipient = "recipient";
    let old = "assignment=old";
    let body = context.state_dir.join("body.txt");
    std::fs::write(&body, "synthetic message").unwrap();
    set(context, recipient, &Label::parse(old).unwrap()).unwrap();
    let send = |key: &str| {
        mailbox::send(
            context,
            MessageSendArgs {
                from_session: "caller".into(),
                to_session: Some(recipient.into()),
                to_machine: None,
                expect_label: Some(old.into()),
                body_file: body.clone(),
                capability_file: Some(fixture.capability.clone()),
                idempotency_key: key.into(),
                reply_to: None,
                reply_by: None,
                expires_in: None,
                format: OutputFormat::Json,
                category: None,
            },
        )
    };

    let timeout = Duration::from_secs(10);
    let (compared, start_rotation) = mpsc::channel();
    let (probed, rotation_probed) = mpsc::channel();
    thread::scope(|scope| {
        let writer = scope.spawn(move || {
            start_rotation.recv_timeout(timeout).unwrap();
            let lock = crate::try_acquire_session_record_lock(context, recipient).unwrap();
            let locked_by_send = lock.is_none();
            drop(lock);
            probed.send(locked_by_send).unwrap();
            set(context, recipient, &Label::parse("assignment=new").unwrap()).unwrap();
        });
        mailbox::AFTER_LABEL_CHECK.with(|hook| {
            hook.set(Some(Box::new(move || {
                compared.send(()).unwrap();
                assert!(
                    rotation_probed.recv_timeout(timeout).unwrap(),
                    "the workflow could acquire the recipient lock after comparison"
                );
            })));
        });
        let sent = send("before-rotation").unwrap();
        assert_eq!(sent["state"], "unread");
        writer.join().unwrap();
        let error = send("after-rotation").unwrap_err();
        assert_eq!(error.code(), "send-label-mismatch");
        let locked = lock_registry(context).unwrap();
        assert_eq!(locked.registry.messages.len(), 1);
        assert_eq!(locked.registry.messages[0].message_id, sent["message_id"]);
    });
}
