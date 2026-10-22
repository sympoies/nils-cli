//! Opt-in execution gate for the one-shot checks read.
use super::{FailedCheck, PendingCheck, PrChecksPayload, SCHEMA, SCHEMA_VERSION, render_text};
use crate::cli::BINARY;
use crate::envelope::emit_success;
use crate::ops::required_check_gate::nothing_was_checked;
use nils_common::cli_contract::{Envelope, EnvelopeError, OutputFormat, exit, schema_version_for};

pub(super) fn emit(mut payload: PrChecksPayload, strict: bool, format: OutputFormat) -> i32 {
    let schema = schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION);
    if strict {
        let empty = nothing_was_checked(&payload);
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
            .unwrap_or(if empty { "pending" } else { "success" });
        // Rebuild from the effective gate: the ordinary snapshot can omit
        // visible fallback rows from its required-only detail lists.
        payload.failed.clear();
        payload.pending.clear();
        for c in gate {
            match c.state {
                "failure" | "timed_out" | "cancelled" | "skipped" | "neutral" => {
                    payload.failed.push(FailedCheck {
                        name: c.name.clone(),
                        url: c.url.clone(),
                        conclusion: c.conclusion.clone().or_else(|| {
                            matches!(c.state, "skipped" | "neutral").then(|| c.state.to_string())
                        }),
                    });
                }
                "pending" => payload.pending.push(PendingCheck {
                    name: c.name.clone(),
                    url: c.url.clone(),
                }),
                _ => {}
            }
        }
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
