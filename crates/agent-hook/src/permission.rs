//! Relay explicit owner answers through the current synchronous Claude hook.
use crate::{
    adapter, evaluator,
    model::{DecisionAction, NormalizedDecision, NormalizedRequest, Product},
};
use serde_json::{Value, json};
use std::{
    process::{Command, Stdio},
    time::Duration,
};

pub(crate) fn await_decision(
    request: &NormalizedRequest,
    raw: &[u8],
    decision: &mut NormalizedDecision,
) {
    if request.product != Product::Claude
        || request.event != "PermissionRequest"
        || matches!(
            decision.action,
            DecisionAction::Block | DecisionAction::Transform
        )
        || decision.provider_output.is_some()
    {
        return;
    }
    let Ok(raw_value) = serde_json::from_slice::<Value>(raw) else {
        return;
    };
    // Claude Code >=2.1.317 requires updatedInput for interactive tools; they
    // keep their question/plan workflow and are never blanket-approved here.
    if matches!(
        raw_value["tool_name"].as_str(),
        Some("AskUserQuestion" | "ExitPlanMode")
    ) {
        return;
    }
    let (Ok(id), Ok(runtime)) = (
        std::env::var("AGENT_SESSION_ID"),
        std::env::var("AGENT_SESSION_RUNTIME_ID"),
    ) else {
        return;
    };
    let Ok(Some(event)) = adapter::normalize_activity_event(request, raw, &runtime) else {
        return;
    };
    let Ok(event) = serde_json::from_slice::<Value>(&event) else {
        return;
    };
    let Some(attention) = event["attention_id"].as_str() else {
        return;
    };
    let Ok(helper) = evaluator::resolve_activity_helper() else {
        return;
    };
    let mut command = Command::new(helper);
    command
        .args([
            "permission",
            "wait",
            &id,
            "--runtime-id",
            &runtime,
            "--attention-id",
            attention,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Outside policy evaluation's short execution budget: this is a bounded
    // human decision wait. The waiter expires at 540s; the provider default is
    // 600s. Failure returns no decision and preserves conservative attention.
    let Ok(output) = evaluator::run_bounded(command, &[], Duration::from_secs(550), 1024, false)
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let Ok(output) = serde_json::from_slice::<Value>(&output.stdout) else {
        return;
    };
    let Some(behavior @ ("allow" | "deny")) =
        output["hookSpecificOutput"]["decision"]["behavior"].as_str()
    else {
        return;
    };
    let canonical = json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": {"behavior": behavior}}});
    if output == canonical {
        decision.provider_output = Some(canonical);
    }
}
