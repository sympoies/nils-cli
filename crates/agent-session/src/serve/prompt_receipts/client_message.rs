//! The provider-visible id a keyed native dispatch carries.
use super::store;

/// Derived from the durable claim identity alone, so the dispatch and a later
/// reconciliation compute the same value and nothing extra is stored. It is a
/// digest, so it reveals neither the prompt nor the request id.
pub(super) fn id(session_id: &str, incarnation: &str, request_id: &str) -> String {
    let digest = store::digest(&(
        "agent-session.prompt-client-message.v1",
        session_id,
        incarnation,
        request_id,
    ));
    format!("agent-session-{}", &digest[..32])
}
