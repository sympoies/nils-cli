//! One event-scoped recovery observation for previously degraded conversation checks.
use std::{
    collections::BTreeSet,
    fs::{File, Metadata, OpenOptions},
    io::Read,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{CONVERSATION_DEGRADED, Lane, lane};
use crate::{
    model::{
        Capability, DecisionAction, DecisionReason, LoadedPolicy, NormalizedDecision,
        NormalizedRequest, PolicyRule, RuleMode,
    },
    paths,
};

pub(crate) const EVENT_CONTEXT: &str = "This degradation applies to this event only. Conversation and read-only diagnosis remain available. Mutations are evaluated individually; retry an authorized operation and use its own tool/hook result.";
const RECOVERED: &str = "conversation-degradation-recovered";
const RECOVERY: &str = "Conversation recovery: the previously degraded checks succeeded for this event. The earlier degradation note applied to its event only. Mutations are evaluated individually against their own tool/hook results; this observation grants no mutation authority.";
const SCHEMA: &str = "agent-hook.conversation-degradation.v1";
const MAX_STATE_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
struct Check {
    rule_id: String,
    capability_digest: String,
}

impl Check {
    fn new(rule: &PolicyRule) -> Self {
        let bytes = serde_json::to_vec(&rule.capability).expect("serializable capability");
        Self {
            rule_id: rule.id.clone(),
            capability_digest: Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        }
    }
}

/// Hold the presentation lock until fallible dispatch preparation has succeeded.
pub(crate) struct PendingRecovery {
    path: PathBuf,
    bytes: Vec<u8>,
    _lock: File,
}

impl PendingRecovery {
    pub(crate) fn commit(self) {
        let _ = nils_common::fs::write_atomic(&self.path, &self.bytes, 0o600);
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema_version: String,
    identity: String,
    pending: BTreeSet<Check>,
}

/// Presentation state is best-effort and cannot change admission on an I/O fault.
pub(crate) fn update(
    root: &Path,
    loaded: &LoadedPolicy,
    prepared: &crate::evaluator::PreparedEvaluation<'_>,
    request: &NormalizedRequest,
    decision: &mut NormalizedDecision,
) -> Option<PendingRecovery> {
    update_private(root, loaded, prepared, request, decision)
}

fn update_private(
    root: &Path,
    loaded: &LoadedPolicy,
    prepared: &crate::evaluator::PreparedEvaluation<'_>,
    request: &NormalizedRequest,
    decision: &mut NormalizedDecision,
) -> Option<PendingRecovery> {
    if matches!(lane(&request.event), Lane::Terminal)
        || matches!(
            decision.action,
            DecisionAction::Block | DecisionAction::Transform
        )
    {
        return None;
    }
    // Use the same exact managed selectors as activity evaluation, without retaining them.
    let session = std::env::var("AGENT_SESSION_ID").ok()?;
    let runtime = std::env::var("AGENT_SESSION_RUNTIME_ID").ok()?;
    if [&session, &runtime]
        .iter()
        .any(|id| id.trim().is_empty() || id.len() > 256 || id.chars().any(char::is_control))
    {
        return None;
    }
    let identity = Sha256::digest(format!(
        "{SCHEMA}\0{}\0{session}\0{runtime}",
        request.product.as_str()
    ))
    .iter()
    .map(|byte| format!("{byte:02x}"))
    .collect::<String>();
    let directory = root.join("conversation-degradation");
    paths::ensure_private_state_dir(&directory, "conversation-degradation").ok()?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join(format!("{identity}.lock")))
        .ok()?;
    if !private(&lock.metadata().ok()?)
        || unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0
    {
        return None;
    }
    // Closing the owned lock descriptor releases flock on every return path.
    let path = directory.join(format!("{identity}.json"));
    let mut state = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
    {
        Ok(file) => {
            if !private(&file.metadata().ok()?) {
                return None;
            }
            let mut bytes = Vec::new();
            file.take(MAX_STATE_BYTES + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() as u64 > MAX_STATE_BYTES {
                return None;
            }
            let state: State = serde_json::from_slice(&bytes).ok()?;
            if state.schema_version != SCHEMA
                || state.identity != identity
                || state.pending.len() > crate::evaluator::MAX_REASONS
            {
                return None;
            }
            state
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => State {
            schema_version: SCHEMA.to_string(),
            identity,
            pending: BTreeSet::new(),
        },
        Err(_) => return None,
    };
    let degraded: Vec<_> = decision
        .reasons
        .iter()
        .filter(|reason| reason.code == CONVERSATION_DEGRADED)
        .collect();
    let recover = if degraded.is_empty() {
        !state.pending.is_empty()
            && state
                .pending
                .iter()
                .all(|check| succeeded(check, loaded, decision))
    } else {
        for reason in degraded {
            if reason.rule_id == "runtime.degradation" {
                state.pending.extend(
                    prepared
                        .rules
                        .iter()
                        .filter(|selected| selected.mode == RuleMode::Enforce && !selected.recovery)
                        .filter(|selected| coordination(&selected.rule.capability))
                        .map(|selected| Check::new(selected.rule)),
                );
            } else if let Some(rule) = loaded
                .bundle
                .rules
                .iter()
                .find(|rule| rule.id == reason.rule_id && activity(&rule.capability).is_some())
            {
                state.pending.insert(Check::new(rule));
            }
        }
        false
    };
    if recover {
        if decision.reasons.len() >= crate::evaluator::MAX_REASONS
            || decision.context.as_ref().map_or(0, |text| text.len() + 1) + RECOVERY.len()
                > crate::evaluator::MAX_AGGREGATE_CONTEXT
        {
            return None;
        }
        state.pending.clear();
    } else if state.pending.is_empty() {
        return None;
    }
    let bytes = serde_json::to_vec(&state).ok()?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return None;
    }
    if !recover {
        nils_common::fs::write_atomic(&path, &bytes, 0o600).ok()?;
        return None;
    }
    if decision.action == DecisionAction::Allow {
        decision.action = DecisionAction::Context;
    }
    decision.context = Some(match decision.context.take() {
        Some(context) => format!("{context}\n{RECOVERY}"),
        None => RECOVERY.to_string(),
    });
    decision.reasons.push(DecisionReason {
        rule_id: "runtime.degradation".to_string(),
        code: RECOVERED.to_string(),
        disposition: "context".to_string(),
        observation: false,
    });
    Some(PendingRecovery {
        path,
        bytes,
        _lock: lock,
    })
}

fn private(metadata: &Metadata) -> bool {
    metadata.is_file()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o077 == 0
        && metadata.nlink() == 1
        && metadata.len() <= MAX_STATE_BYTES
}

fn activity(capability: &Capability) -> Option<&str> {
    match capability {
        Capability::SessionActivity { reason_code } => Some(reason_code),
        Capability::DshPolicy { group }
            if matches!(
                group,
                crate::policy_parity::DshCapabilityGroup::AgentActivity
            ) =>
        {
            Some(group.as_str())
        }
        _ => None,
    }
}

fn coordination(capability: &Capability) -> bool {
    matches!(
        capability,
        Capability::SemanticConflict { .. }
            | Capability::SessionCoordination { .. }
            | Capability::OwnerLiveness { .. }
            | Capability::DshPolicy {
                group: crate::policy_parity::DshCapabilityGroup::SemanticConflict
                    | crate::policy_parity::DshCapabilityGroup::OwnerUnclaimed
                    | crate::policy_parity::DshCapabilityGroup::OperationLifecycle
            }
    )
}

fn succeeded(check: &Check, loaded: &LoadedPolicy, decision: &NormalizedDecision) -> bool {
    decision.reasons.iter().any(|reason| {
        if reason.observation
            || reason.disposition != "allow"
            || reason.code == "coordination-unmanaged"
        {
            return false;
        }
        let Some(rule) = loaded
            .bundle
            .rules
            .iter()
            .find(|rule| rule.id == reason.rule_id && rule.id == check.rule_id)
        else {
            return false;
        };
        if Check::new(rule) != *check {
            return false;
        }
        if let Some(code) = activity(&rule.capability) {
            return code == reason.code;
        }
        match &rule.capability {
            Capability::SemanticConflict { reason_code }
            | Capability::SessionCoordination { reason_code } => &reason.code == reason_code,
            Capability::OwnerLiveness { .. } => matches!(
                reason.code.as_str(),
                "owner-unclaimed" | "owner-active-self"
            ),
            Capability::DshPolicy { group }
                if matches!(
                    group,
                    crate::policy_parity::DshCapabilityGroup::SemanticConflict
                        | crate::policy_parity::DshCapabilityGroup::OwnerUnclaimed
                        | crate::policy_parity::DshCapabilityGroup::OperationLifecycle
                ) =>
            {
                reason.code == group.as_str()
            }
            _ => false,
        }
    })
}
