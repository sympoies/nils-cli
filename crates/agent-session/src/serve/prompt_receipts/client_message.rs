//! The provider-visible id a keyed native dispatch carries.
use super::store;

/// Derived from the full dispatch-time identity digest (the same session,
/// runtime, provider thread, account binding and socket identity that a
/// retained proof fences on) and the request id. A later reconciliation
/// computes it from the identity current at lookup, so a changed identity
/// computes a different id and finds nothing. Nothing extra is stored. It is a
/// digest, so it reveals neither the prompt nor the request id.
pub(super) fn id(identity: &str, request_id: &str) -> String {
    let digest = store::digest(&(
        "agent-session.prompt-client-message.v2",
        identity,
        request_id,
    ));
    format!("agent-session-{}", &digest[..32])
}
