//! An approved terminal disposition of a stopped inventory, without another repair round.
use super::*;

pub const INPUT_SCHEMA: &str = "forge-cli.review-loop-acceptance.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceRequest {
    pub stopped_record: String,
    pub attempted_findings: Vec<ReviewFindingObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_comment: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcceptanceReceipt {
    pub request: AcceptanceRequest,
    pub proposal_digest: String,
    pub original_stop: ReviewLoopHardStop,
    pub approval_reference: String,
    pub accepted_fingerprints: Vec<String>,
    pub findings: Vec<ReviewFindingObservation>,
}

pub fn invalid(message: &str) -> ForgeError {
    ForgeError::validation_with_details(
        error_schema(),
        "review_acceptance_invalid",
        message,
        serde_json::json!({"retryable":false,"next_action":"inspect-stopped-inventory","recovery":{"requires_explicit_state_tip":true,"requires_exact_stopped_head":true}}),
    )
}

impl AcceptanceRequest {
    pub fn validate(&self) -> Result<(), ForgeError> {
        if !self
            .stopped_record
            .strip_prefix("sha256:")
            .is_some_and(|v| {
                v.len() == 64
                    && v.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            || self.approval_comment == Some(0)
        {
            return Err(invalid(
                "acceptance requires a canonical stopped record and a positive approval comment id",
            ));
        }
        canonical_observations(&self.attempted_findings)?;
        Ok(())
    }
}

pub fn proposal_digest(
    request: &AcceptanceRequest,
    head: &str,
    findings: &[ReviewFindingObservation],
) -> Result<String, ForgeError> {
    request.validate()?;
    let bytes = serde_json::to_vec(&(
        INPUT_SCHEMA,
        &request.stopped_record,
        head,
        canonical_observations(&request.attempted_findings)?,
        canonical_observations(findings)?,
    ))
    .map_err(|_| invalid("cannot encode acceptance proposal"))?;
    Ok(sha256_digest(&bytes))
}

pub fn approval_marker(proposal: &str) -> String {
    format!("<!-- forge-cli:review-loop-acceptance:v1 proposal={proposal} -->")
}

pub fn apply(
    chain: &ReviewStateChain,
    head: &str,
    expected_tip: Option<&str>,
    request: &AcceptanceRequest,
    findings: &[ReviewFindingObservation],
    approval_reference: String,
) -> Result<ReviewLoopTransition, ForgeError> {
    request.validate()?;
    if expected_tip.is_none() || expected_tip != chain.tip_digest.as_deref() {
        return Err(invalid(
            "acceptance requires the explicit current state tip",
        ));
    }
    let comment = request
        .approval_comment
        .filter(|id| *id > 0)
        .ok_or_else(|| invalid("acceptance requires a provider approval comment"))?;
    let target = chain
        .records
        .first()
        .ok_or_else(|| invalid("acceptance requires an existing stopped state"))?;
    let reference = url::Url::parse(&approval_reference)
        .map_err(|_| invalid("acceptance approval reference is not a URL"))?;
    if reference.path() != format!("/{}/pull/{}", target.repository, target.pr)
        || reference.fragment() != Some(format!("issuecomment-{comment}").as_str())
    {
        return Err(invalid(
            "acceptance approval reference does not match its comment",
        ));
    }
    let current = canonical_observations(findings)?;
    let attempted = canonical_observations(&request.attempted_findings)?;
    let mut accepted_fingerprints = Vec::new();
    if current.len() != attempted.len() {
        return Err(invalid(
            "acceptance must retain the complete stopped inventory",
        ));
    }
    for (key, row) in &current {
        let original = attempted
            .get(key)
            .ok_or_else(|| invalid("acceptance added a finding"))?;
        if row == original {
            continue;
        }
        let mut accepted = original.clone();
        accepted.status = ReviewFindingStatus::Accepted;
        accepted.blocking = false;
        if original.status != ReviewFindingStatus::Open || row != &accepted {
            return Err(invalid(
                "acceptance may only change open findings to accepted",
            ));
        }
        accepted_fingerprints.push(key.clone());
    }
    if accepted_fingerprints.is_empty()
        || current
            .values()
            .any(|row| row.status == ReviewFindingStatus::Open && row.blocking)
    {
        return Err(invalid(
            "terminal acceptance must dispose every open blocking finding",
        ));
    }
    let source = chain
        .records
        .iter()
        .find(|r| r.record_digest == request.stopped_record)
        .and_then(record_state)
        .ok_or_else(|| invalid("acceptance target is absent from the chain"))?;
    let stop = source
        .hard_stop
        .as_ref()
        .ok_or_else(|| invalid("acceptance requires a durable hard stop"))?;
    if stop.attempted_head_sha != head
        || stop.observation_digest != review_observation_digest(head, &attempted)?
    {
        return Err(invalid(
            "acceptance proof does not match the stopped head and inventory",
        ));
    }
    let receipt = AcceptanceReceipt {
        request: AcceptanceRequest {
            attempted_findings: attempted.values().cloned().collect(),
            ..request.clone()
        },
        proposal_digest: proposal_digest(request, head, findings)?,
        original_stop: stop.clone(),
        approval_reference,
        accepted_fingerprints,
        findings: current.values().cloned().collect(),
    };
    let latest = chain
        .records
        .iter()
        .rev()
        .find(|r| record_state(r).is_some())
        .ok_or_else(|| invalid("acceptance requires an existing stopped state"))?;
    if let ReviewStatePayload::ReviewLoopAcceptance { state, acceptance } = &latest.payload
        && acceptance == &receipt
        && latest.expected_head == head
    {
        if super::super::pr_review_handoff::latest(chain)
            .is_some_and(|h| latest.assignment_generation != Some(h.assignment_generation))
        {
            return Err(invalid(
                "an earlier-assignment acceptance cannot establish a new review interval",
            ));
        }
        return Ok(ReviewLoopTransition {
            state: state.clone(),
            changed: false,
            correction: None,
            supersession: None,
            acceptance: Some(receipt),
        });
    }
    if latest.record_digest != request.stopped_record {
        return Err(invalid(
            "acceptance must name the latest stopped review-loop record",
        ));
    }
    let previous = record_state(latest).unwrap();
    if previous
        .findings
        .iter()
        .any(|(key, f)| f.status == ReviewFindingStatus::Open && !attempted.contains_key(key))
    {
        return Err(invalid("acceptance cannot omit a previously open finding"));
    }
    let mut state = previous.clone();
    state.head_sha = head.into();
    state.hard_stop = None;
    for (key, row) in current {
        let finding = state
            .findings
            .entry(key)
            .or_insert_with(|| ReviewLoopFinding {
                root_cause_fingerprint: row.root_cause_fingerprint.clone(),
                status: row.status,
                blocking: row.blocking,
                first_seen_head: head.into(),
                last_seen_head: head.into(),
                seen_count: 1,
                reopen_count: 0,
                threads: Vec::new(),
            });
        finding.root_cause_fingerprint = row.root_cause_fingerprint;
        finding.status = row.status;
        finding.blocking = row.blocking;
        finding.last_seen_head = head.into();
        finding.threads.extend(row.threads);
        finding.threads.sort();
        finding.threads.dedup();
    }
    Ok(ReviewLoopTransition {
        state,
        changed: true,
        correction: None,
        supersession: None,
        acceptance: Some(receipt),
    })
}

pub(crate) fn record_state(record: &ReviewStateRecord) -> Option<&ReviewLoopState> {
    match &record.payload {
        ReviewStatePayload::ReviewLoop { state }
        | ReviewStatePayload::ReviewLoopCorrection { state, .. }
        | ReviewStatePayload::ReviewLoopSupersession { state, .. }
        | ReviewStatePayload::ReviewLoopAcceptance { state, .. } => Some(state),
        _ => None,
    }
}

pub(super) fn validate_record(
    prefix: &[ReviewStateRecord],
    record: &ReviewStateRecord,
) -> Result<(), ForgeError> {
    let ReviewStatePayload::ReviewLoopAcceptance { state, acceptance } = &record.payload else {
        return Ok(());
    };
    let chain = ReviewStateChain {
        records: prefix.to_vec(),
        tip_digest: record.previous_digest.clone(),
        ignored_stale_records: Vec::new(),
    };
    let expected = apply(
        &chain,
        &record.expected_head,
        record.previous_digest.as_deref(),
        &acceptance.request,
        &acceptance.findings,
        acceptance.approval_reference.clone(),
    )?;
    if !expected.changed
        || expected.state != *state
        || expected.acceptance.as_ref() != Some(acceptance)
    {
        return Err(invalid(
            "acceptance receipt changed history, counters, or budgets",
        ));
    }
    Ok(())
}
