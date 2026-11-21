//! Content-free observation of a structured prompt left in the composer.
use nils_common::observation::{self, Component, Event, FieldError, Severity};
use sha2::{Digest, Sha256};

use crate::{CliContext, CliError, SessionRecord};

/// The receipt reaches only the caller of one request. Record classification of
/// the refusal where `agent-session diagnose` reads it; never the prompt text.
/// A recording failure never changes the refusal, but it is reported on stderr
/// so a missing observation cannot go unnoticed.
pub(super) fn record_failure(context: &CliContext, record: &SessionRecord, error: &CliError) {
    report(context, record, error, &mut std::io::stderr());
}

/// Write the bounded note to `out` when the observation cannot be recorded.
pub(crate) fn report(
    context: &CliContext,
    record: &SessionRecord,
    error: &CliError,
    out: &mut impl std::io::Write,
) {
    if let Err(note) = try_record(context, record, error) {
        let _ = writeln!(out, "{note}");
    }
}

/// Append the observation, returning a bounded, content-free note on failure.
fn try_record(
    context: &CliContext,
    record: &SessionRecord,
    error: &CliError,
) -> Result<(), String> {
    let note = |reason: &dyn std::fmt::Debug| {
        format!("warning: prompt-pasted-not-submitted observation not recorded: {reason:?}")
    };
    let event = event(record, error).map_err(|reason| note(&reason))?;
    observation::append(&context.state_dir, &event).map_err(|reason| note(&reason))
}

fn event(record: &SessionRecord, error: &CliError) -> Result<Event, FieldError> {
    let digest: String = Sha256::digest(record.id.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let event = Event::new(
        Component::AgentSession,
        "prompt-delivery",
        "prompt-pasted-not-submitted",
        Severity::Error,
        env!("CARGO_PKG_VERSION"),
        jiff::Timestamp::now().as_second(),
    )?
    .with_provider(&record.agent, "StructuredPrompt")?
    .with_correlation(&format!("sha256:{digest}"))?
    .with_recovery_action(&format!(
        "{} logs <SESSION>",
        crate::command_identity::executable()
    ))?;
    // A refusal code that is not a bounded slug only loses its disposition.
    Ok(event
        .clone()
        .with_disposition(error.code())
        .unwrap_or(event))
}
