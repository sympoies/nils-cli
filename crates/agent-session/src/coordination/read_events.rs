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
    show_with(context, args, || {})
}

pub(crate) fn ack(context: &CliContext, args: MessageAckArgs) -> Result<Value, CliError> {
    ack_with(context, args, || {})
}

/// `after_commit` runs once the mailbox operation has committed and before the
/// event is recorded; tests use it to interleave a conversation rebind.
pub(crate) fn show_with(
    context: &CliContext,
    args: MessageShowArgs,
    after_commit: impl FnOnce(),
) -> Result<Value, CliError> {
    let (session, message) = (args.session.clone(), args.message.clone());
    let before = Binding::capture(context, &session, &message);
    let result = mailbox::show(context, args)?;
    after_commit();
    record(context, &session, &message, before, store::Action::Read);
    Ok(result)
}

pub(crate) fn ack_with(
    context: &CliContext,
    args: MessageAckArgs,
    after_commit: impl FnOnce(),
) -> Result<Value, CliError> {
    let (session, message) = (args.session.clone(), args.message.clone());
    let before = Binding::capture(context, &session, &message);
    let result = mailbox::ack(context, args)?;
    after_commit();
    record(
        context,
        &session,
        &message,
        before,
        store::Action::Acknowledge,
    );
    Ok(result)
}

/// What the read or ack is attributed to: the recipient incarnation holding the
/// message and the provider conversation it is bound to, including the launch
/// profile and provider configuration root persisted on the session record.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    recipient_incarnation: String,
    provider: String,
    provider_session: String,
    profile: Option<String>,
    config_dir: Option<std::path::PathBuf>,
}

impl Binding {
    fn capture(context: &CliContext, session: &str, message_id: &str) -> Option<Self> {
        let record = crate::load_session_record(context, session).ok()?;
        let resume = record.provider_resume.as_ref()?;
        let recipient_incarnation = lock_registry(context)
            .ok()?
            .registry
            .messages
            .iter()
            .find(|stored| {
                stored.message_id == message_id && stored.recipient_session_id == session
            })?
            .recipient_incarnation
            .clone();
        Some(Self {
            recipient_incarnation,
            provider: resume.provider.clone(),
            provider_session: resume.session_id.clone(),
            profile: crate::session_agent_profile(&record).map(str::to_string),
            config_dir: crate::session_effective_provider_config_dir(&record),
        })
    }
}

/// Records only when the binding observed before the operation still holds
/// after it committed; a rebind in between skips the display record and never
/// affects the completed mailbox operation.
fn record(
    context: &CliContext,
    session: &str,
    message_id: &str,
    before: Option<Binding>,
    action: store::Action,
) {
    let _ = try_record(context, session, message_id, before, action);
}

fn try_record(
    context: &CliContext,
    session: &str,
    message_id: &str,
    before: Option<Binding>,
    action: store::Action,
) -> Option<()> {
    let binding = before?;
    (Binding::capture(context, session, message_id)? == binding).then_some(())?;
    let history_id = crate::provider_history::stable_history_id(
        &binding.provider,
        binding.profile.as_deref(),
        &binding.provider_session,
    );
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
    let anchor = anchor::transcript_len(
        &binding.provider,
        &binding.provider_session,
        binding.config_dir.as_deref(),
    )?;
    store::record(&context.state_dir, &history_id, &source, anchor, action).ok()
}
