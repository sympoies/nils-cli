use super::{render_coordination, service_mailbox};
use crate::{CliContext, cli};

pub(super) fn inbox(context: &CliContext, args: cli::MessageServiceInboxArgs) -> i32 {
    render_coordination(
        "message-service-inbox",
        args.credential.format,
        service_mailbox::cli_inbox(context, args),
    )
}

pub(super) fn show(context: &CliContext, args: cli::MessageServiceShowArgs) -> i32 {
    render_coordination(
        "message-service-show",
        args.credential.format,
        service_mailbox::cli_show(context, args),
    )
}

pub(super) fn ack(context: &CliContext, args: cli::MessageServiceAckArgs) -> i32 {
    render_coordination(
        "message-service-ack",
        args.credential.format,
        service_mailbox::cli_ack(context, args),
    )
}
