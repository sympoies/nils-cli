//! Display-only mailbox read events (`docs/specs/mailbox-read-events-v1.md`).
//!
//! The wrappers run the unchanged mailbox operation first, then best-effort
//! record the event. Recording never fails the read, never touches notice
//! input or delivery, and never changes a notification generation.
use serde_json::Value;

use super::{lock_registry, mailbox};
use crate::cli::{MessageAckArgs, MessageShowArgs};
use crate::{CliContext, CliError};

mod anchor;
pub(crate) mod store;
pub(crate) mod view;

pub(crate) fn show(context: &CliContext, args: MessageShowArgs) -> Result<Value, CliError> {
    let (session, message) = (args.session.clone(), args.message.clone());
    let result = mailbox::show(context, args)?;
    record(context, &session, &message, store::Action::Read);
    Ok(result)
}

pub(crate) fn ack(context: &CliContext, args: MessageAckArgs) -> Result<Value, CliError> {
    let (session, message) = (args.session.clone(), args.message.clone());
    let result = mailbox::ack(context, args)?;
    record(context, &session, &message, store::Action::Acknowledge);
    Ok(result)
}

fn record(context: &CliContext, session: &str, message_id: &str, action: store::Action) {
    let _ = try_record(context, session, message_id, action);
}

fn try_record(
    context: &CliContext,
    session: &str,
    message_id: &str,
    action: store::Action,
) -> Option<()> {
    let record = crate::load_session_record(context, session).ok()?;
    let resume = record.provider_resume.as_ref()?;
    let history_id =
        crate::provider_history::stable_history_id(&resume.provider, None, &resume.session_id);
    let source = {
        let locked = lock_registry(context).ok()?;
        let stored = locked.registry.messages.iter().find(|stored| {
            stored.message_id == message_id && stored.recipient_session_id == session
        })?;
        store::Source {
            message_id: stored.message_id.clone(),
            sender: view::sender(&mailbox::message_projection(stored)["sender"], context),
            category: serde_json::to_value(stored.category.unwrap_or_default())
                .ok()?
                .as_str()?
                .to_string(),
            body: stored.body.clone(),
        }
    };
    // The anchor is read last, so it follows any provider rows already written.
    let anchor = anchor::transcript_len(&resume.provider, &resume.session_id)?;
    store::record(&context.state_dir, &history_id, &source, anchor, action).ok()
}
