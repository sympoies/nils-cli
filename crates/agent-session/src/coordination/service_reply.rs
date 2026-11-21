//! Local reply admission under the owning daemon's registry lock.
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    LockedRegistry, authenticate_from_file, clean_expired, digest_bytes, idempotency_replay,
    lock_registry, now_epoch, request_digest, revalidate_capability_file, store_receipt, timestamp,
};
use super::{mailbox, remote, service, service_mailbox, unauthorized};
use crate::{CliContext, CliError, cli};

pub(crate) struct Daemon<'a> {
    pub machine: &'a str,
    pub admission: Option<&'a Path>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    body: String,
    idempotency_key: String,
    if_revision: Option<u64>,
    expires_in: Option<String>,
    reply_by: Option<String>,
    category: Option<cli::MessageCategory>,
}

impl Request {
    pub(super) fn checked(args: cli::MessageReplyArgs, body: String) -> Self {
        Self {
            body,
            idempotency_key: args.idempotency_key,
            if_revision: Some(args.if_revision),
            expires_in: None,
            reply_by: None,
            category: args.category,
        }
    }

    fn operation(&self) -> &'static str {
        if self.if_revision.is_some() {
            "message-reply"
        } else {
            "message-send"
        }
    }

    fn digest(&self, sender: &str, message: &str) -> Result<String, CliError> {
        if let Some(if_revision) = self.if_revision {
            // The parent UUID identifies the immutable stored origin. Use the
            // ordinary reply digest so both revision-checked transports replay.
            return Ok(mailbox::reply_request_digest(
                sender,
                message,
                &self.body,
                if_revision,
                self.category,
            ));
        }
        Ok(request_digest(
            self.operation(),
            &json!({
                "sender": sender,
                "recipient": null,
                "reply_to": message,
                "body_digest": digest_bytes(self.body.as_bytes()),
                "expiry_secs": mailbox::parse_expiry(self.expires_in.as_deref())?,
                "reply_by_secs": mailbox::parse_reply_by(self.reply_by.as_deref())?,
                "category": self.category,
            }),
        ))
    }
}

pub(crate) fn is_service_result(result: &Value) -> bool {
    result.pointer("/recipient/kind").and_then(Value::as_str) == Some("service")
}

pub(super) fn cli_send(
    context: &CliContext,
    args: cli::MessageSendArgs,
) -> Result<Value, CliError> {
    let message = args
        .reply_to
        .as_deref()
        .ok_or_else(mailbox::message_not_found)?;
    let request = Request {
        body: mailbox::read_body(&args.body_file)?,
        idempotency_key: args.idempotency_key,
        if_revision: None,
        expires_in: args.expires_in,
        reply_by: args.reply_by,
        category: args.category,
    };
    cli_submit(
        context,
        &args.from_session,
        args.capability_file.as_deref(),
        message,
        request,
    )
}

pub(super) fn cli_reply(
    context: &CliContext,
    args: cli::MessageReplyArgs,
    body: String,
) -> Result<Value, CliError> {
    let session = args.session.clone();
    let capability = args.capability_file.clone();
    let message = args.message.clone();
    cli_submit(
        context,
        &session,
        capability.as_deref(),
        &message,
        Request::checked(args, body),
    )
}

fn cli_submit(
    context: &CliContext,
    session: &str,
    capability: Option<&Path>,
    message: &str,
    request: Request,
) -> Result<Value, CliError> {
    let message = uuid::Uuid::parse_str(message).map_err(|_| mailbox::message_not_found())?;
    let payload = serde_json::to_value(request).expect("service reply request serializes");
    service_mailbox::daemon_payload(remote::local_service_reply(
        context,
        session,
        capability,
        &format!("/sessions/{session}/messages/{message}/service-reply/v1"),
        payload,
    ))
}

pub(super) fn receive(
    context: &CliContext,
    daemon: Daemon<'_>,
    session: &str,
    message: &str,
    capability: &Path,
    request: Request,
) -> Result<Value, CliError> {
    if request.if_revision.is_some() && (request.expires_in.is_some() || request.reply_by.is_some())
    {
        return Err(CliError::usage(
            "mailbox-service-request-invalid",
            "revision-checked replies do not accept expiry or deadline selectors",
            None,
        ));
    }
    mailbox::validate_body(&request.body)?;
    let (record, incarnation) = authenticate_from_file(context, session, Some(capability))?;
    let _sender_lock =
        crate::acquire_session_record_lock(context, &record.id).map_err(|_| unauthorized())?;
    let sender = crate::load_session_record(context, &record.id).map_err(|_| unauthorized())?;
    let mut locked = lock_registry(context)?;
    revalidate_capability_file(context, &locked.registry, &sender, &incarnation, capability)?;
    let now = now_epoch();
    clean_expired(&mut locked.registry, now);
    let digest = request.digest(&sender.id, message)?;
    if let Some(receipt) = idempotency_replay(
        &locked.registry,
        &request.idempotency_key,
        &sender.id,
        &incarnation,
        request.operation(),
        &digest,
    )? {
        return if is_service_result(&receipt) {
            Ok(receipt)
        } else {
            Err(mailbox::message_not_found())
        };
    }
    let parent = mailbox::find_recipient_message_mut(
        &mut locked.registry,
        &sender.id,
        &incarnation,
        message,
    )?
    .clone();
    commit(
        locked,
        daemon,
        (&sender.id, &incarnation),
        &parent,
        request,
        digest,
        now,
    )
}

pub(super) fn commit(
    mut locked: LockedRegistry,
    daemon: Daemon<'_>,
    (sender, incarnation): (&str, &str),
    parent: &mailbox::StoredMessage,
    request: Request,
    digest: String,
    now: i64,
) -> Result<Value, CliError> {
    let destination = service::sender_origin(parent).ok_or_else(mailbox::message_not_found)?;
    if destination.machine != daemon.machine {
        return Err(service::refusal(
            "mailbox-service-reply-route-unsupported",
            "the service reply destination is on another machine",
        ));
    }
    if parent.state == "expired" || parent.expires_at_epoch <= now {
        return Err(mailbox::message_expired());
    }
    if request
        .if_revision
        .is_some_and(|revision| parent.revision != revision)
    {
        return Err(mailbox::message_revision_conflict());
    }
    if parent.reply_depth >= mailbox::MAX_REPLY_DEPTH {
        return Err(CliError::data(
            "reply-depth-exceeded",
            "message reply depth limit exceeded",
            None,
        ));
    }
    service::require_recipient(daemon.admission, &destination)?;
    let expiry = now.saturating_add(mailbox::parse_expiry(request.expires_in.as_deref())?);
    let reply_by = mailbox::parse_reply_by(request.reply_by.as_deref())?;
    let recipient = destination.recipient_key();
    let now_millis = mailbox::now_epoch_millis();
    mailbox::admit_message(
        &mut locked.registry,
        sender,
        &recipient,
        request.body.len(),
        Some(&parent.message_id),
        now,
        now_millis,
    )
    .map_err(|error| service_mailbox::quota_recovery(error, &destination))?;

    let operation = request.operation();
    let message = mailbox::StoredMessage {
        schema_version: "agent-session.message.v1".into(),
        message_id: uuid::Uuid::new_v4().to_string(),
        sender_session_id: sender.into(),
        sender_incarnation: incarnation.into(),
        recipient_session_id: recipient,
        recipient_incarnation: destination.service_generation.clone(),
        state: "unread".into(),
        revision: 1,
        reply_to: Some(parent.message_id.clone()),
        reply_by_epoch: reply_by.map(|seconds| now.saturating_add(seconds)),
        reply_sent_at_epoch: None,
        reply_depth: parent.reply_depth + 1,
        created_at: timestamp(now),
        created_at_epoch: now,
        remote_created_at_epoch: None,
        created_at_epoch_millis: now_millis,
        expires_at: timestamp(expiry),
        expires_at_epoch: expiry,
        terminal_at_epoch: None,
        forwarded_from_incarnation: None,
        forwarded_at_epoch: None,
        category: request.category,
        notification_suppressed: false,
        forwarding: None,
        resume_carry: None,
        body_bytes: request.body.len(),
        body: request.body,
    };
    let outcome = service_mailbox::metadata(&message, &destination);
    store_receipt(
        &mut locked.registry,
        request.idempotency_key,
        sender.into(),
        incarnation.into(),
        operation.into(),
        digest,
        outcome.clone(),
        now,
    )?;
    let original = mailbox::find_recipient_message_mut(
        &mut locked.registry,
        sender,
        incarnation,
        &parent.message_id,
    )?;
    original.reply_sent_at_epoch.get_or_insert(now);
    locked.registry.messages.push(message);
    locked.save()?;
    Ok(outcome)
}
