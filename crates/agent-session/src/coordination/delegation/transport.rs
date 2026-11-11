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
    let usage = match body.filter(|_| delegated_send) {
        Some(body) => super::Use::MessageSend {
            machine: body["to_machine"]
                .as_str()
                .ok_or_else(super::scope_denied)?,
            recipient: body["to_session"]
                .as_str()
                .ok_or_else(super::scope_denied)?,
        },
        None => super::Use::CapabilityOnly,
    };
    // The daemon authenticates again; this pre-check must not queue behind registry maintenance.
    let token = super::super::capability_token_from_file(Some(capability))?;
    super::authenticate_snapshot_first(context, session, &token, usage)?;
    Ok(())
}
