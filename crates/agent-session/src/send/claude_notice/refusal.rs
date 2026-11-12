//! Fixed diagnostic labels: never include native payloads or private paths.
use std::io;

use serde_json::json;

use super::{bridge, runtime};
use crate::CliError;

pub(super) fn unavailable(reason: &'static str) -> CliError {
    CliError::runtime(
        "notice-native-unavailable",
        format!("native notice capability is unavailable: {reason}"),
        Some(json!({"reason": reason})),
    )
}

pub(super) fn receive(error: io::Error) -> CliError {
    unavailable(match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "authorization-timeout",
        io::ErrorKind::InvalidData => "authorization-protocol",
        _ => "authorization-connection",
    })
}

pub(super) fn authorize(
    path: &str,
    request: &bridge::Authorize,
    op: &str,
    bootstrap: &runtime::Bootstrap,
    ready: &runtime::Ready,
) -> Result<(), CliError> {
    for (failed, reason) in [
        (path != "/authorize", "authorization-path"),
        (
            request.schema_version != "agent-session.claude-notice-authorize.v1",
            "authorization-schema",
        ),
        (request.op != op, "authorization-operation"),
        (request.binding != ready.binding, "authorization-binding"),
        (request.nonce != bootstrap.nonce, "authorization-nonce"),
        (!request.terminal, "authorization-surface"),
        (!request.box_empty, "authorization-composer-not-empty"),
        (request.cursor != 0, "authorization-cursor"),
    ] {
        if failed {
            return Err(unavailable(reason));
        }
    }
    Ok(())
}
