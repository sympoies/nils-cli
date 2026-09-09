//! Incremental semantic memory for transcript-size-independent session retitling.
//!
//! The marker is private daemon state. It retains small, sanitized semantic
//! facts and integrity hashes, never raw provider frames or model prompts.

#[cfg(test)]
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::provider_history::{
    HistoryCatalog, HistoryError, HistoryMessage, IncrementalHistoryCursor,
};
use crate::{
    CliContext, CliError, SessionRecord, SessionTitleState, SessionTitleTopicSource,
    canonicalize_structured_title_pair, load_session_record, lock_exact_session_authority,
    write_session_document,
};

pub(crate) const CAPABILITY: &str = "agent-session.session-retitle.v3";
pub(crate) const REQUEST_SCHEMA: &str = "agent-session.session-retitle.request.v3";
pub(crate) const RESPONSE_SCHEMA: &str = "agent-session.session-retitle.v3";
pub(crate) const READINESS_SCHEMA: &str = "agent-session.session-retitle.readiness.v3";
const MARKER_KEY: &str = "session_retitle_v3";
const MARKER_SCHEMA: &str = "agent-session.session-retitle-state.v3";
const MAX_MARKER_BYTES: usize = 16 * 1024;
pub(crate) const MAX_PROVIDER_INPUT_BYTES: usize = 16 * 1024;
const MAX_SEMANTIC_PROJECTION_BYTES: usize = 12 * 1024;
pub(crate) const REFRESH_CHUNK_BYTES: usize = 1024 * 1024 + 1;
const MAX_TEXT_CHARS: usize = 320;
const MAX_ACTIVITY_CHARS: usize = 320;
const MAX_LEDGER_ENTRIES: usize = 6;
const MAX_SEGMENTS: usize = 8;
const MAX_RECEIPTS: usize = 8;
const MAX_APPLIED_MESSAGE_IDS: usize = 48;
const MAX_PROVIDER_ATTEMPTS: usize = 2;
const PROVIDER_CLAIM_TTL_SECONDS: i64 = 150;

fn default_duration_bucket() -> String {
    "under_10_ms".to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryReadiness {
    Ready,
    CatchingUp,
    Stale,
    Degraded,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct MemoryFact {
    turn_id: String,
    text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct LedgerEntry {
    kind: String,
    turn_id: String,
    text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct SegmentReceipt {
    segment_id: String,
    source_id: String,
    #[serde(default)]
    discontinuity: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ExecutionClaim {
    token_hash: String,
    generation: u8,
    acquired_at: String,
    expires_at_second: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct OperationReceipt {
    pub(crate) operation_hash: String,
    idempotency_hash: String,
    #[serde(default)]
    pub(crate) trigger: String,
    pub(crate) state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    changed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) failure_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    provider_attempts: Vec<crate::retitle::ProviderAttemptObservation>,
    #[serde(default)]
    admitted_incarnation: Option<String>,
    #[serde(default)]
    admitted_title_revision: u64,
    #[serde(default)]
    admitted_memory_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    activity_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_turn_id_hash: Option<String>,
    #[serde(default)]
    result_incarnation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result_title_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result_memory_revision: Option<u64>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default = "default_duration_bucket")]
    duration_bucket: String,
    #[serde(default)]
    attempt_generation: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    execution_claim: Option<ExecutionClaim>,
    memory_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct SemanticMemory {
    schema_version: String,
    revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<MemoryFact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_objective: Option<MemoryFact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_activity: Option<MemoryFact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    milestones: Vec<LedgerEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    decisions: Vec<LedgerEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    blockers: Vec<LedgerEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    journey: Vec<LedgerEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    segments: Vec<SegmentReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<IncrementalHistoryCursor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_delta_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    applied_message_ids: Vec<String>,
    readiness: MemoryReadiness,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    receipts: Vec<OperationReceipt>,
}

impl Default for SemanticMemory {
    fn default() -> Self {
        Self {
            schema_version: MARKER_SCHEMA.to_string(),
            revision: 0,
            origin: None,
            active_objective: None,
            current_activity: None,
            milestones: Vec::new(),
            decisions: Vec::new(),
            blockers: Vec::new(),
            journey: Vec::new(),
            segments: Vec::new(),
            cursor: None,
            last_turn_id: None,
            last_delta_hash: None,
            applied_message_ids: Vec::new(),
            readiness: MemoryReadiness::CatchingUp,
            receipts: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetitleV3Request {
    pub(crate) schema_version: String,
    pub(crate) trigger: String,
    pub(crate) idempotency_key: String,
    pub(crate) expected: RetitleV3FenceInput,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetitleV3FenceInput {
    pub(crate) session_incarnation: String,
    pub(crate) title_revision: u64,
    pub(crate) memory_revision: u64,
    #[serde(default)]
    pub(crate) activity_revision: Option<u64>,
    #[serde(default)]
    pub(crate) provider_turn_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RetitleV3MachineReadiness {
    schema_version: &'static str,
    capability: &'static str,
    pub(crate) status: MemoryReadiness,
    reason_code: &'static str,
    next_action: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RetitleV3Readiness {
    schema_version: &'static str,
    capability: &'static str,
    pub(crate) status: MemoryReadiness,
    provider_status: &'static str,
    context_status: MemoryReadiness,
    title_status: &'static str,
    reason_code: &'static str,
    next_action: &'static str,
    memory_revision: u64,
    usable_memory: bool,
    pending_operation: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor_fence_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RetitleV3ResultFence {
    session_incarnation: Option<String>,
    title_revision: u64,
    memory_revision: u64,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RetitleV3Response {
    schema_version: &'static str,
    capability: &'static str,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) outcome: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) changed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_stage: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) provider_attempts: Vec<crate::retitle::ProviderAttemptObservation>,
    pub(crate) operation_hash: String,
    pub(crate) memory_revision: u64,
    pub(crate) title_revision: u64,
    pub(crate) session_incarnation: Option<String>,
    pub(crate) readiness: MemoryReadiness,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) title: Option<String>,
    pub(crate) admission_fence: RetitleV3ResultFence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result_fence: Option<RetitleV3ResultFence>,
    pub(crate) current_fence: RetitleV3ResultFence,
    pub(crate) result_is_current: bool,
    pub(crate) started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at: Option<String>,
    pub(crate) duration_bucket: String,
}

#[derive(Clone, Debug)]
pub(crate) struct InferenceContext {
    pub(crate) input: String,
    pub(crate) existing: Option<SessionTitleState>,
    memory_revision: u64,
    title_revision: u64,
    incarnation: Option<String>,
    cursor: Option<IncrementalHistoryCursor>,
    last_turn_id: Option<String>,
    last_delta_hash: Option<String>,
    activity_revision: Option<u64>,
    provider_turn_id_hash: Option<String>,
    execution_claim_hash: Option<String>,
    attempt_generation: u8,
    pub(crate) trigger: String,
    operation_hash: String,
}

pub(crate) enum ProviderClaim {
    Claimed(InferenceContext),
    Waiting(RetitleV3Response),
    RecoveredTerminal(RetitleV3Response),
}

#[derive(Clone, Debug)]
struct MemoryFence {
    created_at: String,
    incarnation: Option<String>,
    title_revision: u64,
    memory_revision: u64,
    last_turn_id: Option<String>,
    segment_id: Option<String>,
    cursor: Option<IncrementalHistoryCursor>,
    previous_delta_hash: Option<String>,
    applied_message_ids: Vec<String>,
    delta_hash: String,
    idempotency_hash: String,
}

pub(crate) fn global_readiness(provider_available: bool) -> RetitleV3MachineReadiness {
    RetitleV3MachineReadiness {
        schema_version: READINESS_SCHEMA,
        capability: CAPABILITY,
        status: if provider_available {
            MemoryReadiness::Ready
        } else {
            MemoryReadiness::Degraded
        },
        reason_code: if provider_available {
            "ready"
        } else {
            "provider_unavailable_memory_supported"
        },
        next_action: if provider_available {
            "none"
        } else {
            "use_cached_memory_or_restore_provider"
        },
    }
}

pub(crate) fn session_readiness(
    context: &CliContext,
    catalog: &HistoryCatalog,
    id: &str,
    provider_available: bool,
) -> Result<RetitleV3Readiness, CliError> {
    let record = load_session_record(context, id)?;
    let memory = memory_from_record(&record)?;
    let usable = usable_memory(&memory);
    let (status, reason_code) = if record.provider_resume.is_none() {
        (MemoryReadiness::Unavailable, "provider_history_unavailable")
    } else if memory.revision == 0 {
        (MemoryReadiness::CatchingUp, "memory_not_initialized")
    } else {
        let Some(resume) = record.provider_resume.as_ref() else {
            unreachable!("provider_resume absence handled above")
        };
        let probed = catalog.incremental_messages_for_provider_session(
            &resume.provider,
            &resume.session_id,
            memory.cursor.as_ref(),
            &memory.applied_message_ids,
            1,
        );
        match probed {
            Ok(page) if page.discontinuity || !page.caught_up => {
                return Ok(RetitleV3Readiness {
                    schema_version: READINESS_SCHEMA,
                    capability: CAPABILITY,
                    status: MemoryReadiness::Stale,
                    provider_status: if provider_available {
                        "ready"
                    } else {
                        "unavailable"
                    },
                    context_status: MemoryReadiness::Stale,
                    title_status: title_status(&record, &memory),
                    reason_code: "history_advanced",
                    next_action: "refresh_memory",
                    memory_revision: memory.revision,
                    usable_memory: usable,
                    pending_operation: has_pending_operation(&memory),
                    cursor_fence_hash: cursor_fence_hash(memory.cursor.as_ref()),
                });
            }
            Err(HistoryError::NotFound) => {
                return Ok(RetitleV3Readiness {
                    schema_version: READINESS_SCHEMA,
                    capability: CAPABILITY,
                    status: MemoryReadiness::Unavailable,
                    provider_status: if provider_available {
                        "ready"
                    } else {
                        "unavailable"
                    },
                    context_status: MemoryReadiness::Unavailable,
                    title_status: title_status(&record, &memory),
                    reason_code: "provider_history_unavailable",
                    next_action: "restore_provider_history",
                    memory_revision: memory.revision,
                    usable_memory: usable,
                    pending_operation: has_pending_operation(&memory),
                    cursor_fence_hash: cursor_fence_hash(memory.cursor.as_ref()),
                });
            }
            Err(_) => {
                return Ok(RetitleV3Readiness {
                    schema_version: READINESS_SCHEMA,
                    capability: CAPABILITY,
                    status: if usable {
                        MemoryReadiness::Degraded
                    } else {
                        MemoryReadiness::Unavailable
                    },
                    provider_status: if provider_available {
                        "ready"
                    } else {
                        "unavailable"
                    },
                    context_status: if usable {
                        MemoryReadiness::Degraded
                    } else {
                        MemoryReadiness::Unavailable
                    },
                    title_status: title_status(&record, &memory),
                    reason_code: if usable {
                        "history_read_degraded"
                    } else {
                        "memory_unavailable"
                    },
                    next_action: if usable {
                        "use_cached_memory_or_retry"
                    } else {
                        "restore_provider_history"
                    },
                    memory_revision: memory.revision,
                    usable_memory: usable,
                    pending_operation: has_pending_operation(&memory),
                    cursor_fence_hash: cursor_fence_hash(memory.cursor.as_ref()),
                });
            }
            Ok(_) => {}
        }
        let reason = match memory.readiness {
            MemoryReadiness::Ready => "ready",
            MemoryReadiness::CatchingUp => "history_catching_up",
            MemoryReadiness::Stale => "history_stale",
            MemoryReadiness::Degraded => "degraded_cached",
            MemoryReadiness::Unavailable => "memory_unavailable",
        };
        (memory.readiness.clone(), reason)
    };
    let (status, reason_code) = if !provider_available && usable {
        (MemoryReadiness::Degraded, "degraded_cached")
    } else {
        (status, reason_code)
    };
    Ok(RetitleV3Readiness {
        schema_version: READINESS_SCHEMA,
        capability: CAPABILITY,
        status: status.clone(),
        provider_status: if provider_available {
            "ready"
        } else {
            "unavailable"
        },
        context_status: memory.readiness.clone(),
        title_status: title_status(&record, &memory),
        reason_code,
        next_action: match reason_code {
            "ready" => "none",
            "history_catching_up" | "history_stale" | "memory_not_initialized" => "refresh_memory",
            "degraded_cached" => "use_cached_memory_or_retry",
            _ => "restore_provider_history",
        },
        memory_revision: memory.revision,
        usable_memory: usable,
        pending_operation: has_pending_operation(&memory),
        cursor_fence_hash: cursor_fence_hash(memory.cursor.as_ref()),
    })
}

pub(crate) fn operation_response(
    context: &CliContext,
    id: &str,
    operation_hash: &str,
) -> Result<Option<RetitleV3Response>, CliError> {
    let record = load_session_record(context, id)?;
    let memory = memory_from_record(&record)?;
    Ok(memory
        .receipts
        .iter()
        .find(|receipt| receipt.operation_hash == operation_hash)
        .map(|receipt| response_from_receipt(&record, &memory, receipt)))
}

pub(crate) fn refresh_once(
    context: &CliContext,
    catalog: &HistoryCatalog,
    id: &str,
    request: &RetitleV3Request,
) -> Result<RetitleV3Response, CliError> {
    validate_request(request)?;
    let operation_hash = request_operation_hash(id, request);
    let idempotency_hash = request_fingerprint(request);
    refresh_admitted_once(
        context,
        catalog,
        id,
        &operation_hash,
        &idempotency_hash,
        &request.trigger,
        Some(&request.expected),
    )
}

/// Advances a previously admitted operation without requiring the original
/// bearer request. Only content-free receipt fields are used, so polling can
/// durably adopt pending work after a daemon restart.
pub(crate) fn refresh_operation_once(
    context: &CliContext,
    catalog: &HistoryCatalog,
    id: &str,
    operation_hash: &str,
) -> Result<RetitleV3Response, CliError> {
    if !valid_operation_hash(operation_hash) {
        return Err(CliError::usage(
            "invalid-retitle-v3-operation-hash",
            "retitle v3 operation hash is invalid",
            Some(error_details(false, "fix_request", "correct_request")),
        ));
    }
    let record = load_session_record(context, id)?;
    let memory = memory_from_record(&record)?;
    let receipt = memory
        .receipts
        .iter()
        .find(|receipt| receipt.operation_hash == operation_hash)
        .ok_or_else(operation_not_found)?;
    if is_terminal_receipt(receipt) {
        return Ok(response_from_receipt(&record, &memory, receipt));
    }
    refresh_admitted_once(
        context,
        catalog,
        id,
        operation_hash,
        &receipt.idempotency_hash,
        &receipt.trigger,
        None,
    )
}

fn refresh_admitted_once(
    context: &CliContext,
    catalog: &HistoryCatalog,
    id: &str,
    operation_hash: &str,
    idempotency_hash: &str,
    trigger: &str,
    expected: Option<&RetitleV3FenceInput>,
) -> Result<RetitleV3Response, CliError> {
    let observed = load_session_record(context, id)?;
    let mut memory = memory_from_record(&observed)?;
    if let Some(receipt) = memory
        .receipts
        .iter()
        .find(|receipt| receipt.operation_hash == operation_hash)
    {
        if receipt.idempotency_hash != idempotency_hash {
            return Err(v3_error(
                "retitle-v3-idempotency-conflict",
                "retitle v3 idempotency key was reused with different input",
            ));
        }
        if is_terminal_receipt(receipt) {
            return Ok(response_from_receipt(&observed, &memory, receipt));
        }
        if receipt.execution_claim.is_some() {
            return Ok(response_from_receipt(&observed, &memory, receipt));
        }
    } else if let Some(expected) = expected {
        // Expected values fence admission. Durable continuations are fenced by
        // the receipt plus the memory/source CAS and must survive revision bumps.
        validate_request_fence(context, &observed, expected)?;
    } else {
        return Err(operation_not_found());
    }
    let resume = observed.provider_resume.as_ref().ok_or_else(|| {
        v3_error(
            "retitle-v3-history-unavailable",
            "provider history is unavailable for this session",
        )
    })?;
    let page = catalog
        .incremental_messages_for_provider_session(
            &resume.provider,
            &resume.session_id,
            memory.cursor.as_ref(),
            &memory.applied_message_ids,
            REFRESH_CHUNK_BYTES,
        )
        .map_err(history_error)?;
    let prior_cursor = memory.cursor.clone();
    let made_history_progress = page.discontinuity
        || prior_cursor.as_ref() != Some(&page.cursor)
        || !page.messages.is_empty();
    if !made_history_progress
        && let Some(receipt) = memory.receipts.iter().find(|receipt| {
            receipt.operation_hash == operation_hash
                && matches!(receipt.state.as_str(), "pending" | "ready")
        })
    {
        // A provider may have an incomplete JSONL tail. Do not spin, rewrite
        // the same cursor, or advance past that partial record.
        return Ok(response_from_receipt(&observed, &memory, receipt));
    }
    let delta_hash = delta_hash(&page.messages, &page.cursor);
    let fence = MemoryFence {
        created_at: observed.created_at.clone(),
        incarnation: incarnation(&observed),
        title_revision: observed.title_revision,
        memory_revision: memory.revision,
        last_turn_id: memory.last_turn_id.clone(),
        segment_id: memory
            .cursor
            .as_ref()
            .map(|cursor| cursor.segment_id.clone()),
        cursor: memory.cursor.clone(),
        previous_delta_hash: memory.last_delta_hash.clone(),
        applied_message_ids: memory.applied_message_ids.clone(),
        delta_hash: delta_hash.clone(),
        idempotency_hash: idempotency_hash.to_string(),
    };
    if page.discontinuity {
        push_bounded(
            &mut memory.segments,
            SegmentReceipt {
                segment_id: page.cursor.segment_id.clone(),
                source_id: page.cursor.source_id.clone(),
                discontinuity: true,
            },
            MAX_SEGMENTS,
        );
    } else if memory.segments.is_empty() {
        memory.segments.push(SegmentReceipt {
            segment_id: page.cursor.segment_id.clone(),
            source_id: page.cursor.source_id.clone(),
            discontinuity: false,
        });
    }
    let previous_readiness = memory.readiness.clone();
    let had_semantic_delta = !page.messages.is_empty();
    reduce_messages(&mut memory, &page.messages);
    memory.cursor = Some(page.cursor);
    memory.last_delta_hash = Some(delta_hash);
    memory.readiness =
        if page.caught_up && !had_semantic_delta && previous_readiness == MemoryReadiness::Degraded
        {
            MemoryReadiness::Degraded
        } else if page.caught_up {
            MemoryReadiness::Ready
        } else {
            MemoryReadiness::CatchingUp
        };
    memory.revision = memory.revision.saturating_add(1);
    let cached_usable = page.caught_up
        && !had_semantic_delta
        && !page.discontinuity
        && usable_memory(&memory)
        && observed.title.is_some();
    let degraded_cached = cached_usable && memory.readiness == MemoryReadiness::Degraded;
    let now = jiff::Timestamp::now().to_string();
    let (activity_revision, provider_turn_id_hash) = activity_fence(context, &observed);
    let next_receipt = OperationReceipt {
        operation_hash: operation_hash.to_string(),
        idempotency_hash: fence.idempotency_hash.clone(),
        trigger: trigger.to_string(),
        state: if cached_usable {
            if degraded_cached {
                "degraded_cached"
            } else {
                "completed"
            }
        } else if page.caught_up {
            "ready"
        } else {
            "pending"
        }
        .to_string(),
        outcome: cached_usable.then(|| {
            if degraded_cached {
                "degraded_cached"
            } else {
                "unchanged"
            }
            .to_string()
        }),
        changed: cached_usable.then_some(false),
        failure_class: None,
        failure_stage: None,
        provider_attempts: Vec::new(),
        admitted_incarnation: incarnation(&observed),
        admitted_title_revision: observed.title_revision,
        admitted_memory_revision: fence.memory_revision,
        activity_revision,
        provider_turn_id_hash,
        result_incarnation: cached_usable.then(|| incarnation(&observed)).flatten(),
        result_title_revision: cached_usable.then_some(observed.title_revision),
        result_memory_revision: cached_usable.then_some(memory.revision),
        created_at: memory
            .receipts
            .iter()
            .find(|receipt| receipt.operation_hash == operation_hash)
            .map(|receipt| receipt.created_at.clone())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| now.clone()),
        updated_at: now,
        duration_bucket: default_duration_bucket(),
        attempt_generation: memory
            .receipts
            .iter()
            .find(|receipt| receipt.operation_hash == operation_hash)
            .map_or(0, |receipt| receipt.attempt_generation),
        execution_claim: memory
            .receipts
            .iter()
            .find(|receipt| receipt.operation_hash == operation_hash)
            .and_then(|receipt| receipt.execution_claim.clone()),
        memory_revision: memory.revision,
    };
    terminalize_superseded_operations(&mut memory, &next_receipt, &observed);
    if let Some(receipt) = memory
        .receipts
        .iter_mut()
        .find(|receipt| receipt.operation_hash == operation_hash)
    {
        *receipt = next_receipt;
    } else {
        push_receipt_bounded(&mut memory.receipts, next_receipt)?;
    }
    commit_memory(context, id, fence, memory.clone())?;
    let current = load_session_record(context, id)?;
    let current_memory = memory_from_record(&current)?;
    let receipt = current_memory
        .receipts
        .iter()
        .find(|receipt| receipt.operation_hash == operation_hash)
        .expect("committed receipt exists");
    Ok(response_from_receipt(&current, &current_memory, receipt))
}

pub(crate) fn inference_context(
    context: &CliContext,
    id: &str,
    operation_hash: &str,
) -> Result<InferenceContext, CliError> {
    let record = load_session_record(context, id)?;
    let memory = memory_from_record(&record)?;
    let receipt_ready = memory
        .receipts
        .iter()
        .any(|receipt| receipt.operation_hash == operation_hash && receipt.state == "ready");
    if memory.readiness != MemoryReadiness::Ready || !receipt_ready || !usable_memory(&memory) {
        return Err(v3_error(
            "retitle-v3-memory-not-ready",
            "semantic memory is not ready for provider evaluation",
        ));
    }
    let receipt = memory
        .receipts
        .iter()
        .find(|receipt| receipt.operation_hash == operation_hash)
        .expect("ready receipt exists");
    if selected_operation(&memory).map(|selected| selected.operation_hash.as_str())
        != Some(operation_hash)
    {
        return Err(v3_error(
            "retitle-v3-operation-queued",
            "retitle v3 operation is durably queued behind higher-priority work",
        ));
    }
    Ok(InferenceContext {
        input: render_provider_input(&memory)?,
        existing: record.title_state.clone(),
        memory_revision: memory.revision,
        title_revision: record.title_revision,
        incarnation: incarnation(&record),
        cursor: memory.cursor.clone(),
        last_turn_id: memory.last_turn_id.clone(),
        last_delta_hash: memory.last_delta_hash.clone(),
        activity_revision: receipt.activity_revision,
        provider_turn_id_hash: receipt.provider_turn_id_hash.clone(),
        execution_claim_hash: receipt
            .execution_claim
            .as_ref()
            .map(|claim| claim.token_hash.clone()),
        attempt_generation: receipt.attempt_generation,
        trigger: receipt.trigger.clone(),
        operation_hash: operation_hash.to_string(),
    })
}

pub(crate) fn claim_provider_inference(
    context: &CliContext,
    id: &str,
    operation_hash: &str,
    claim_token: &str,
) -> Result<ProviderClaim, CliError> {
    let mut authority = lock_exact_session_authority(context, id)?
        .ok_or_else(|| v3_error("session-not-found", "session does not exist"))?;
    let mut memory = memory_from_record(&authority.record)?;
    let selected = selected_operation(&memory).map(|receipt| receipt.operation_hash.clone());
    let index = memory
        .receipts
        .iter()
        .position(|receipt| receipt.operation_hash == operation_hash)
        .ok_or_else(operation_not_found)?;
    if is_terminal_receipt(&memory.receipts[index]) {
        return Ok(ProviderClaim::RecoveredTerminal(response_from_receipt(
            &authority.record,
            &memory,
            &memory.receipts[index],
        )));
    }
    if selected.as_deref() != Some(operation_hash) || memory.receipts[index].state != "ready" {
        return Ok(ProviderClaim::Waiting(response_from_receipt(
            &authority.record,
            &memory,
            &memory.receipts[index],
        )));
    }
    let now_second = jiff::Timestamp::now().as_second();
    if let Some(claim) = memory.receipts[index].execution_claim.as_ref() {
        if claim.expires_at_second > now_second {
            return Ok(ProviderClaim::Waiting(response_from_receipt(
                &authority.record,
                &memory,
                &memory.receipts[index],
            )));
        }
        // A crashed provider boundary is uncertain. Never replay an unchanged
        // non-transient input; terminalize it and let a later authoritative
        // turn admit a distinct operation.
        let usable_cached = usable_memory(&memory) && authority.record.title.is_some();
        let receipt = &mut memory.receipts[index];
        receipt.state = if usable_cached {
            "degraded_cached"
        } else {
            "failed"
        }
        .to_string();
        receipt.outcome = Some(
            if usable_cached {
                "degraded_cached"
            } else {
                "terminal_failure"
            }
            .to_string(),
        );
        receipt.changed = Some(false);
        receipt.failure_class = Some("uncertain_execution".to_string());
        receipt.failure_stage = Some("provider_worker".to_string());
        receipt.execution_claim = None;
        receipt.result_incarnation = incarnation(&authority.record);
        receipt.result_title_revision = Some(authority.record.title_revision);
        receipt.result_memory_revision = Some(memory.revision);
        receipt.updated_at = jiff::Timestamp::now().to_string();
        memory.readiness = if usable_cached {
            MemoryReadiness::Degraded
        } else {
            MemoryReadiness::Unavailable
        };
        compact_memory(&mut memory);
        store_memory(&mut authority.record, &memory)?;
        authority.record.updated_at = jiff::Timestamp::now().to_string();
        write_session_document(context, &authority.record)?;
        return Ok(ProviderClaim::RecoveredTerminal(response_from_receipt(
            &authority.record,
            &memory,
            &memory.receipts[index],
        )));
    }
    let token_hash = hash_value(claim_token);
    let receipt = &mut memory.receipts[index];
    receipt.attempt_generation = receipt.attempt_generation.saturating_add(1);
    receipt.execution_claim = Some(ExecutionClaim {
        token_hash,
        generation: receipt.attempt_generation,
        acquired_at: jiff::Timestamp::now().to_string(),
        expires_at_second: now_second.saturating_add(PROVIDER_CLAIM_TTL_SECONDS),
    });
    receipt.updated_at = jiff::Timestamp::now().to_string();
    store_memory(&mut authority.record, &memory)?;
    authority.record.updated_at = jiff::Timestamp::now().to_string();
    write_session_document(context, &authority.record)?;
    drop(authority);
    inference_context(context, id, operation_hash).map(ProviderClaim::Claimed)
}

/// Completes a ready manual operation from accepted semantic memory without a
/// provider call. This is the normal long-session path, including the first
/// title produced after catch-up.
pub(crate) fn complete_manual_locally(
    context: &CliContext,
    catalog: &HistoryCatalog,
    id: &str,
    operation_hash: &str,
) -> Result<Option<RetitleV3Response>, CliError> {
    let inference = inference_context(context, id, operation_hash)?;
    let record = load_session_record(context, id)?;
    if inference.trigger != "manual" && record.title.is_some() {
        return Ok(None);
    }
    let memory = memory_from_record(&record)?;
    let Some(objective) = memory.active_objective.as_ref().or(memory.origin.as_ref()) else {
        return Ok(None);
    };
    let state = SessionTitleState {
        topic: Some(objective.text.clone()),
        topic_source: SessionTitleTopicSource::Auto,
        references: Vec::new(),
        // Activity is private semantic context. A deterministic local title
        // must not turn assistant/provider prose into an HTTP-visible field.
        activity: None,
        extra: std::collections::BTreeMap::new(),
    };
    commit_inference(context, catalog, id, &inference, state, &[]).map(Some)
}

pub(crate) fn commit_inference(
    context: &CliContext,
    catalog: &HistoryCatalog,
    id: &str,
    inference: &InferenceContext,
    state: SessionTitleState,
    provider_attempts: &[crate::retitle::ProviderAttemptObservation],
) -> Result<RetitleV3Response, CliError> {
    let (title, state) = canonicalize_structured_title_pair(None, false, state)?;
    let state = state.expect("inferred title state is structured");
    let mut authority = lock_exact_session_authority(context, id)?
        .ok_or_else(|| v3_error("session-not-found", "session does not exist"))?;
    let mut memory = memory_from_record(&authority.record)?;
    if memory.revision != inference.memory_revision
        || authority.record.title_revision != inference.title_revision
        || incarnation(&authority.record) != inference.incarnation
        || memory.cursor != inference.cursor
        || memory.last_turn_id != inference.last_turn_id
        || memory.last_delta_hash != inference.last_delta_hash
        || !memory.receipts.iter().any(|receipt| {
            receipt.operation_hash == inference.operation_hash
                && receipt.state == "ready"
                && receipt.attempt_generation == inference.attempt_generation
                && receipt
                    .execution_claim
                    .as_ref()
                    .map(|claim| claim.token_hash.as_str())
                    == inference.execution_claim_hash.as_deref()
        })
    {
        return Err(v3_error(
            "retitle-v3-state-conflict",
            "session state changed before retitle v3 provider commit",
        ));
    }
    validate_history_fence(catalog, &authority.record, &memory)?;
    validate_activity_fence(context, &authority.record, inference)?;
    let changed =
        authority.record.title != title || authority.record.title_state.as_ref() != Some(&state);
    if changed {
        authority.record.title = title;
        authority.record.title_state = Some(state);
        authority.record.title_revision = authority
            .record
            .title_revision
            .checked_add(1)
            .ok_or_else(|| v3_error("title-revision-overflow", "title revision overflow"))?;
    }
    if let Some(receipt) = memory
        .receipts
        .iter_mut()
        .find(|receipt| receipt.operation_hash == inference.operation_hash)
    {
        receipt.state = "completed".to_string();
        receipt.outcome = Some(if changed { "committed" } else { "unchanged" }.to_string());
        receipt.changed = Some(changed);
        receipt.provider_attempts = provider_attempts
            .iter()
            .take(MAX_PROVIDER_ATTEMPTS)
            .cloned()
            .collect();
        receipt.duration_bucket = provider_attempts
            .last()
            .map(|attempt| attempt.duration_bucket.clone())
            .unwrap_or_else(default_duration_bucket);
        receipt.result_incarnation = incarnation(&authority.record);
        receipt.result_title_revision = Some(authority.record.title_revision);
        receipt.result_memory_revision = Some(memory.revision);
        receipt.execution_claim = None;
        receipt.updated_at = jiff::Timestamp::now().to_string();
        receipt.memory_revision = memory.revision;
    }
    compact_memory(&mut memory);
    store_memory(&mut authority.record, &memory)?;
    authority.record.updated_at = jiff::Timestamp::now().to_string();
    write_session_document(context, &authority.record)?;
    let receipt = memory
        .receipts
        .iter()
        .find(|receipt| receipt.operation_hash == inference.operation_hash)
        .expect("inference receipt exists");
    Ok(response_from_receipt(&authority.record, &memory, receipt))
}

pub(crate) fn complete_provider_failure(
    context: &CliContext,
    catalog: &HistoryCatalog,
    id: &str,
    inference: &InferenceContext,
    failure_class: &str,
    failure_stage: &str,
    provider_attempts: &[crate::retitle::ProviderAttemptObservation],
) -> Result<RetitleV3Response, CliError> {
    let mut authority = lock_exact_session_authority(context, id)?
        .ok_or_else(|| v3_error("session-not-found", "session does not exist"))?;
    let mut memory = memory_from_record(&authority.record)?;
    if memory.revision != inference.memory_revision
        || authority.record.title_revision != inference.title_revision
        || incarnation(&authority.record) != inference.incarnation
        || memory.cursor != inference.cursor
        || memory.last_turn_id != inference.last_turn_id
        || memory.last_delta_hash != inference.last_delta_hash
        || !memory.receipts.iter().any(|receipt| {
            receipt.operation_hash == inference.operation_hash
                && receipt.state == "ready"
                && receipt.attempt_generation == inference.attempt_generation
                && receipt
                    .execution_claim
                    .as_ref()
                    .map(|claim| claim.token_hash.as_str())
                    == inference.execution_claim_hash.as_deref()
        })
    {
        return Err(v3_error(
            "retitle-v3-state-conflict",
            "session state changed before retitle v3 failure commit",
        ));
    }
    validate_history_fence(catalog, &authority.record, &memory)?;
    validate_activity_fence(context, &authority.record, inference)?;
    let usable = usable_memory(&memory) && authority.record.title.is_some();
    if let Some(receipt) = memory
        .receipts
        .iter_mut()
        .find(|receipt| receipt.operation_hash == inference.operation_hash)
    {
        receipt.state = if usable { "degraded_cached" } else { "failed" }.to_string();
        receipt.outcome = Some(
            if usable {
                "degraded_cached"
            } else {
                "terminal_failure"
            }
            .to_string(),
        );
        receipt.changed = Some(false);
        receipt.failure_class = Some(truncate_chars(failure_class, 96));
        receipt.failure_stage = Some(truncate_chars(failure_stage, 64));
        receipt.provider_attempts = provider_attempts
            .iter()
            .take(MAX_PROVIDER_ATTEMPTS)
            .cloned()
            .collect();
        receipt.duration_bucket = provider_attempts
            .last()
            .map(|attempt| attempt.duration_bucket.clone())
            .unwrap_or_else(default_duration_bucket);
        receipt.result_incarnation = incarnation(&authority.record);
        receipt.result_title_revision = Some(authority.record.title_revision);
        receipt.result_memory_revision = Some(memory.revision);
        receipt.execution_claim = None;
        receipt.updated_at = jiff::Timestamp::now().to_string();
    }
    memory.readiness = if usable {
        MemoryReadiness::Degraded
    } else {
        MemoryReadiness::Unavailable
    };
    compact_memory(&mut memory);
    store_memory(&mut authority.record, &memory)?;
    authority.record.updated_at = jiff::Timestamp::now().to_string();
    write_session_document(context, &authority.record)?;
    let receipt = memory
        .receipts
        .iter()
        .find(|receipt| receipt.operation_hash == inference.operation_hash)
        .expect("failure receipt exists");
    Ok(response_from_receipt(&authority.record, &memory, receipt))
}

/// Terminalizes an inference whose admission fence became stale while the
/// provider was running. The execution claim is the authority for this write:
/// title/activity/history fences are intentionally observed, not revalidated,
/// because their advancement is the terminal condition being recorded.
pub(crate) fn complete_inference_conflict(
    context: &CliContext,
    id: &str,
    inference: &InferenceContext,
    failure_class: &str,
    provider_attempts: &[crate::retitle::ProviderAttemptObservation],
) -> Result<RetitleV3Response, CliError> {
    let mut authority = lock_exact_session_authority(context, id)?
        .ok_or_else(|| v3_error("session-not-found", "session does not exist"))?;
    let mut memory = memory_from_record(&authority.record)?;
    let receipt = memory
        .receipts
        .iter_mut()
        .find(|receipt| {
            receipt.operation_hash == inference.operation_hash
                && receipt.state == "ready"
                && receipt.attempt_generation == inference.attempt_generation
                && receipt
                    .execution_claim
                    .as_ref()
                    .map(|claim| claim.token_hash.as_str())
                    == inference.execution_claim_hash.as_deref()
        })
        .ok_or_else(|| {
            v3_error(
                "retitle-v3-state-conflict",
                "retitle v3 execution claim changed before conflict observation",
            )
        })?;
    receipt.state = "failed".to_string();
    receipt.outcome = Some("terminal_failure".to_string());
    receipt.changed = Some(false);
    receipt.failure_class = Some(truncate_chars(failure_class, 96));
    receipt.failure_stage = Some("commit".to_string());
    receipt.provider_attempts = provider_attempts
        .iter()
        .take(MAX_PROVIDER_ATTEMPTS)
        .cloned()
        .collect();
    receipt.duration_bucket = provider_attempts
        .last()
        .map(|attempt| attempt.duration_bucket.clone())
        .unwrap_or_else(default_duration_bucket);
    receipt.result_incarnation = incarnation(&authority.record);
    receipt.result_title_revision = Some(authority.record.title_revision);
    receipt.result_memory_revision = Some(memory.revision);
    receipt.execution_claim = None;
    receipt.updated_at = jiff::Timestamp::now().to_string();
    compact_memory(&mut memory);
    store_memory(&mut authority.record, &memory)?;
    authority.record.updated_at = jiff::Timestamp::now().to_string();
    write_session_document(context, &authority.record)?;
    let receipt = memory
        .receipts
        .iter()
        .find(|receipt| receipt.operation_hash == inference.operation_hash)
        .expect("conflict receipt exists");
    Ok(response_from_receipt(&authority.record, &memory, receipt))
}

fn validate_request(request: &RetitleV3Request) -> Result<(), CliError> {
    if request.schema_version != REQUEST_SCHEMA
        || !matches!(request.trigger.as_str(), "manual" | "automatic")
        || !(8..=128).contains(&request.idempotency_key.len())
        || !request
            .idempotency_key
            .bytes()
            .all(|byte| matches!(byte, 0x21..=0x7e))
        || (request.trigger == "manual"
            && (request.expected.activity_revision.is_some()
                || request.expected.provider_turn_id.is_some()))
        || (request.trigger == "automatic"
            && (request.expected.activity_revision.is_none()
                || request
                    .expected
                    .provider_turn_id
                    .as_deref()
                    .is_none_or(|turn| {
                        turn.is_empty()
                            || turn.len() > 128
                            || !turn.bytes().all(|byte| matches!(byte, 0x21..=0x7e))
                    })))
    {
        return Err(CliError::usage(
            "invalid-retitle-v3-request",
            "retitle v3 request is invalid",
            Some(error_details(false, "fix_request", "correct_request")),
        ));
    }
    Ok(())
}

fn validate_request_fence(
    context: &CliContext,
    record: &SessionRecord,
    expected: &RetitleV3FenceInput,
) -> Result<(), CliError> {
    let memory = memory_from_record(record)?;
    if incarnation(record).as_deref() != Some(expected.session_incarnation.as_str())
        || record.title_revision != expected.title_revision
        || memory.revision != expected.memory_revision
    {
        return Err(v3_error(
            "retitle-v3-state-conflict",
            "session state changed before retitle v3 admission",
        ));
    }
    if let (Some(expected_revision), Some(expected_turn)) = (
        expected.activity_revision,
        expected.provider_turn_id.as_deref(),
    ) {
        let (actual_revision, actual_turn) = activity_fence(context, record);
        if actual_revision.is_none_or(|revision| revision < expected_revision)
            || actual_turn.as_deref() != Some(hash_value(expected_turn).as_str())
        {
            return Err(v3_error(
                "retitle-v3-turn-conflict",
                "provider turn changed before retitle v3 admission",
            ));
        }
    }
    Ok(())
}

fn activity_fence(context: &CliContext, record: &SessionRecord) -> (Option<u64>, Option<String>) {
    let Some(state) = crate::activity::state_for_view(context, record) else {
        return (None, None);
    };
    let turn_id = state
        .current_turn
        .as_ref()
        .and_then(|turn| turn.provider_turn_id.as_deref())
        .or_else(|| {
            state
                .last_turn
                .as_ref()
                .and_then(|turn| turn.provider_turn_id.as_deref())
        });
    (Some(state.revision), turn_id.map(hash_value))
}

fn validate_activity_fence(
    context: &CliContext,
    record: &SessionRecord,
    inference: &InferenceContext,
) -> Result<(), CliError> {
    if inference.activity_revision.is_none() && inference.provider_turn_id_hash.is_none() {
        return Ok(());
    }
    let (actual_revision, actual_turn) = activity_fence(context, record);
    if actual_revision.is_none_or(|actual| {
        inference
            .activity_revision
            .is_some_and(|expected| actual < expected)
    }) || actual_turn != inference.provider_turn_id_hash
    {
        return Err(v3_error(
            "retitle-v3-turn-conflict",
            "provider turn changed before retitle v3 completion",
        ));
    }
    Ok(())
}

fn validate_history_fence(
    catalog: &HistoryCatalog,
    record: &SessionRecord,
    memory: &SemanticMemory,
) -> Result<(), CliError> {
    let resume = record.provider_resume.as_ref().ok_or_else(|| {
        v3_error(
            "retitle-v3-history-unavailable",
            "provider history is unavailable for this session",
        )
    })?;
    let page = catalog
        .incremental_messages_for_provider_session(
            &resume.provider,
            &resume.session_id,
            memory.cursor.as_ref(),
            &memory.applied_message_ids,
            1,
        )
        .map_err(history_error)?;
    if page.discontinuity
        || !page.caught_up
        || memory.cursor.as_ref() != Some(&page.cursor)
        || !page.messages.is_empty()
    {
        return Err(v3_error(
            "retitle-v3-history-conflict",
            "provider history advanced before retitle v3 completion",
        ));
    }
    Ok(())
}

fn commit_memory(
    context: &CliContext,
    id: &str,
    fence: MemoryFence,
    mut memory: SemanticMemory,
) -> Result<(), CliError> {
    compact_memory(&mut memory);
    let mut authority = lock_exact_session_authority(context, id)?
        .ok_or_else(|| v3_error("session-not-found", "session does not exist"))?;
    let current = memory_from_record(&authority.record)?;
    let actual_segment = current
        .cursor
        .as_ref()
        .map(|cursor| cursor.segment_id.clone());
    if authority.record.created_at != fence.created_at
        || incarnation(&authority.record) != fence.incarnation
        || authority.record.title_revision != fence.title_revision
        || current.revision != fence.memory_revision
        || current.last_turn_id != fence.last_turn_id
        || actual_segment != fence.segment_id
        || current.cursor != fence.cursor
        || current.last_delta_hash != fence.previous_delta_hash
        || current.applied_message_ids != fence.applied_message_ids
        || memory.last_delta_hash.as_deref() != Some(fence.delta_hash.as_str())
        || !memory
            .receipts
            .iter()
            .any(|receipt| receipt.idempotency_hash == fence.idempotency_hash)
    {
        return Err(v3_error(
            "retitle-v3-state-conflict",
            "session state changed before semantic-memory commit",
        ));
    }
    store_memory(&mut authority.record, &memory)?;
    authority.record.updated_at = jiff::Timestamp::now().to_string();
    write_session_document(context, &authority.record)
}

fn reduce_messages(memory: &mut SemanticMemory, messages: &[HistoryMessage]) {
    for message in messages {
        if memory.applied_message_ids.contains(&message.id) {
            continue;
        }
        let Some(text) = sanitize_text(
            &message.text,
            if message.human_prompt {
                MAX_TEXT_CHARS
            } else {
                MAX_ACTIVITY_CHARS
            },
        ) else {
            continue;
        };
        if message.human_prompt && message.role == "user" {
            let projected = semantic_label("objective", &text, MAX_TEXT_CHARS);
            let fact = MemoryFact {
                turn_id: message.id.clone(),
                text: projected.clone(),
            };
            if memory.origin.is_none() {
                memory.origin = Some(fact.clone());
                memory.active_objective = Some(fact.clone());
            } else if explicit_objective_pivot(&text) {
                memory.active_objective = Some(fact.clone());
            }
            push_bounded(
                &mut memory.journey,
                LedgerEntry {
                    kind: "human_objective".to_string(),
                    turn_id: fact.turn_id.clone(),
                    text: projected,
                },
                MAX_LEDGER_ENTRIES,
            );
        } else if message.role == "assistant" {
            let kind = classify_assistant_kind(&text);
            let projected = semantic_label(kind, &text, MAX_ACTIVITY_CHARS);
            let fact = MemoryFact {
                turn_id: message.id.clone(),
                text: projected.clone(),
            };
            memory.current_activity = Some(fact.clone());
            let target = match kind {
                "blocker" => &mut memory.blockers,
                "decision" => &mut memory.decisions,
                _ => &mut memory.milestones,
            };
            push_bounded(
                target,
                LedgerEntry {
                    kind: kind.to_string(),
                    turn_id: fact.turn_id.clone(),
                    text: projected,
                },
                MAX_LEDGER_ENTRIES,
            );
        }
        memory.last_turn_id = Some(message.id.clone());
        push_bounded(
            &mut memory.applied_message_ids,
            message.id.clone(),
            MAX_APPLIED_MESSAGE_IDS,
        );
    }
}

fn explicit_objective_pivot(text: &str) -> bool {
    let normalized = text.trim().to_ascii_lowercase();
    [
        "now ",
        "next ",
        "instead",
        "switch ",
        "change the objective",
        "new objective",
        "new task",
        "please implement",
        "please fix",
        "接下來",
        "現在",
        "改成",
        "換成",
        "新目標",
    ]
    .iter()
    .any(|cue| normalized.starts_with(cue) || normalized.contains(cue))
}

fn classify_assistant_kind(text: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if ["blocked", "blocker", "cannot continue", "waiting for"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        "blocker"
    } else if ["decided", "decision", "will use", "chosen"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        "decision"
    } else {
        "progress"
    }
}

fn semantic_label(kind: &str, value: &str, max_chars: usize) -> String {
    let stop = [
        "a", "an", "and", "are", "as", "at", "be", "been", "being", "but", "by", "can", "continue",
        "could", "do", "for", "from", "has", "have", "i", "in", "is", "it", "just", "now", "of",
        "on", "please", "status", "that", "the", "this", "to", "we", "will", "with", "would",
        "yes", "you", "your", "redacted",
    ];
    let mut concepts = std::collections::BTreeSet::new();
    for token in value.split(|character: char| {
        character.is_whitespace() || (character.is_ascii_punctuation() && character != '#')
    }) {
        let token = token.trim().to_lowercase();
        if token.is_empty()
            || stop.contains(&token.as_str())
            || credential_shaped(&token)
            || credential_assignment(&token)
            || token.contains('@')
            || token.starts_with("http")
            || token.len() > 48
        {
            continue;
        }
        if token.is_ascii() {
            concepts.insert(token);
        } else {
            concepts.extend(token.chars().map(|character| character.to_string()));
        }
    }
    let joined = concepts
        .into_iter()
        .take(20)
        .collect::<Vec<_>>()
        .join(" · ");
    let label = if joined.is_empty() {
        format!("{kind}: session work")
    } else {
        format!("{kind}: {joined}")
    };
    truncate_chars(&label, max_chars)
}

pub(crate) fn render_provider_input(memory: &SemanticMemory) -> Result<String, CliError> {
    let value = json!({
        "schema_version": CAPABILITY,
        "origin": memory.origin,
        "active_objective": memory.active_objective,
        "current_activity": memory.current_activity,
        "milestones": memory.milestones,
        "decisions": memory.decisions,
        "blockers": memory.blockers,
        "journey": memory.journey,
    });
    let rendered = serde_json::to_string(&value).map_err(|_| {
        v3_error(
            "retitle-v3-memory-invalid",
            "semantic memory cannot be rendered",
        )
    })?;
    if rendered.len() >= MAX_SEMANTIC_PROJECTION_BYTES {
        return Err(v3_error(
            "retitle-v3-memory-too-large",
            "semantic memory provider input exceeds its private bound",
        ));
    }
    Ok(rendered)
}

fn sanitize_text(value: &str, max_chars: usize) -> Option<String> {
    let filtered = crate::retitle::filter_text(value);
    let mut words = Vec::new();
    for word in filtered.split_whitespace() {
        let trimmed = word.trim_matches(|character: char| {
            matches!(
                character,
                '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']'
            )
        });
        if credential_shaped(trimmed)
            || trimmed.starts_with('/')
            || trimmed.starts_with("~/")
            || credential_assignment(trimmed)
            || environment_assignment(trimmed)
            || trimmed.to_ascii_lowercase().starts_with("http://")
            || trimmed.to_ascii_lowercase().starts_with("https://")
            || trimmed.contains('@')
        {
            words.push("<redacted>");
        } else {
            words.push(word);
        }
    }
    let normalized = words.join(" ");
    let normalized = normalized.trim();
    if normalized.is_empty() {
        return None;
    }
    Some(truncate_chars(normalized, max_chars))
}

fn environment_assignment(value: &str) -> bool {
    let Some((key, _)) = value.split_once('=') else {
        return false;
    };
    (2..=64).contains(&key.len())
        && key
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn credential_assignment(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    ["token=", "api_key=", "apikey=", "password=", "secret="]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn credential_shaped(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if ["ghp_", "github_pat_", "akia", "sk-", "bearer"]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return true;
    }
    let jwt = value.split('.').count() == 3
        && value.len() >= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    let high_entropy = value.len() >= 40
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
        && value.bytes().any(|byte| byte.is_ascii_lowercase())
        && value.bytes().any(|byte| byte.is_ascii_uppercase())
        && value.bytes().any(|byte| byte.is_ascii_digit());
    jwt || high_entropy
}

fn memory_from_record(record: &SessionRecord) -> Result<SemanticMemory, CliError> {
    let Some(value) = record.extra.get(MARKER_KEY) else {
        return Ok(SemanticMemory::default());
    };
    let memory: SemanticMemory = serde_json::from_value(value.clone()).map_err(|_| {
        v3_error(
            "retitle-v3-memory-invalid",
            "semantic memory marker is invalid",
        )
    })?;
    if memory.schema_version != MARKER_SCHEMA {
        return Err(v3_error(
            "retitle-v3-memory-version-unsupported",
            "semantic memory marker version is unsupported",
        ));
    }
    Ok(memory)
}

pub(crate) fn memory_revision(record: &SessionRecord) -> u64 {
    memory_from_record(record).map_or(0, |memory| memory.revision)
}

pub(crate) fn selected_pending_operation_hash(record: &SessionRecord) -> Option<String> {
    memory_from_record(record).ok().and_then(|memory| {
        selected_operation(&memory).map(|receipt| receipt.operation_hash.clone())
    })
}

fn store_memory(record: &mut SessionRecord, memory: &SemanticMemory) -> Result<(), CliError> {
    let mut memory = memory.clone();
    compact_memory(&mut memory);
    let value = serde_json::to_value(&memory).map_err(|_| {
        v3_error(
            "retitle-v3-memory-invalid",
            "semantic memory cannot be stored",
        )
    })?;
    let bytes = serde_json::to_vec(&value).map_err(|_| {
        v3_error(
            "retitle-v3-memory-invalid",
            "semantic memory cannot be stored",
        )
    })?;
    if bytes.len() > MAX_MARKER_BYTES {
        return Err(v3_error(
            "retitle-v3-memory-too-large",
            "semantic memory marker exceeds its private bound",
        ));
    }
    record.extra.insert(MARKER_KEY.to_string(), value);
    Ok(())
}

fn compact_memory(memory: &mut SemanticMemory) {
    while serde_json::to_vec(&*memory).is_ok_and(|bytes| bytes.len() > MAX_MARKER_BYTES) {
        let largest = [
            (memory.journey.len(), "journey"),
            (memory.milestones.len(), "milestones"),
            (memory.decisions.len(), "decisions"),
            (memory.blockers.len(), "blockers"),
        ]
        .into_iter()
        .max_by_key(|(length, _)| *length);
        match largest {
            Some((length, "journey")) if length > 1 => {
                memory.journey.remove(0);
            }
            Some((length, "milestones")) if length > 1 => {
                memory.milestones.remove(0);
            }
            Some((length, "decisions")) if length > 1 => {
                memory.decisions.remove(0);
            }
            Some((length, "blockers")) if length > 1 => {
                memory.blockers.remove(0);
            }
            _ if memory.receipts.iter().any(is_terminal_receipt) => {
                let index = memory
                    .receipts
                    .iter()
                    .position(is_terminal_receipt)
                    .expect("terminal receipt exists");
                memory.receipts.remove(index);
            }
            _ if memory.segments.len() > 1 => {
                memory.segments.remove(0);
            }
            _ if memory.applied_message_ids.len() > 16 => {
                memory
                    .applied_message_ids
                    .drain(..memory.applied_message_ids.len() - 16);
            }
            _ => {
                for fact in [
                    memory.origin.as_mut(),
                    memory.active_objective.as_mut(),
                    memory.current_activity.as_mut(),
                ]
                .into_iter()
                .flatten()
                {
                    fact.text = truncate_chars(&fact.text, 64);
                }
                for entry in memory
                    .milestones
                    .iter_mut()
                    .chain(memory.decisions.iter_mut())
                    .chain(memory.blockers.iter_mut())
                    .chain(memory.journey.iter_mut())
                {
                    entry.text = truncate_chars(&entry.text, 64);
                }
                for receipt in &mut memory.receipts {
                    receipt.failure_class = receipt
                        .failure_class
                        .as_deref()
                        .map(|value| truncate_chars(value, 48));
                    receipt.failure_stage = receipt
                        .failure_stage
                        .as_deref()
                        .map(|value| truncate_chars(value, 32));
                }
                // Attempt observations are diagnostic and terminal-only;
                // preserve operation outcome/fences before their details.
                for receipt in memory
                    .receipts
                    .iter_mut()
                    .filter(|receipt| is_terminal_receipt(receipt))
                {
                    receipt.provider_attempts.clear();
                }
                break;
            }
        }
    }
}

fn usable_memory(memory: &SemanticMemory) -> bool {
    memory.origin.is_some() || memory.active_objective.is_some()
}

fn has_pending_operation(memory: &SemanticMemory) -> bool {
    memory
        .receipts
        .iter()
        .any(|receipt| matches!(receipt.state.as_str(), "pending" | "ready"))
}

fn selected_operation(memory: &SemanticMemory) -> Option<&OperationReceipt> {
    let pending = |receipt: &&OperationReceipt| !is_terminal_receipt(receipt);
    memory
        .receipts
        .iter()
        .filter(pending)
        .filter(|receipt| receipt.execution_claim.is_some())
        .min_by(|left, right| left.created_at.cmp(&right.created_at))
        .or_else(|| {
            memory
                .receipts
                .iter()
                .filter(pending)
                .filter(|receipt| receipt.trigger == "manual")
                .min_by(|left, right| left.created_at.cmp(&right.created_at))
        })
        .or_else(|| {
            memory
                .receipts
                .iter()
                .filter(pending)
                .filter(|receipt| receipt.trigger == "automatic")
                .max_by(|left, right| left.created_at.cmp(&right.created_at))
        })
}

fn terminalize_superseded_operations(
    memory: &mut SemanticMemory,
    incoming: &OperationReceipt,
    record: &SessionRecord,
) {
    for receipt in &mut memory.receipts {
        if receipt.operation_hash == incoming.operation_hash || is_terminal_receipt(receipt) {
            continue;
        }
        let replace = receipt.execution_claim.is_none()
            && receipt.trigger == "automatic"
            && matches!(incoming.trigger.as_str(), "manual" | "automatic");
        if !replace {
            continue;
        }
        receipt.state = "failed".to_string();
        receipt.outcome = Some("terminal_failure".to_string());
        receipt.changed = Some(false);
        receipt.failure_class = Some("superseded".to_string());
        receipt.failure_stage = Some("provider_admission".to_string());
        receipt.result_incarnation = incarnation(record);
        receipt.result_title_revision = Some(record.title_revision);
        receipt.result_memory_revision = Some(memory.revision);
        receipt.updated_at = jiff::Timestamp::now().to_string();
        receipt.duration_bucket = default_duration_bucket();
    }
}

fn push_receipt_bounded(
    receipts: &mut Vec<OperationReceipt>,
    receipt: OperationReceipt,
) -> Result<(), CliError> {
    if receipts.len() >= MAX_RECEIPTS
        && !is_terminal_receipt(&receipt)
        && !receipts.iter().any(is_terminal_receipt)
    {
        return Err(CliError::unavailable(
            "retitle-v3-operation-capacity",
            "retitle v3 operation capacity is exhausted by active work",
            Some(error_details(
                true,
                "poll_active_operations",
                "retry_after_completion",
            )),
        ));
    }
    receipts.push(receipt);
    while receipts.len() > MAX_RECEIPTS {
        if let Some(index) = receipts.iter().position(is_terminal_receipt) {
            receipts.remove(index);
        } else {
            // Non-terminal work is pinned. The durable manager normally keeps
            // this branch unreachable by superseding queued automatic work.
            break;
        }
    }
    Ok(())
}

fn title_status(record: &SessionRecord, memory: &SemanticMemory) -> &'static str {
    if record.title.is_none() {
        "missing"
    } else if memory.readiness == MemoryReadiness::Degraded {
        "degraded_cached"
    } else if memory.readiness == MemoryReadiness::Ready {
        "current"
    } else {
        "stale"
    }
}

fn cursor_fence_hash(cursor: Option<&IncrementalHistoryCursor>) -> Option<String> {
    cursor.and_then(|cursor| {
        serde_json::to_string(cursor)
            .ok()
            .map(|value| hash_value(&value))
    })
}

fn is_terminal_receipt(receipt: &OperationReceipt) -> bool {
    matches!(
        receipt.state.as_str(),
        "completed" | "degraded_cached" | "failed"
    )
}

fn response_from_receipt(
    record: &SessionRecord,
    memory: &SemanticMemory,
    receipt: &OperationReceipt,
) -> RetitleV3Response {
    let terminal = is_terminal_receipt(receipt);
    RetitleV3Response {
        schema_version: RESPONSE_SCHEMA,
        capability: CAPABILITY,
        status: if terminal { "terminal" } else { "accepted" }.to_string(),
        outcome: receipt.outcome.clone(),
        changed: receipt.changed,
        failure_class: receipt.failure_class.clone(),
        failure_stage: receipt.failure_stage.clone(),
        provider_attempts: receipt.provider_attempts.clone(),
        operation_hash: receipt.operation_hash.clone(),
        memory_revision: memory.revision,
        title_revision: record.title_revision,
        session_incarnation: incarnation(record),
        readiness: memory.readiness.clone(),
        title: record.title.clone(),
        admission_fence: RetitleV3ResultFence {
            session_incarnation: receipt.admitted_incarnation.clone(),
            title_revision: receipt.admitted_title_revision,
            memory_revision: receipt.admitted_memory_revision,
        },
        result_fence: receipt
            .result_title_revision
            .map(|title_revision| RetitleV3ResultFence {
                session_incarnation: receipt.result_incarnation.clone(),
                title_revision,
                memory_revision: receipt
                    .result_memory_revision
                    .unwrap_or(receipt.admitted_memory_revision),
            }),
        current_fence: RetitleV3ResultFence {
            session_incarnation: incarnation(record),
            title_revision: record.title_revision,
            memory_revision: memory.revision,
        },
        result_is_current: receipt.result_title_revision.is_some_and(|title_revision| {
            receipt.result_incarnation == incarnation(record)
                && title_revision == record.title_revision
                && receipt.result_memory_revision == Some(memory.revision)
        }),
        started_at: receipt.created_at.clone(),
        finished_at: terminal.then(|| receipt.updated_at.clone()),
        duration_bucket: receipt.duration_bucket.clone(),
    }
}

fn incarnation(record: &SessionRecord) -> Option<String> {
    record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .filter(|launch_id| !launch_id.is_empty())
}

fn delta_hash(messages: &[HistoryMessage], cursor: &IncrementalHistoryCursor) -> String {
    let mut digest = Sha256::new();
    digest.update(cursor.segment_id.as_bytes());
    digest.update(cursor.offset.to_le_bytes());
    for message in messages {
        digest.update(message.id.as_bytes());
        digest.update([0]);
        digest.update(hash_value(&message.text).as_bytes());
        digest.update([0]);
    }
    format_digest(digest.finalize().as_slice())
}

fn operation_hash(id: &str, key: &str) -> String {
    hash_value(&format!("{id}\0{key}"))
}

pub(crate) fn request_operation_hash(id: &str, request: &RetitleV3Request) -> String {
    if request.trigger == "automatic" {
        operation_hash(id, &request_fingerprint(request))
    } else {
        operation_hash(id, &request.idempotency_key)
    }
}

pub(crate) fn valid_operation_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    })
}

fn request_fingerprint(request: &RetitleV3Request) -> String {
    let value = if request.trigger == "automatic" {
        json!({
            "schema_version": request.schema_version,
            "trigger": request.trigger,
            "session_incarnation": &request.expected.session_incarnation,
            "provider_turn_id": request.expected.provider_turn_id.as_deref(),
        })
    } else {
        json!({
            "schema_version": request.schema_version,
            "trigger": request.trigger,
            "idempotency_key": request.idempotency_key,
            "expected": &request.expected,
        })
    };
    hash_value(&serde_json::to_string(&value).expect("request fingerprint is serializable"))
}

fn hash_value(value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(value.as_bytes());
    format_digest(digest.finalize().as_slice())
}

fn format_digest(bytes: &[u8]) -> String {
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("sha256:{hex}")
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut output = value
        .chars()
        .take(max.saturating_sub(1))
        .collect::<String>();
    output.push('…');
    output
}

fn push_bounded<T>(values: &mut Vec<T>, value: T, max: usize) {
    values.push(value);
    if values.len() > max {
        values.drain(..values.len() - max);
    }
}

fn history_error(error: HistoryError) -> CliError {
    match error {
        HistoryError::NotFound => v3_error(
            "retitle-v3-history-unavailable",
            "provider history is unavailable for this session",
        ),
        HistoryError::InvalidCursor => v3_error(
            "retitle-v3-history-stale",
            "provider history cursor is stale",
        ),
        HistoryError::Io => v3_error(
            "retitle-v3-history-degraded",
            "provider history cannot be read",
        ),
    }
}

fn error_details(retryable: bool, next_action: &str, strategy: &str) -> serde_json::Value {
    json!({
        "capability": CAPABILITY,
        "retryable": retryable,
        "next_action": next_action,
        "recovery": {
            "strategy": strategy,
            "safe_to_retry": retryable,
        },
    })
}

pub(crate) fn operation_not_found() -> CliError {
    CliError::data(
        "retitle-v3-operation-not-found",
        "retitle v3 operation does not exist",
        Some(error_details(false, "start_new_operation", "new_request")),
    )
}

pub(crate) fn invalid_operation_hash_error() -> CliError {
    CliError::usage(
        "invalid-retitle-v3-operation-hash",
        "retitle v3 operation hash is invalid",
        Some(error_details(false, "fix_request", "correct_request")),
    )
}

fn v3_error(code: &str, message: &str) -> CliError {
    let retryable = matches!(
        code,
        "retitle-v3-history-stale" | "retitle-v3-history-degraded" | "retitle-v3-memory-not-ready"
    );
    CliError::runtime(
        code,
        message,
        Some(error_details(
            retryable,
            "retry_or_inspect_readiness",
            if retryable {
                "same_request"
            } else {
                "refresh_fences"
            },
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    fn message(id: &str, role: &str, text: &str, human_prompt: bool) -> HistoryMessage {
        HistoryMessage {
            id: id.to_string(),
            role: role.to_string(),
            text: text.to_string(),
            timestamp: None,
            human_prompt,
        }
    }

    fn fixture_record(id: &str, title: Option<&str>) -> SessionRecord {
        SessionRecord {
            schema_version: crate::SESSION_DOCUMENT_VERSION.to_string(),
            id: id.to_string(),
            agent: "codex".to_string(),
            mode: "interactive".to_string(),
            coordination_mode: Default::default(),
            title: title.map(str::to_string),
            title_state: title.map(|title| SessionTitleState {
                topic: Some(title.to_string()),
                topic_source: crate::SessionTitleTopicSource::Auto,
                references: Vec::new(),
                activity: None,
                extra: BTreeMap::new(),
            }),
            title_revision: u64::from(title.is_some()),
            cwd: "/work".to_string(),
            tmux_session: format!("hs-{id}"),
            prompt_file: None,
            log_file: None,
            created_at: "2026-09-09T00:00:00Z".to_string(),
            updated_at: "2026-09-09T00:00:00Z".to_string(),
            provider_resume: Some(crate::ProviderResume {
                provider: "codex".to_string(),
                session_id: format!("provider-{id}"),
                captured_at: "2026-09-09T00:00:00Z".to_string(),
                capture_method: "test".to_string(),
                resume_args: Vec::new(),
                extra: BTreeMap::new(),
            }),
            runtime: Some(crate::RuntimeInfo {
                kind: "tmux".to_string(),
                tmux_session: format!("hs-{id}"),
                generation: 1,
                started_at: "2026-09-09T00:00:00Z".to_string(),
                launch_id: format!("launch-{id}"),
                extra: BTreeMap::new(),
            }),
            agent_args: Vec::new(),
            agent_bin: None,
            extra: BTreeMap::new(),
            resume_sidecar_extra: BTreeMap::new(),
        }
    }

    fn codex_row(role: &str, text: &str, turn: &str) -> String {
        let content_type = if role == "user" {
            "input_text"
        } else {
            "output_text"
        };
        serde_json::to_string(&json!({
            "timestamp":"2026-09-09T00:00:01Z",
            "type":"response_item",
            "payload":{
                "type":"message",
                "role":role,
                "content":[{"type":content_type,"text":text}],
                "internal_chat_message_metadata_passthrough":{
                    "turn_id":turn,
                    "content_item_kinds":[format!("{role}.text")]
                }
            }
        }))
        .unwrap()
            + "\n"
    }

    fn fixture(
        tmp: &Path,
        id: &str,
        title: Option<&str>,
        rows: &str,
    ) -> (CliContext, HistoryCatalog, PathBuf) {
        let state_dir = tmp.join("state");
        let session_dir = state_dir.join("sessions").join(id);
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(
            session_dir.join("session.json"),
            serde_json::to_vec_pretty(&fixture_record(id, title)).unwrap(),
        )
        .unwrap();
        let history_root = tmp.join("provider/sessions");
        let transcript_dir = history_root.join("2026/09/09");
        fs::create_dir_all(&transcript_dir).unwrap();
        let transcript = transcript_dir.join("rollout.jsonl");
        let meta = serde_json::to_string(&json!({
            "timestamp":"2026-09-09T00:00:00Z",
            "type":"session_meta",
            "payload":{
                "id":format!("provider-{id}"),
                "cwd":"/work",
                "source":"cli",
                "timestamp":"2026-09-09T00:00:00Z"
            }
        }))
        .unwrap();
        fs::write(&transcript, format!("{meta}\n{rows}")).unwrap();
        let catalog = HistoryCatalog::new(
            vec![crate::provider_history::HistorySource {
                provider: "codex".to_string(),
                agent_profile: None,
                root: history_root,
            }],
            tmp.join("archives"),
            tmp.join("stars"),
        );
        (
            CliContext {
                state_dir,
                host: None,
            },
            catalog,
            transcript,
        )
    }

    fn request(id: &str, title_revision: u64, memory_revision: u64) -> RetitleV3Request {
        RetitleV3Request {
            schema_version: REQUEST_SCHEMA.to_string(),
            trigger: "manual".to_string(),
            idempotency_key: format!("request-{id}"),
            expected: RetitleV3FenceInput {
                session_incarnation: format!("launch-{id}"),
                title_revision,
                memory_revision,
                activity_revision: None,
                provider_turn_id: None,
            },
        }
    }

    fn receipt(hash: &str, trigger: &str, state: &str, created_at: &str) -> OperationReceipt {
        OperationReceipt {
            operation_hash: hash_value(hash),
            idempotency_hash: hash_value(&format!("input-{hash}")),
            trigger: trigger.to_string(),
            state: state.to_string(),
            outcome: is_terminal_receipt_state(state).then(|| "unchanged".to_string()),
            changed: is_terminal_receipt_state(state).then_some(false),
            failure_class: None,
            failure_stage: None,
            provider_attempts: Vec::new(),
            admitted_incarnation: Some("launch-test".to_string()),
            admitted_title_revision: 0,
            admitted_memory_revision: 0,
            activity_revision: None,
            provider_turn_id_hash: None,
            result_incarnation: None,
            result_title_revision: None,
            result_memory_revision: None,
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
            duration_bucket: default_duration_bucket(),
            attempt_generation: 0,
            execution_claim: None,
            memory_revision: 0,
        }
    }

    fn is_terminal_receipt_state(state: &str) -> bool {
        matches!(state, "completed" | "degraded_cached" | "failed")
    }

    #[test]
    fn explicit_human_pivot_replaces_objective_but_assistant_progress_never_does() {
        let mut memory = SemanticMemory::default();
        reduce_messages(
            &mut memory,
            &[
                message("turn-1", "user", "investigate long retitle failures", true),
                message("turn-2", "assistant", "Implementing bounded cursor", false),
                message("turn-3", "assistant", "Decided to retain the origin", false),
            ],
        );
        assert_eq!(
            memory.origin.as_ref().unwrap().text,
            "objective: failures · investigate · long · retitle"
        );
        assert_eq!(memory.active_objective, memory.origin);
        assert_eq!(
            memory.current_activity.as_ref().unwrap().text,
            "decision: decided · origin · retain"
        );
        let durable = serde_json::to_string(&memory).unwrap();
        let provider = render_provider_input(&memory).unwrap();
        for raw in [
            "investigate long retitle failures",
            "Implementing bounded cursor",
            "Decided to retain the origin",
        ] {
            assert!(!durable.contains(raw));
            assert!(!provider.contains(raw));
        }
        for concept in ["investigate", "retitle", "failures", "decision"] {
            assert!(durable.contains(concept));
        }

        reduce_messages(
            &mut memory,
            &[message("turn-followup", "user", "yes, continue", true)],
        );
        assert_eq!(memory.active_objective, memory.origin);

        reduce_messages(
            &mut memory,
            &[message(
                "turn-4",
                "user",
                "now implement semantic memory",
                true,
            )],
        );
        assert_eq!(
            memory.origin.as_ref().unwrap().text,
            "objective: failures · investigate · long · retitle"
        );
        assert_eq!(
            memory.active_objective.as_ref().unwrap().text,
            "objective: implement · memory · semantic"
        );
    }

    #[test]
    fn pivot_corpus_distinguishes_routine_followups_from_explicit_human_changes() {
        for prompt in [
            "yes",
            "continue",
            "what is the status?",
            "please explain that",
        ] {
            assert!(
                !explicit_objective_pivot(prompt),
                "unexpected pivot: {prompt}"
            );
        }
        for prompt in [
            "now implement semantic memory",
            "switch to the readiness endpoint",
            "change the objective to crash recovery",
            "接下來處理 rotation",
            "改成 observability",
        ] {
            assert!(explicit_objective_pivot(prompt), "missed pivot: {prompt}");
        }
    }

    #[test]
    fn state_and_provider_input_stay_bounded_and_remove_privacy_canaries() {
        let mut memory = SemanticMemory::default();
        let canaries = [
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "github_pat_11_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "AKIAIOSFODNN7EXAMPLE",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature",
            "aB3dE5fG7hI9jK1lM3nO5pQ7rS9tU1vW3xY5zA7b",
        ];
        for index in 0..1_000 {
            let text = format!(
                "milestone {index} credential {} {}",
                canaries[index % canaries.len()],
                "progress ".repeat(200)
            );
            reduce_messages(
                &mut memory,
                &[message(&format!("turn-{index}"), "assistant", &text, false)],
            );
        }
        reduce_messages(
            &mut memory,
            &[message(
                "human",
                "user",
                "deliver bounded semantic memory /home/terry/private TOKEN=secret RAW_PRIVATE_TAIL_42",
                true,
            )],
        );
        let marker = serde_json::to_vec(&memory).unwrap();
        let input = render_provider_input(&memory).unwrap();
        assert!(marker.len() <= MAX_MARKER_BYTES);
        assert!(input.len() < MAX_PROVIDER_INPUT_BYTES);
        for canary in canaries {
            assert!(!String::from_utf8_lossy(&marker).contains(canary));
            assert!(!input.contains(canary));
        }
        assert!(!String::from_utf8_lossy(&marker).contains("/home/terry/private"));
        assert!(!String::from_utf8_lossy(&marker).contains("TOKEN=secret"));
        assert!(!input.contains("/home/terry/private"));
        assert!(!input.contains("TOKEN=secret"));
    }

    #[test]
    fn global_compaction_saturates_all_collections_without_evicting_live_work() {
        let text = "semantic fact ".repeat(80);
        let entry = |index: usize| LedgerEntry {
            kind: "progress".to_string(),
            turn_id: hash_value(&format!("turn-{index}")),
            text: truncate_chars(&text, MAX_TEXT_CHARS),
        };
        let mut memory = SemanticMemory {
            revision: 99,
            origin: Some(MemoryFact {
                turn_id: hash_value("origin"),
                text: truncate_chars(&text, MAX_TEXT_CHARS),
            }),
            active_objective: Some(MemoryFact {
                turn_id: hash_value("objective"),
                text: truncate_chars(&text, MAX_TEXT_CHARS),
            }),
            current_activity: Some(MemoryFact {
                turn_id: hash_value("activity"),
                text: truncate_chars(&text, MAX_TEXT_CHARS),
            }),
            milestones: (0..MAX_LEDGER_ENTRIES).map(entry).collect(),
            decisions: (10..10 + MAX_LEDGER_ENTRIES).map(entry).collect(),
            blockers: (20..20 + MAX_LEDGER_ENTRIES).map(entry).collect(),
            journey: (30..30 + MAX_LEDGER_ENTRIES).map(entry).collect(),
            segments: (0..MAX_SEGMENTS)
                .map(|index| SegmentReceipt {
                    segment_id: hash_value(&format!("segment-{index}")),
                    source_id: hash_value(&format!("source-{index}")),
                    discontinuity: index > 0,
                })
                .collect(),
            applied_message_ids: (0..MAX_APPLIED_MESSAGE_IDS)
                .map(|index| hash_value(&format!("applied-{index}")))
                .collect(),
            receipts: (0..MAX_RECEIPTS)
                .map(|index| {
                    receipt(
                        &format!("live-{index}"),
                        "manual",
                        "pending",
                        &format!("2026-09-09T00:00:{index:02}Z"),
                    )
                })
                .collect(),
            ..SemanticMemory::default()
        };
        compact_memory(&mut memory);
        assert!(serde_json::to_vec(&memory).unwrap().len() <= MAX_MARKER_BYTES);
        assert_eq!(memory.receipts.len(), MAX_RECEIPTS);
        assert!(
            memory
                .receipts
                .iter()
                .all(|receipt| receipt.state == "pending")
        );

        push_receipt_bounded(
            &mut memory.receipts,
            receipt(
                "old-terminal",
                "automatic",
                "completed",
                "2026-09-08T00:00:00Z",
            ),
        )
        .unwrap();
        assert_eq!(memory.receipts.len(), MAX_RECEIPTS);
        assert!(
            memory
                .receipts
                .iter()
                .all(|receipt| receipt.state == "pending")
        );
        assert_eq!(
            push_receipt_bounded(
                &mut memory.receipts,
                receipt("overflow-live", "manual", "pending", "2026-09-09T00:00:09Z",),
            )
            .unwrap_err()
            .code(),
            "retitle-v3-operation-capacity"
        );
        assert_eq!(memory.receipts.len(), MAX_RECEIPTS);
    }

    #[test]
    fn strict_request_requires_fences_and_printable_bounded_key() {
        let mut valid = request("strict", 0, 0);
        assert!(validate_request(&valid).is_ok());
        valid.idempotency_key = "short".to_string();
        assert_eq!(
            validate_request(&valid).unwrap_err().code(),
            "invalid-retitle-v3-request"
        );
        valid.idempotency_key = "contains space".to_string();
        assert_eq!(
            validate_request(&valid).unwrap_err().code(),
            "invalid-retitle-v3-request"
        );
        valid.idempotency_key = "valid-key".to_string();
        valid.trigger = "automatic".to_string();
        assert_eq!(
            validate_request(&valid).unwrap_err().code(),
            "invalid-retitle-v3-request"
        );
    }

    #[test]
    fn machine_readiness_never_serializes_session_sentinel_fields() {
        let value = serde_json::to_value(global_readiness(false)).unwrap();
        assert_eq!(value["status"], "degraded");
        for forbidden in [
            "provider_status",
            "context_status",
            "title_status",
            "memory_revision",
            "usable_memory",
            "pending_operation",
            "cursor_fence_hash",
        ] {
            assert!(value.get(forbidden).is_none(), "unexpected {forbidden}");
        }
    }

    #[test]
    fn restart_polling_adopts_ready_manual_operation_and_renders_without_provider() {
        let tmp = tempfile::tempdir().unwrap();
        let id = "restart-manual";
        let (context, catalog, _) = fixture(
            tmp.path(),
            id,
            None,
            &codex_row("user", "make long retitle reliable", "turn-one"),
        );
        let accepted = refresh_once(&context, &catalog, id, &request(id, 0, 0)).unwrap();
        assert_eq!(accepted.status, "accepted");
        let adopted =
            refresh_operation_once(&context, &catalog, id, &accepted.operation_hash).unwrap();
        assert_eq!(adopted.status, "accepted");
        let terminal = complete_manual_locally(&context, &catalog, id, &accepted.operation_hash)
            .unwrap()
            .expect("manual memory-first result");
        assert_eq!(terminal.status, "terminal");
        assert_eq!(terminal.outcome.as_deref(), Some("committed"));
        assert_eq!(
            terminal.title.as_deref(),
            Some("objective: long · make · reliable · retitle")
        );
        let replay = operation_response(&context, id, &accepted.operation_hash)
            .unwrap()
            .unwrap();
        assert_eq!(replay.result_fence.as_ref().unwrap().title_revision, 1);
        assert!(replay.finished_at.is_some());
    }

    #[test]
    fn provider_claim_is_single_owner_and_expired_uncertain_work_is_not_retried() {
        let tmp = tempfile::tempdir().unwrap();
        let id = "claim-race";
        let (context, _catalog, _) = fixture(
            tmp.path(),
            id,
            None,
            &codex_row("user", "claim exactly once", "turn-one"),
        );
        let mut record = load_session_record(&context, id).unwrap();
        let mut memory = SemanticMemory {
            revision: 1,
            readiness: MemoryReadiness::Ready,
            origin: Some(MemoryFact {
                turn_id: hash_value("turn-one"),
                text: "claim exactly once".to_string(),
            }),
            active_objective: Some(MemoryFact {
                turn_id: hash_value("turn-one"),
                text: "claim exactly once".to_string(),
            }),
            ..SemanticMemory::default()
        };
        let mut operation = receipt(
            "claim-operation",
            "automatic",
            "ready",
            "2026-09-09T00:00:00Z",
        );
        operation.operation_hash = request_operation_hash(
            id,
            &RetitleV3Request {
                schema_version: REQUEST_SCHEMA.to_string(),
                trigger: "automatic".to_string(),
                idempotency_key: "automatic-claim".to_string(),
                expected: RetitleV3FenceInput {
                    session_incarnation: format!("launch-{id}"),
                    title_revision: 0,
                    memory_revision: 1,
                    activity_revision: Some(1),
                    provider_turn_id: Some("turn-one".to_string()),
                },
            },
        );
        let operation_hash = operation.operation_hash.clone();
        memory.receipts.push(operation);
        store_memory(&mut record, &memory).unwrap();
        crate::write_session_record(&context, &record).unwrap();

        assert!(matches!(
            claim_provider_inference(&context, id, &operation_hash, "first-claim").unwrap(),
            ProviderClaim::Claimed(_)
        ));
        assert!(matches!(
            claim_provider_inference(&context, id, &operation_hash, "second-claim").unwrap(),
            ProviderClaim::Waiting(_)
        ));
        let mut record = load_session_record(&context, id).unwrap();
        let mut memory = memory_from_record(&record).unwrap();
        memory.receipts[0]
            .execution_claim
            .as_mut()
            .unwrap()
            .expires_at_second = 0;
        store_memory(&mut record, &memory).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        let terminal =
            match claim_provider_inference(&context, id, &operation_hash, "restart-takeover")
                .unwrap()
            {
                ProviderClaim::RecoveredTerminal(response) => response,
                _ => panic!("expired claim must terminalize"),
            };
        assert_eq!(terminal.outcome.as_deref(), Some("terminal_failure"));
        assert_eq!(
            terminal.failure_class.as_deref(),
            Some("uncertain_execution")
        );
        let marker = serde_json::to_string(
            load_session_record(&context, id)
                .unwrap()
                .extra
                .get(MARKER_KEY)
                .unwrap(),
        )
        .unwrap();
        assert!(!marker.contains("first-claim"));
        assert!(!marker.contains("second-claim"));
        assert!(!marker.contains("restart-takeover"));
    }

    #[test]
    fn expired_provider_claim_preserves_cached_title_as_degraded() {
        let tmp = tempfile::tempdir().unwrap();
        let id = "claim-cached";
        let (context, _, _) = fixture(tmp.path(), id, Some("Cached title"), "");
        let mut record = load_session_record(&context, id).unwrap();
        let mut memory = SemanticMemory {
            revision: 1,
            readiness: MemoryReadiness::Ready,
            origin: Some(MemoryFact {
                turn_id: hash_value("origin"),
                text: "cached objective".to_string(),
            }),
            active_objective: Some(MemoryFact {
                turn_id: hash_value("origin"),
                text: "cached objective".to_string(),
            }),
            ..SemanticMemory::default()
        };
        let mut operation = receipt(
            "cached-operation",
            "automatic",
            "ready",
            "2026-09-09T00:00:00Z",
        );
        operation.execution_claim = Some(ExecutionClaim {
            token_hash: hash_value("dead-daemon"),
            generation: 1,
            acquired_at: "2026-09-09T00:00:00Z".to_string(),
            expires_at_second: 0,
        });
        operation.attempt_generation = 1;
        let operation_hash = operation.operation_hash.clone();
        memory.receipts.push(operation);
        store_memory(&mut record, &memory).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        let response =
            match claim_provider_inference(&context, id, &operation_hash, "takeover").unwrap() {
                ProviderClaim::RecoveredTerminal(response) => response,
                _ => panic!("expired claim must terminalize"),
            };
        assert_eq!(response.outcome.as_deref(), Some("degraded_cached"));
        assert_eq!(response.title.as_deref(), Some("Cached title"));
        assert_eq!(
            response.failure_class.as_deref(),
            Some("uncertain_execution")
        );
    }

    #[test]
    fn durable_manager_prioritizes_manual_and_supersedes_queued_auto() {
        let record = fixture_record("manager", Some("title"));
        let mut memory = SemanticMemory::default();
        let mut executing = receipt(
            "auto-executing",
            "automatic",
            "ready",
            "2026-09-08T23:59:59Z",
        );
        executing.execution_claim = Some(ExecutionClaim {
            token_hash: hash_value("active-provider"),
            generation: 1,
            acquired_at: "2026-09-09T00:00:00Z".to_string(),
            expires_at_second: i64::MAX,
        });
        executing.attempt_generation = 1;
        let executing_hash = executing.operation_hash.clone();
        memory.receipts.push(executing);
        let old_auto = receipt("auto-old", "automatic", "ready", "2026-09-09T00:00:00Z");
        let old_auto_hash = old_auto.operation_hash.clone();
        memory.receipts.push(old_auto);
        let manual = receipt("manual", "manual", "ready", "2026-09-09T00:00:01Z");
        terminalize_superseded_operations(&mut memory, &manual, &record);
        push_receipt_bounded(&mut memory.receipts, manual).unwrap();
        assert_eq!(
            selected_operation(&memory).unwrap().operation_hash,
            executing_hash
        );
        let old = memory
            .receipts
            .iter()
            .find(|receipt| receipt.operation_hash == old_auto_hash)
            .unwrap();
        assert!(is_terminal_receipt(old));
        assert_eq!(old.failure_class.as_deref(), Some("superseded"));
        let executing = memory
            .receipts
            .iter_mut()
            .find(|receipt| receipt.operation_hash == executing_hash)
            .unwrap();
        assert!(!is_terminal_receipt(executing));
        executing.state = "completed".to_string();
        executing.execution_claim = None;
        assert_eq!(selected_operation(&memory).unwrap().trigger, "manual");
    }

    #[test]
    fn reducer_is_deterministic_across_restart_boundaries() {
        let messages = (0..1_000)
            .map(|index| {
                if index % 25 == 0 {
                    message(
                        &format!("turn-{index}"),
                        "user",
                        &format!("now objective {index}"),
                        true,
                    )
                } else {
                    message(
                        &format!("turn-{index}"),
                        "assistant",
                        &format!("progress {index}"),
                        false,
                    )
                }
            })
            .collect::<Vec<_>>();
        let mut uninterrupted = SemanticMemory::default();
        reduce_messages(&mut uninterrupted, &messages);
        let mut resumed = SemanticMemory::default();
        reduce_messages(&mut resumed, &messages[..500]);
        resumed = serde_json::from_slice(&serde_json::to_vec(&resumed).unwrap()).unwrap();
        reduce_messages(&mut resumed, &messages[500..]);
        assert_eq!(uninterrupted, resumed);
        assert_eq!(
            resumed.origin.as_ref().unwrap().text,
            "objective: 0 · objective"
        );
        assert_eq!(
            resumed.active_objective.as_ref().unwrap().text,
            "objective: 975 · objective"
        );
    }

    #[test]
    fn legacy_or_unknown_v2_state_does_not_change_v3_default() {
        let mut extra = BTreeMap::new();
        extra.insert("session_retitle_v2".to_string(), json!({"opaque": true}));
        let record = SessionRecord {
            schema_version: "agent-session.session.v1".to_string(),
            id: "session".to_string(),
            agent: "codex".to_string(),
            mode: "interactive".to_string(),
            coordination_mode: Default::default(),
            title: None,
            title_state: None,
            title_revision: 0,
            cwd: "/work".to_string(),
            tmux_session: "session".to_string(),
            prompt_file: None,
            log_file: None,
            created_at: "2026-09-09T00:00:00Z".to_string(),
            updated_at: "2026-09-09T00:00:00Z".to_string(),
            provider_resume: None,
            runtime: None,
            agent_args: Vec::new(),
            agent_bin: None,
            extra,
            resume_sidecar_extra: BTreeMap::new(),
        };
        assert_eq!(
            memory_from_record(&record).unwrap(),
            SemanticMemory::default()
        );
    }

    #[test]
    fn automatic_requests_coalesce_by_observation_while_manual_keys_remain_distinct() {
        let expected = RetitleV3FenceInput {
            session_incarnation: "launch".to_string(),
            title_revision: 3,
            memory_revision: 7,
            activity_revision: Some(12),
            provider_turn_id: Some("turn-12".to_string()),
        };
        let automatic = |key: &str| RetitleV3Request {
            schema_version: REQUEST_SCHEMA.to_string(),
            trigger: "automatic".to_string(),
            idempotency_key: key.to_string(),
            expected: expected.clone(),
        };
        assert_eq!(
            request_operation_hash("session", &automatic("first")),
            request_operation_hash("session", &automatic("newest"))
        );
        let mut manual_one = automatic("first");
        manual_one.trigger = "manual".to_string();
        let mut manual_two = automatic("newest");
        manual_two.trigger = "manual".to_string();
        assert_ne!(
            request_operation_hash("session", &manual_one),
            request_operation_hash("session", &manual_two)
        );
    }

    #[test]
    fn operation_response_has_unambiguous_terminal_outcome_and_revision_fence() {
        let record = fixture_record("contract", Some("Existing title"));
        let mut memory = SemanticMemory {
            revision: 9,
            readiness: MemoryReadiness::Degraded,
            ..SemanticMemory::default()
        };
        memory.receipts.push(OperationReceipt {
            operation_hash: "sha256:operation".to_string(),
            idempotency_hash: "sha256:input".to_string(),
            trigger: "manual".to_string(),
            state: "degraded_cached".to_string(),
            outcome: Some("degraded_cached".to_string()),
            changed: Some(false),
            failure_class: Some("primary:timeout,fallback:quota_exceeded".to_string()),
            failure_stage: Some("provider_call".to_string()),
            provider_attempts: Vec::new(),
            admitted_incarnation: Some("launch-contract".to_string()),
            admitted_title_revision: 1,
            admitted_memory_revision: 8,
            activity_revision: None,
            provider_turn_id_hash: None,
            result_incarnation: Some("launch-contract".to_string()),
            result_title_revision: Some(1),
            result_memory_revision: Some(9),
            created_at: "2026-09-09T00:00:00Z".to_string(),
            updated_at: "2026-09-09T00:00:01Z".to_string(),
            duration_bucket: "1_4_s".to_string(),
            attempt_generation: 1,
            execution_claim: None,
            memory_revision: 9,
        });
        let value =
            serde_json::to_value(response_from_receipt(&record, &memory, &memory.receipts[0]))
                .unwrap();
        assert_eq!(value["status"], "terminal");
        assert_eq!(value["outcome"], "degraded_cached");
        assert_eq!(value["changed"], false);
        assert_eq!(value["title"], "Existing title");
        assert_eq!(value["session_incarnation"], "launch-contract");
        assert_eq!(value["title_revision"], 1);
        assert_eq!(value["memory_revision"], 9);
        assert_eq!(
            value["result_fence"]["session_incarnation"],
            "launch-contract"
        );
        assert_eq!(value["result_fence"]["title_revision"], 1);
        assert_eq!(value["result_fence"]["memory_revision"], 9);
        let rendered = value.to_string();
        assert!(!rendered.contains("primary prompt"));
        assert!(!rendered.contains("/home/"));
    }

    #[test]
    fn crash_before_memory_commit_replays_without_skipping_or_duplicating_the_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let id = "crash-reduce";
        let (context, catalog, _) = fixture(
            tmp.path(),
            id,
            None,
            &codex_row("user", "preserve this origin", "turn-one"),
        );
        let request = request(id, 0, 0);
        crate::fail_session_record_write_on_nth_call(1);
        assert_eq!(
            refresh_once(&context, &catalog, id, &request)
                .unwrap_err()
                .code(),
            "session-write-injected"
        );
        assert_eq!(
            memory_from_record(&load_session_record(&context, id).unwrap())
                .unwrap()
                .revision,
            0
        );

        let response = refresh_once(&context, &catalog, id, &request).unwrap();
        assert_eq!(response.status, "accepted");
        let memory = memory_from_record(&load_session_record(&context, id).unwrap()).unwrap();
        assert_eq!(memory.revision, 1);
        assert_eq!(
            memory.origin.as_ref().unwrap().text,
            "objective: origin · preserve"
        );
        assert_eq!(memory.journey.len(), 1);
    }

    #[test]
    fn crash_before_title_commit_retries_idempotently_and_history_progress_rejects_stale_result() {
        let tmp = tempfile::tempdir().unwrap();
        let id = "crash-title";
        let (context, catalog, transcript) = fixture(
            tmp.path(),
            id,
            None,
            &codex_row("user", "make long retitle reliable", "turn-one"),
        );
        let request = request(id, 0, 0);
        let accepted = refresh_once(&context, &catalog, id, &request).unwrap();
        let inference = inference_context(&context, id, &accepted.operation_hash).unwrap();
        let title_state = SessionTitleState {
            topic: Some("Reliable long retitle".to_string()),
            topic_source: crate::SessionTitleTopicSource::Auto,
            references: Vec::new(),
            activity: None,
            extra: BTreeMap::new(),
        };
        crate::fail_session_record_write_on_nth_call(1);
        assert_eq!(
            commit_inference(&context, &catalog, id, &inference, title_state.clone(), &[])
                .unwrap_err()
                .code(),
            "session-write-injected"
        );
        let after_crash = load_session_record(&context, id).unwrap();
        assert_eq!(after_crash.title_revision, 0);
        assert_eq!(
            memory_from_record(&after_crash).unwrap().receipts[0].state,
            "ready"
        );
        let completed =
            commit_inference(&context, &catalog, id, &inference, title_state, &[]).unwrap();
        assert_eq!(completed.status, "terminal");
        assert_eq!(completed.outcome.as_deref(), Some("committed"));
        assert_eq!(completed.changed, Some(true));
        assert_eq!(completed.title_revision, 1);

        let next = RetitleV3Request {
            idempotency_key: "stale-history".to_string(),
            expected: RetitleV3FenceInput {
                session_incarnation: format!("launch-{id}"),
                title_revision: 1,
                memory_revision: 1,
                activity_revision: None,
                provider_turn_id: None,
            },
            ..request
        };
        let fresh = refresh_once(&context, &catalog, id, &next).unwrap();
        assert_eq!(fresh.outcome.as_deref(), Some("unchanged"));

        // A direct history-fence probe proves even a same-record inference is
        // rejected once a provider turn has appended.
        let record = load_session_record(&context, id).unwrap();
        let current_memory = memory_from_record(&record).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap()
            .write_all(codex_row("user", "new pivot", "turn-two").as_bytes())
            .unwrap();
        assert_eq!(
            validate_history_fence(&catalog, &record, &current_memory)
                .unwrap_err()
                .code(),
            "retitle-v3-history-conflict"
        );
    }

    #[test]
    fn stale_client_fence_and_reused_manual_key_are_rejected() {
        let record = fixture_record("fence", Some("Current"));
        let stale = RetitleV3FenceInput {
            session_incarnation: "launch-fence".to_string(),
            title_revision: 0,
            memory_revision: 0,
            activity_revision: None,
            provider_turn_id: None,
        };
        assert_eq!(
            validate_request_fence(
                &CliContext {
                    state_dir: tempfile::tempdir().unwrap().path().to_path_buf(),
                    host: None
                },
                &record,
                &stale
            )
            .unwrap_err()
            .code(),
            "retitle-v3-state-conflict"
        );
        let one = request_fingerprint(&RetitleV3Request {
            schema_version: REQUEST_SCHEMA.to_string(),
            trigger: "manual".to_string(),
            idempotency_key: "same".to_string(),
            expected: stale.clone(),
        });
        let two = request_fingerprint(&RetitleV3Request {
            schema_version: REQUEST_SCHEMA.to_string(),
            trigger: "manual".to_string(),
            idempotency_key: "same".to_string(),
            expected: RetitleV3FenceInput {
                title_revision: 1,
                ..stale
            },
        });
        assert_ne!(one, two);
    }
}
