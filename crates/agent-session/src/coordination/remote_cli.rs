//! CLI mailbox requests to the local federation daemon.
use super::*;

pub(crate) fn cli_send(
    context: &CliContext,
    args: crate::cli::MessageSendArgs,
) -> Result<Value, CliError> {
    super::super::mailbox::parse_reply_by(args.reply_by.as_deref())?;
    let body = super::super::mailbox::read_body(&args.body_file)?;
    local_request(
        context,
        &args.from_session,
        args.capability_file.as_deref(),
        &format!("/sessions/{}/messages/remote/v1", args.from_session),
        Some(with_category(
            json!({"to_machine":args.to_machine,"to_session":args.to_session,"body":body,"idempotency_key":args.idempotency_key,"reply_to":args.reply_to,"expires_in":args.expires_in,"reply_by":args.reply_by,"reply_revision":null}),
            args.category,
        )),
    )
}
pub(crate) fn cli_reply(
    context: &CliContext,
    args: &crate::cli::MessageReplyArgs,
    original: &super::super::mailbox::StoredMessage,
    body: String,
) -> Result<Value, CliError> {
    let sender = sender_address(original).ok_or_else(invalid)?;
    local_request(
        context,
        &args.session,
        args.capability_file.as_deref(),
        &format!("/sessions/{}/messages/remote/v1", args.session),
        Some(with_category(
            json!({"to_machine":sender.machine,"to_session":sender.session_id,"body":body,"idempotency_key":args.idempotency_key,"reply_to":args.message,"expires_in":null,"reply_revision":args.if_revision}),
            args.category,
        )),
    )
}
pub(crate) fn cli_peers(
    context: &CliContext,
    args: crate::cli::MessagePeersArgs,
) -> Result<Value, CliError> {
    local_request(
        context,
        &args.session,
        args.capability_file.as_deref(),
        &format!("/sessions/{}/messages/peers/v1", args.session),
        None,
    )
}
