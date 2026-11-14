use super::*;
use agent_hook::tool_call::{ToolCallEvidence, ToolCallPhase};

/// Claude runs one turn at a time and names it with `prompt_id`, but a turn
/// whose `UserPromptSubmit` was never recorded (a lost or refused hook
/// delivery) is never announced. An attention request from another turn
/// therefore proves the open turn is stale rather than that the request is
/// foreign, so it supersedes that turn instead of being refused
/// (sympoies/nils-cli#1962). Other providers keep the exact turn binding.
pub(super) fn claude_attention_supersedes_open_turn(provider: &str) -> bool {
    provider == AgentKind::Claude.as_str()
}

/// A Claude attention request for the most recently closed turn, while a
/// different identified turn is open. Superseding on it would reopen an older
/// turn. In every other case the request is live and is reduced normally:
/// Claude keeps stamping hooks with a prompt that `idle_prompt` already closed
/// while a background subagent is still working under it.
pub(super) fn late_claude_attention_for_closed_turn(
    document: &ActivityDocument,
    event: &TurnEvent,
) -> bool {
    let Some(event_turn) = event.provider_turn_id.as_ref() else {
        return false;
    };
    event.kind == TurnEventKind::AttentionRequested
        && claude_attention_supersedes_open_turn(event.provider.as_str())
        && document
            .state
            .last_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.as_ref())
            == Some(event_turn)
        && document
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.as_ref())
            .is_some_and(|open| open != event_turn)
}

// Approval correlation. `PermissionRequest` has no `tool_use_id`, so a request
// is bound to the call it belongs to by turn, tool name and input digest, and
// only when exactly one open call of the turn matches. A bound request is then
// cleared by that call's own `PostToolUse`, `PostToolUseFailure` or
// `PermissionDenied`. Anything else stays on the conservative latch that only a
// same-turn Stop, a new turn or a completion clears. A manual denial and an
// interrupt emit no hook at all, so they end only the same way.

const MAX_OPEN_TOOL_CALLS: usize = 64;

/// `PreToolUse` calls of the current turn that have no result yet.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct OpenToolCalls(Vec<OpenToolCall>);

impl OpenToolCalls {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct OpenToolCall {
    turn_id: String,
    call_id: String,
    input_digest: String,
    /// The pending approval bound to this call, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    approval_id: Option<String>,
}

fn evidence(event: &TurnEvent) -> Option<(&ToolCallEvidence, &String)> {
    if event.provider != AgentKind::Claude.as_str() || event.source_kind != SourceKind::ProviderHook
    {
        return None;
    }
    Some((event.tool_call.as_ref()?, event.provider_turn_id.as_ref()?))
}

/// Part of the duplicate-suppression key: two tool events of one turn within a
/// second are different events when their call or phase differs.
pub(super) fn semantic_discriminator(event: &TurnEvent) -> String {
    event.tool_call.as_ref().map_or_else(String::new, |call| {
        format!(
            "{:?}\0{}\0{}",
            call.phase,
            call.call_id.as_deref().unwrap_or(""),
            call.input_digest.as_deref().unwrap_or("")
        )
    })
}

/// Record a started call, or settle a finished one and the approval bound to it.
pub(super) fn observe_tool_progress(document: &mut ActivityDocument, event: &TurnEvent) {
    let Some((call, turn)) = evidence(event) else {
        return;
    };
    match (
        call.phase,
        call.call_id.as_ref(),
        call.input_digest.as_ref(),
    ) {
        (ToolCallPhase::Started, Some(call_id), Some(input_digest)) => {
            let calls = &mut document.open_tool_calls.0;
            calls.retain(|open| &open.turn_id == turn && &open.call_id != call_id);
            let evicted = (calls.len() >= MAX_OPEN_TOOL_CALLS)
                .then(|| calls.remove(0).approval_id)
                .flatten();
            calls.push(OpenToolCall {
                turn_id: turn.clone(),
                call_id: call_id.clone(),
                input_digest: input_digest.clone(),
                approval_id: None,
            });
            // An evicted call can no longer deliver the result that clears its
            // approval, so that approval is no longer exactly correlated.
            if let Some(approval_id) = evicted
                && let Some(pending) = document
                    .pending_attention
                    .iter_mut()
                    .find(|pending| pending.id == approval_id)
            {
                pending.certainty = AttentionCertainty::Conservative;
                refresh_attention(document);
            }
        }
        (ToolCallPhase::Finished, Some(call_id), _) => {
            let calls = &mut document.open_tool_calls.0;
            let Some(position) = calls
                .iter()
                .position(|open| &open.turn_id == turn && &open.call_id == call_id)
            else {
                return;
            };
            if let Some(approval_id) = calls.remove(position).approval_id {
                document
                    .pending_attention
                    .retain(|pending| pending.id != approval_id);
                refresh_attention(document);
            }
        }
        _ => {}
    }
}

/// Name the tool on the pending approval this event just recorded and, when it
/// matches exactly one unbound open call of the turn, bind them.
pub(super) fn observe_permission(document: &mut ActivityDocument, event: &TurnEvent) {
    let (Some((call, turn)), Some(attention_id)) = (evidence(event), event.attention_id.as_ref())
    else {
        return;
    };
    if call.phase != ToolCallPhase::Permission {
        return;
    }
    let Some(pending) = document
        .pending_attention
        .iter_mut()
        .find(|pending| &pending.id == attention_id)
    else {
        return;
    };
    if let Some(name) = &call.tool_name {
        pending
            .extra
            .insert("tool_name".to_string(), Value::String(name.clone()));
    }
    let Some(digest) = &call.input_digest else {
        return;
    };
    let mut candidates = document.open_tool_calls.0.iter_mut().filter(|open| {
        &open.turn_id == turn && &open.input_digest == digest && open.approval_id.is_none()
    });
    if let (Some(open), None) = (candidates.next(), candidates.next()) {
        open.approval_id = Some(attention_id.clone());
        pending.certainty = AttentionCertainty::Exact;
    }
}

/// Run an answer operation while the exact current permission binding is locked.
/// Callers hold the session record lock first, matching activity ingress order.
pub(crate) fn with_permission_call<T>(
    context: &CliContext,
    record: &SessionRecord,
    attention_id: &str,
    operation: impl FnOnce(&str) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let _ingress = pending::probe_lock(context, &record.id)?
        .ok_or_else(|| crate::cli::permission::refusal("permission-event-pending"))?;
    let dir = session_dir(context, &record.id);
    let _lock = acquire_lock(&dir)?;
    let document = read_document(&dir.join(ACTIVITY_FILE))?;
    let runtime = record.runtime.as_ref();
    let turn = document
        .state
        .current_turn
        .as_ref()
        .and_then(|t| t.provider_turn_id.as_ref());
    let pending = document
        .pending_attention
        .iter()
        .find(|p| p.id == attention_id && p.kind == "approval");
    if record.agent != "claude"
        || runtime.is_none_or(|r| {
            r.launch_id != document.runtime_id || r.generation != document.runtime_generation
        })
        || document.runtime_unhealthy_reason.is_some()
        || provider_session_for_runtime(record, &document.runtime_id)?
            != document.provider_session_id
        || pending.is_none_or(|p| {
            p.certainty != AttentionCertainty::Exact
                || matches!(
                    p.extra.get("tool_name").and_then(Value::as_str),
                    Some("AskUserQuestion" | "ExitPlanMode")
                )
        })
    {
        return Err(crate::cli::permission::refusal(
            "permission-request-uncorrelated",
        ));
    }
    let mut calls = document.open_tool_calls.0.iter().filter(|call| {
        call.approval_id.as_deref() == Some(attention_id) && Some(&call.turn_id) == turn
    });
    match (calls.next(), calls.next()) {
        (Some(call), None) => operation(&call.call_id),
        _ => Err(crate::cli::permission::refusal(
            "permission-request-uncorrelated",
        )),
    }
}

/// Public attention summarizes only its first entry. Any approval or overflow
/// in the current runtime must refuse uncorrelated terminal input.
pub(crate) fn has_permission(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<bool, CliError> {
    // The caller holds the record lock and exclusive ingress probe: permission
    // writers cannot enter. Snapshots are atomically replaced, so reading here
    // needs no nested activity lock (native conversation commands hold it).
    let dir = session_dir(context, &record.id);
    let path = dir.join(ACTIVITY_FILE);
    if !path.exists() {
        return Ok(false);
    }
    let document = read_document(&path)?;
    if record.runtime.as_ref().is_none_or(|r| {
        r.launch_id != document.runtime_id || r.generation != document.runtime_generation
    }) {
        return Ok(false);
    }
    Ok(document
        .pending_attention
        .iter()
        .any(|p| p.kind == "approval")
        || document.overflow_attention.is_some())
}
