//! Content-free observation of a structured prompt left in the composer.
use nils_common::observation::{self, Component, Event, Severity};
use sha2::{Digest, Sha256};

use crate::{CliContext, CliError, SessionRecord};

/// The receipt reaches only the caller of one request. Record classification of
/// the refusal where `agent-session diagnose` reads it; never the prompt text.
pub(super) fn record_failure(context: &CliContext, record: &SessionRecord, error: &CliError) {
    let digest: String = Sha256::digest(record.id.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let correlation = format!("sha256:{digest}");
    let event = Event::new(
        Component::AgentSession,
        "prompt-delivery",
        "prompt-pasted-not-submitted",
        Severity::Error,
        env!("CARGO_PKG_VERSION"),
        jiff::Timestamp::now().as_second(),
    )
    .and_then(|event| event.with_provider(&record.agent, "StructuredPrompt"))
    .and_then(|event| event.with_correlation(&correlation))
    .and_then(|event| {
        event.with_recovery_action(&format!(
            "{} logs <SESSION>",
            crate::command_identity::executable()
        ))
    });
    // A refusal code that is not a bounded slug only loses its disposition.
    let event = event.map(|event| {
        let slug = error.code();
        event.clone().with_disposition(slug).unwrap_or(event)
    });
    if let Ok(event) = event {
        let _ = observation::append(&context.state_dir, &event);
    }
}
