//! Poll-only mailbox operations for authenticated service recipients.
use serde_json::{Value, json};

use super::{
    clean_expired, idempotency_replay, lock_registry, now_epoch, request_digest, store_receipt,
};
use super::{mailbox, remote, service};
use crate::{CliContext, CliError, cli};

pub(crate) enum Operation {
    Inbox {
        state: Option<String>,
        cursor: Option<String>,
        limit: Option<usize>,
    },
    Show(String),
    Ack {
        message: String,
        if_revision: u64,
        idempotency_key: String,
    },
}

pub(crate) fn poll(
    context: &CliContext,
    recipient: &service::Origin,
    operation: Operation,
    authenticate: impl FnOnce() -> Result<(), CliError>,
) -> Result<Value, CliError> {
    let mut locked = lock_registry(context)?;
    authenticate()?;
    let now = now_epoch();
    clean_expired(&mut locked.registry, now);
    let principal = recipient.recipient_key();
    let generation = &recipient.service_generation;
    let result = match operation {
        Operation::Inbox {
            state,
            cursor,
            limit,
        } => {
            let filters = mailbox::InboxFilters {
                state: state.as_deref(),
                cursor: cursor.as_deref(),
                limit,
                categories: &[],
            };
            let limit = filters.validate()?;
            let (messages, next_cursor) = mailbox::inbox_page(
                &mut locked.registry,
                &principal,
                generation,
                &filters,
                limit,
                now,
            )?;
            let rows: Vec<_> = messages
                .iter()
                .map(|message| metadata(message, recipient))
                .collect();
            json!({
                "schema_version": "agent-session.message-inbox.v1",
                "messages": rows,
                "next_cursor": next_cursor,
            })
        }
        Operation::Show(id) => {
            let message = mailbox::find_recipient_message_mut(
                &mut locked.registry,
                &principal,
                generation,
                &id,
            )?;
            if message.state == "expired" {
                return Err(mailbox::message_expired());
            }
            if message.state == "unread" {
                message.state = "read".into();
                message.revision = message.revision.saturating_add(1);
            }
            let mut result = metadata(message, recipient);
            result["body"] = json!({ "classification": "peer_message", "text": message.body });
            result
        }
        Operation::Ack {
            message,
            if_revision,
            idempotency_key,
        } => {
            let operation = "message-service-ack";
            let digest = request_digest(operation, &(&principal, &message, if_revision));
            if let Some(receipt) = idempotency_replay(
                &locked.registry,
                &idempotency_key,
                &principal,
                generation,
                operation,
                &digest,
            )? {
                return Ok(receipt);
            }
            let message = mailbox::find_recipient_message_mut(
                &mut locked.registry,
                &principal,
                generation,
                &message,
            )?;
            if message.state == "expired" {
                return Err(mailbox::message_expired());
            }
            if message.revision != if_revision {
                return Err(mailbox::message_revision_conflict());
            }
            message.state = "acknowledged".into();
            message.revision = message.revision.saturating_add(1);
            message.terminal_at_epoch = Some(now);
            let outcome = metadata(message, recipient);
            store_receipt(
                &mut locked.registry,
                idempotency_key,
                principal,
                generation.clone(),
                operation.into(),
                digest,
                outcome.clone(),
                now,
            )?;
            outcome
        }
    };
    locked.save()?;
    Ok(result)
}

pub(super) fn metadata(message: &mailbox::StoredMessage, recipient: &service::Origin) -> Value {
    let mut projected = mailbox::message_projection(message);
    let fields = projected
        .as_object_mut()
        .expect("message metadata is an object");
    fields.remove("recipient_session_id");
    fields.insert("recipient".into(), recipient.recipient_projection());
    projected
}

pub(super) fn quota_recovery(error: CliError, recipient: &service::Origin) -> CliError {
    if error.code() != "quota-exceeded" {
        return error;
    }
    let Some(details) = error.details().filter(|details| {
        details["quota"]
            .as_str()
            .is_some_and(mailbox::recipient_quota)
    }) else {
        return error;
    };
    let name = crate::executable();
    let scope = format!(
        "--service {} --service-generation {} --credential-file <credential-file>",
        recipient.service_id, recipient.service_generation
    );
    let guidance = format!(
        "The recipient service must read pending messages with {name} message service-show, then acknowledge handled messages with {name} message service-ack using the returned revision and an idempotency key"
    );
    let message = format!(
        "coordination mailbox quota exceeded; {guidance} ({} {}/{})",
        details["quota"].as_str().expect("recipient quota name"),
        details["count"],
        details["limit"]
    );
    let mut details = details.clone();
    details["recovery"] = json!({
        "actor": "recipient",
        "action": "read_and_ack",
        "inbox_unread": format!("{name} message service-inbox {scope} --state unread --limit 100 --format json"),
        "inbox_read": format!("{name} message service-inbox {scope} --state read --limit 100 --format json"),
        "show": format!("{name} message service-show {scope} --message <message-id> --format json"),
        "ack": format!("{name} message service-ack {scope} --message <message-id> --if-revision <returned-revision> --idempotency-key <unique-key> --format json"),
        "guidance": guidance,
    });
    CliError::data("quota-exceeded", message, Some(details))
}

pub(super) fn daemon_payload(result: Result<Value, CliError>) -> Result<Value, CliError> {
    let mut value = result.map_err(|error| {
        if error.code() == "mailbox-service-unavailable" {
            let inner = error.into_inner();
            CliError::unavailable(inner.code, inner.message, inner.details)
        } else {
            error
        }
    })?;
    value
        .pointer_mut("/data/coordination")
        .map(Value::take)
        .ok_or_else(|| {
            CliError::unavailable(
                "coordination-unavailable",
                "local mailbox response is invalid",
                None,
            )
        })
}

pub(super) fn cli_inbox(
    context: &CliContext,
    args: cli::MessageServiceInboxArgs,
) -> Result<Value, CliError> {
    cli_poll(
        context,
        args.credential,
        Operation::Inbox {
            state: args.state,
            cursor: args.cursor,
            limit: args.limit,
        },
    )
}

pub(super) fn cli_show(
    context: &CliContext,
    args: cli::MessageServiceShowArgs,
) -> Result<Value, CliError> {
    cli_poll(context, args.credential, Operation::Show(args.message))
}

pub(super) fn cli_ack(
    context: &CliContext,
    args: cli::MessageServiceAckArgs,
) -> Result<Value, CliError> {
    cli_poll(
        context,
        args.credential,
        Operation::Ack {
            message: args.message,
            if_revision: args.if_revision,
            idempotency_key: args.idempotency_key,
        },
    )
}

fn cli_poll(
    context: &CliContext,
    args: cli::ServiceMailboxCredential,
    operation: Operation,
) -> Result<Value, CliError> {
    let token = service::credential(&args.credential_file)?;
    let mut url = remote::daemon_url(context)?;
    let client = remote::daemon_client()?;
    let mut query = vec![
        ("service_id", args.service),
        ("service_generation", args.service_generation),
    ];
    let (path, ack) = match operation {
        Operation::Inbox {
            state,
            cursor,
            limit,
        } => {
            query.extend(state.map(|value| ("state", value)));
            query.extend(cursor.map(|value| ("cursor", value)));
            query.extend(limit.map(|value| ("limit", value.to_string())));
            (service::ROUTE.into(), None)
        }
        Operation::Show(id) => {
            let id = uuid::Uuid::parse_str(&id).map_err(|_| mailbox::message_not_found())?;
            (format!("/coordination/services/messages/{id}/v1"), None)
        }
        Operation::Ack {
            message,
            if_revision,
            idempotency_key,
        } => {
            let id = uuid::Uuid::parse_str(&message).map_err(|_| mailbox::message_not_found())?;
            (
                format!("/coordination/services/messages/{id}/ack/v1"),
                Some(json!({ "if_revision": if_revision, "idempotency_key": idempotency_key })),
            )
        }
    };
    url.set_path(&path);
    url.set_query(None);
    url.query_pairs_mut().extend_pairs(query);
    let request = match ack {
        Some(ack) => client.post(url).json(&ack),
        None => client.get(url),
    };
    let response = request.bearer_auth(token).send().map_err(|_| {
        CliError::unavailable(
            "coordination-unavailable",
            "local mailbox daemon is unavailable",
            None,
        )
    })?;
    daemon_payload(remote::local_response(response))
}
