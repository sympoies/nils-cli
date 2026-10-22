//! Opt-in execution gate for the one-shot checks read.
use super::{FailedCheck, PrChecksPayload, SCHEMA, SCHEMA_VERSION, render_text};
use crate::cli::BINARY;
use crate::envelope::emit_success;
use nils_common::cli_contract::{Envelope, EnvelopeError, OutputFormat, exit, schema_version_for};

pub(super) fn emit(mut payload: PrChecksPayload, strict: bool, format: OutputFormat) -> i32 {
    let schema = schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION);
    if strict {
        let gate: Vec<_> = payload
            .checks
            .iter()
            .filter(|c| payload.gate_scope != "required" || c.required)
            .collect();
        let has_skipped = gate
            .iter()
            .any(|c| matches!(c.state, "skipped" | "neutral"));
        payload.state = ["failure", "timed_out", "cancelled", "pending"]
            .into_iter()
            .find(|state| {
                (*state == "failure" && has_skipped) || gate.iter().any(|c| c.state == *state)
            })
            .unwrap_or("success");
        payload.failed.extend(
            gate.into_iter()
                .filter(|c| matches!(c.state, "skipped" | "neutral"))
                .map(|c| FailedCheck {
                    name: c.name.clone(),
                    url: c.url.clone(),
                    conclusion: Some(c.conclusion.clone().unwrap_or_else(|| c.state.to_string())),
                }),
        );
        if payload.state != "success" {
            let kind = if payload.state == "pending" {
                "checks_pending"
            } else {
                "checks_failed"
            };
            let message = "strict checks require every gating check to conclude success";
            let envelope = Envelope {
                schema_version: schema,
                ok: false,
                data: Some(payload),
                warnings: Vec::new(),
                error: Some(EnvelopeError {
                    code: kind.into(),
                    message: message.into(),
                    hint: None,
                    details: None,
                }),
            };
            match format {
                OutputFormat::Json => match serde_json::to_string(&envelope) {
                    Ok(json) => println!("{json}"),
                    Err(_) => return exit::SOFTWARE,
                },
                OutputFormat::Text => {
                    eprintln!("error: {kind}: {message}");
                    if let Some(data) = &envelope.data {
                        render_text(data);
                    }
                }
            }
            return exit::RUNTIME;
        }
    }
    emit_success(schema, payload, format, render_text)
}
