//! Allowlisted forwarding and metadata-only receipts for known launch failures.
use crate::CliError;
use serde_json::{Value, json};

fn identifier(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| {
        !text.is_empty()
            && text.len() <= 128
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    })
}

pub(super) fn forward(error: &Value) -> Option<CliError> {
    if error["code"] == "session-capacity-exceeded" {
        return crate::admission::forward_refusal(error);
    }
    if error["code"] != "start-failed" {
        return None;
    }
    let source = &error["details"];
    let reason = source["reason"]
        .as_str()
        .filter(|reason| {
            matches!(
                *reason,
                "runtime-exited-during-startup"
                    | "startup-trust-required"
                    | "startup-prompt-required"
            )
        })
        .unwrap_or("startup-unconfirmed");
    let complete = source["cleanup"]["state"] == "completed";
    let retryable = reason == "runtime-exited-during-startup"
        && complete
        && source["retryable"] == true
        && source["safe_to_retry"] == true
        && source["recovery"]["safe_to_retry"] == true
        && identifier(&source["session_id"]).is_some();
    let action = match (reason, complete, retryable) {
        (_, _, true) => "retry_create",
        ("startup-trust-required", true, _) => "configure_provider_trust",
        ("startup-prompt-required", true, _) => "configure_provider",
        _ => "inspect_session",
    };
    let mut details = json!({
        "reason":reason, "retryable":retryable, "safe_to_retry":retryable,
        "next_action":action, "recovery":{"action":action, "safe_to_retry":retryable},
        "provider_exit_code":source["provider_exit_code"].as_u64().filter(|code| *code <= 255),
        "cleanup":cleanup(&source["cleanup"])
    });
    if let Some(id) = identifier(&source["session_id"]) {
        details["session_id"] = json!(id);
    }
    if let Some(tail) = source["stderr_tail"]
        .as_str()
        .filter(|tail| tail.len() <= 16384)
    {
        details["stderr_tail"] = json!(crate::startup::redact_diagnostic(tail));
    }
    if let Ok(mut startup) =
        serde_json::from_value::<crate::StartupProjection>(source["startup"].clone())
        && crate::startup_projection_is_valid(&startup)
    {
        startup.extra.clear();
        details["startup"] = json!(startup);
    }
    Some(CliError::runtime(
        "start-failed",
        "The provider failed during session startup.",
        Some(details),
    ))
}

pub(super) fn receipt(error: &CliError) -> Option<Value> {
    if error.code() != "start-failed" {
        return None;
    }
    let details = error.details()?;
    let id = identifier(&details["session_id"])?;
    if !matches!(
        details["reason"].as_str(),
        Some(
            "runtime-exited-during-startup" | "startup-trust-required" | "startup-prompt-required"
        )
    ) {
        return None;
    }
    // Revalidate policy even for a local caller; diagnostics never enter durable receipts.
    let projected = forward(&json!({"code":"start-failed", "details":details}))?;
    let mut metadata = projected.details()?.clone();
    metadata.as_object_mut()?.remove("stderr_tail");
    metadata.as_object_mut()?.remove("startup");
    metadata["session_id"] = json!(id);
    metadata["code"] = json!("start-failed");
    Some(metadata)
}

fn cleanup(source: &Value) -> Value {
    if source["state"] == "completed" {
        return json!({"state":"completed"});
    }
    let state = match source["state"].as_str() {
        Some("pending") => "pending",
        _ => "blocked",
    };
    let reason = source["reason"]
        .as_str()
        .filter(|reason| {
            matches!(
                *reason,
                "session_still_running"
                    | "process_boundary_live"
                    | "runtime_identity_changed"
                    | "runtime_identity_unavailable"
                    | "termination_failed"
                    | "termination_timeout"
                    | "verification_failed"
                    | "cleanup_unavailable"
            )
        })
        .unwrap_or("unknown");
    json!({"state":state, "reason":reason})
}
