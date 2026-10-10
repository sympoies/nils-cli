//! Map coordination commands to their envelope names, including raw-argument
//! routing used when clap rejects a command before it is parsed.

use std::ffi::OsString;

use crate::cli::{self, Command};

pub(crate) fn command_name(command: &Command) -> Option<&'static str> {
    match command {
        Command::Readiness(_) => Some("readiness"),
        Command::WorkContext(args) => Some(match &args.command {
            cli::WorkContextCommand::Status(_) => "work-context-status",
            cli::WorkContextCommand::Set(_) => "work-context-set",
            cli::WorkContextCommand::Clear(_) => "work-context-clear",
            cli::WorkContextCommand::Advise(_) => "work-context-advise",
            cli::WorkContextCommand::Acknowledge(_) => "work-context-acknowledge",
            cli::WorkContextCommand::Claim(_) => "work-context-claim",
            cli::WorkContextCommand::Show(_) => "work-context-show",
            cli::WorkContextCommand::Check(_) => "work-context-check",
            cli::WorkContextCommand::Renew(_) => "work-context-renew",
            cli::WorkContextCommand::Release(_) => "work-context-release",
            cli::WorkContextCommand::Admit(_) => "work-context-admit",
            cli::WorkContextCommand::Complete(_) => "work-context-complete",
            cli::WorkContextCommand::Reconcile(_) => "work-context-reconcile",
        }),
        Command::Broker(args) => Some(match &args.command {
            cli::BrokerCommand::Identity(_) => "broker-identity",
            cli::BrokerCommand::Status(_) => "broker-status",
            cli::BrokerCommand::Adopt(_) => "broker-adopt",
            cli::BrokerCommand::Reconcile(_) => "broker-reconcile",
            cli::BrokerCommand::Stop(_) => "broker-stop",
            cli::BrokerCommand::Heartbeat(_) => "broker-heartbeat",
        }),
        Command::Message(args) => Some(match &args.command {
            cli::MessageCommand::Audit(_) => "message-audit",
            cli::MessageCommand::Peers(_) => "message-peers",
            cli::MessageCommand::Delivery(_) => "message-delivery",
            cli::MessageCommand::ServiceSend(_) => "message-service-send",
            cli::MessageCommand::Send(_) => "message-send",
            cli::MessageCommand::Forward(_) => "message-forward",
            cli::MessageCommand::Inbox(_) => "message-inbox",
            cli::MessageCommand::Show(_) => "message-show",
            cli::MessageCommand::Ack(_) => "message-ack",
            cli::MessageCommand::Reply(_) => "message-reply",
            cli::MessageCommand::Wait(_) => "message-wait",
            cli::MessageCommand::Reminder(_) => "message-reminder",
        }),
        Command::Metadata(args) => Some(match &args.command {
            cli::MetadataCommand::Attach(_) => "metadata-attach",
            cli::MetadataCommand::Show(_) => "metadata-show",
        }),
        Command::Account(args) => Some(match &args.command {
            cli::AccountCommand::Show(_) => "account-show",
            cli::AccountCommand::Switch(_) => "account-switch",
        }),
        _ => None,
    }
}

pub(crate) fn leaf_from_raw_args(args: &[OsString]) -> Option<&'static str> {
    let mut root_args = args.iter().skip(1);
    while let Some(arg) = root_args.next() {
        // Match bytes so a non-UTF-8 option value cannot hide later command names.
        match arg.as_encoded_bytes() {
            b"--state-dir" | b"--host" => {
                root_args.next()?;
            }
            arg if arg.starts_with(b"--state-dir=") || arg.starts_with(b"--host=") => {}
            b"readiness" => return Some("readiness"),
            _ => break,
        }
    }
    args.windows(2).find_map(|pair| {
        let group = pair[0].to_str()?;
        let leaf = pair[1].to_str()?;
        match (group, leaf) {
            ("work-context", "status") => Some("work-context-status"),
            ("work-context", "set") => Some("work-context-set"),
            ("work-context", "clear") => Some("work-context-clear"),
            ("work-context", "advise") => Some("work-context-advise"),
            ("work-context", "acknowledge") => Some("work-context-acknowledge"),
            ("work-context", "claim") => Some("work-context-claim"),
            ("work-context", "show") => Some("work-context-show"),
            ("work-context", "check") => Some("work-context-check"),
            ("work-context", "renew") => Some("work-context-renew"),
            ("work-context", "release") => Some("work-context-release"),
            ("work-context", "admit") => Some("work-context-admit"),
            ("work-context", "complete") => Some("work-context-complete"),
            ("work-context", "reconcile") => Some("work-context-reconcile"),
            ("broker", "identity") => Some("broker-identity"),
            ("broker", "status") => Some("broker-status"),
            ("broker", "adopt") => Some("broker-adopt"),
            ("broker", "reconcile") => Some("broker-reconcile"),
            ("broker", "stop") => Some("broker-stop"),
            ("message", "audit") => Some("message-audit"),
            ("message", "send") => Some("message-send"),
            ("message", "peers") => Some("message-peers"),
            ("message", "delivery") => Some("message-delivery"),
            ("message", "inbox") => Some("message-inbox"),
            ("message", "show") => Some("message-show"),
            ("message", "ack") => Some("message-ack"),
            ("message", "reply") => Some("message-reply"),
            ("message", "wait") => Some("message-wait"),
            ("message", "reminder") => Some("message-reminder"),
            ("metadata", "attach") => Some("metadata-attach"),
            ("metadata", "show") => Some("metadata-show"),
            ("account", "show") => Some("account-show"),
            ("account", "switch") => Some("account-switch"),
            _ => None,
        }
    })
}
