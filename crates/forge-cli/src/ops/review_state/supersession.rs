//! Append-only replacement of a stopped attempt after a verified signed fast-forward.
use super::*;

pub const INPUT_SCHEMA: &str = "forge-cli.review-loop-supersession.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SupersessionRequest {
    pub superseded_record: String,
    pub reason: SupersessionReason,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SupersessionReason {
    SignedFastForward,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SupersessionReceipt {
    pub request: SupersessionRequest,
    pub from_head: String,
    /// Every newly reachable commit was verified by the provider before append.
    pub verified_commits: Vec<String>,
    pub findings: Vec<ReviewFindingObservation>,
}

pub fn invalid(message: &str) -> ForgeError {
    ForgeError::validation(error_schema(), "review_supersession_invalid", message, None)
}

pub fn valid_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl SupersessionRequest {
    pub fn validate(&self) -> Result<(), ForgeError> {
        let Some(digest) = self.superseded_record.strip_prefix("sha256:") else {
            return Err(invalid(
                "superseded record must be a canonical record digest",
            ));
        };
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err(invalid(
                "superseded record must be a canonical record digest",
            ));
        }
        Ok(())
    }
}

pub fn source_head<'a>(
    chain: &'a ReviewStateChain,
    request: &SupersessionRequest,
) -> Result<&'a str, ForgeError> {
    request.validate()?;
    let target = chain
        .records
        .iter()
        .find(|r| r.record_digest == request.superseded_record)
        .ok_or_else(|| invalid("superseded record is not in the current chain"))?;
    let state = record_state(target)
        .ok_or_else(|| invalid("superseded record is not a review-loop state"))?;
    state
        .hard_stop
        .as_ref()
        .map(|s| s.attempted_head_sha.as_str())
        .ok_or_else(|| invalid("supersession requires a durable budget hard stop"))
}

fn record_state(record: &ReviewStateRecord) -> Option<&ReviewLoopState> {
    match &record.payload {
        ReviewStatePayload::ReviewLoop { state }
        | ReviewStatePayload::ReviewLoopCorrection { state, .. }
        | ReviewStatePayload::ReviewLoopSupersession { state, .. } => Some(state),
        _ => None,
    }
}

pub fn apply(
    chain: &ReviewStateChain,
    head: &str,
    expected_tip: Option<&str>,
    request: &SupersessionRequest,
    observations: &[ReviewFindingObservation],
    verified_commits: &[String],
) -> Result<ReviewLoopTransition, ForgeError> {
    request.validate()?;
    if expected_tip.is_none() || expected_tip != chain.tip_digest.as_deref() {
        return Err(invalid(
            "supersession requires the explicit current state tip",
        ));
    }
    let from_head = source_head(chain, request)?;
    if !valid_sha(from_head) || !valid_sha(head) || from_head == head {
        return Err(invalid("supersession requires distinct full commit IDs"));
    }
    if verified_commits.is_empty()
        || verified_commits.last().map(String::as_str) != Some(head)
        || verified_commits
            .iter()
            .any(|sha| !valid_sha(sha) || sha == from_head)
        || verified_commits.iter().collect::<BTreeSet<_>>().len() != verified_commits.len()
    {
        return Err(invalid("signed fast-forward proof is incomplete"));
    }
    let canonical = canonical_observations(observations)?;
    let receipt = SupersessionReceipt {
        request: request.clone(),
        from_head: from_head.into(),
        verified_commits: verified_commits.to_vec(),
        findings: canonical.values().cloned().collect(),
    };
    let latest = chain
        .records
        .iter()
        .rev()
        .find(|r| record_state(r).is_some())
        .ok_or_else(|| invalid("supersession requires an existing state"))?;
    if let ReviewStatePayload::ReviewLoopSupersession {
        state,
        supersession,
    } = &latest.payload
        && supersession == &receipt
        && latest.expected_head == head
    {
        return Ok(ReviewLoopTransition {
            state: state.clone(),
            changed: false,
            correction: None,
            supersession: Some(receipt),
        });
    }
    if latest.record_digest != request.superseded_record {
        return Err(invalid(
            "supersession target must be the latest review-loop state",
        ));
    }
    let previous = record_state(latest).expect("latest state exists");
    let stop = previous
        .hard_stop
        .as_ref()
        .ok_or_else(|| invalid("supersession requires a durable budget hard stop"))?;
    if previous
        .findings
        .iter()
        .any(|(key, f)| f.status == ReviewFindingStatus::Open && !canonical.contains_key(key))
    {
        return Err(invalid(
            "supersession inventory must include every open finding",
        ));
    }
    let observation_digest = review_observation_digest(head, &canonical)?;
    // Bind the new proposal to both the consumed ledger tip and the new attempt.
    let binding = sha256_digest(
        &serde_json::to_vec(&(INPUT_SCHEMA, expected_tip, &observation_digest))
            .map_err(|_| invalid("cannot encode supersession proposal"))?,
    );
    let proposal_digest =
        extension_proposal_digest(&binding, &stop.code, &stop.budget_field, stop.increment)?;
    let mut state = previous.clone();
    state.hard_stop = Some(ReviewLoopHardStop {
        attempted_head_sha: head.into(),
        observation_digest,
        proposal_digest,
        extension_applied: false,
        ..stop.clone()
    });
    Ok(ReviewLoopTransition {
        state,
        changed: true,
        correction: None,
        supersession: Some(receipt),
    })
}

pub(super) fn validate_record(
    prefix: &[ReviewStateRecord],
    record: &ReviewStateRecord,
) -> Result<(), ForgeError> {
    let ReviewStatePayload::ReviewLoopSupersession {
        state,
        supersession,
    } = &record.payload
    else {
        return Ok(());
    };
    let chain = ReviewStateChain {
        records: prefix.to_vec(),
        tip_digest: prefix.last().map(|r| r.record_digest.clone()),
        ignored_stale_records: Vec::new(),
    };
    if supersession.from_head != source_head(&chain, &supersession.request)? {
        return Err(invalid(
            "supersession source head does not match its target",
        ));
    }
    let expected = apply(
        &chain,
        &record.expected_head,
        record.previous_digest.as_deref(),
        &supersession.request,
        &supersession.findings,
        &supersession.verified_commits,
    )?;
    if !expected.changed
        || expected.state != *state
        || expected.supersession.as_ref() != Some(supersession)
    {
        return Err(invalid(
            "supersession receipt changed history, counters, or budgets",
        ));
    }
    Ok(())
}
