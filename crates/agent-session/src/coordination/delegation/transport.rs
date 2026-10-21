use crate::{CliContext, CliError};
use serde_json::Value;
use std::path::Path;

pub(crate) fn authenticate_remote_request(
    context: &CliContext,
    session: &str,
    capability: &Path,
    path: &str,
    body: Option<&Value>,
) -> Result<(), CliError> {
    let delegated_send = path == format!("/sessions/{session}/messages/remote/v1")
        && body
            .as_ref()
            .is_some_and(|body| body.get("forward").is_none());
    if delegated_send {
        let body = body.expect("checked body");
        super::authenticate_file(
            context,
            session,
            Some(capability),
            super::Use::MessageSend {
                machine: body["to_machine"]
                    .as_str()
                    .ok_or_else(super::scope_denied)?,
                recipient: body["to_session"]
                    .as_str()
                    .ok_or_else(super::scope_denied)?,
            },
            false,
        )?;
    } else {
        super::super::authenticate_from_file(context, session, Some(capability))?;
    }
    Ok(())
}
