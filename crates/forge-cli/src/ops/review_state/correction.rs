//! Append-only recovery for findings the previous omission transition fixed.
use super::*;

mod attempted_inventory;

pub const INPUT_SCHEMA: &str = "forge-cli.review-loop-correction.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CorrectionRequest {
    pub corrected_record: String,
    pub reason: CorrectionReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempted_findings: Option<Vec<ReviewFindingObservation>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CorrectionReason {
    OmittedFindings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CorrectionReceipt {
    pub corrected_record: String,
    pub reason: CorrectionReason,
    pub restored_findings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempted_findings: Option<Vec<ReviewFindingObservation>>,
}

pub fn invalid(message: &str) -> ForgeError {
    ForgeError::validation(error_schema(), "review_correction_invalid", message, None)
}

impl CorrectionRequest {
    pub fn validate(&self) -> Result<(), ForgeError> {
        if !self
            .corrected_record
            .strip_prefix("sha256:")
            .is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
        {
            return Err(invalid(
                "correction must name a canonical state record digest",
            ));
        }
        attempted_inventory::canonicalize(self.attempted_findings.as_deref())?;
        Ok(())
    }
}

fn state(record: &ReviewStateRecord) -> Option<&ReviewLoopState> {
    match &record.payload {
        ReviewStatePayload::ReviewLoop { state }
        | ReviewStatePayload::ReviewLoopCorrection { state, .. }
        | ReviewStatePayload::ReviewLoopAcceptance { state, .. }
        | ReviewStatePayload::ReviewLoopSupersession { state, .. } => Some(state),
        _ => None,
    }
}

fn context<'a>(
    records: &'a [ReviewStateRecord],
    head: &str,
    request: &CorrectionRequest,
) -> Result<(&'a ReviewLoopState, &'a ReviewLoopState), ForgeError> {
    request.validate()?;
    let index = records
        .iter()
        .position(|record| record.record_digest == request.corrected_record)
        .ok_or_else(|| invalid("corrected record is absent from the current chain"))?;
    let ReviewStatePayload::ReviewLoop { state: target } = &records[index].payload else {
        return Err(invalid(
            "correction must target an ordinary observation record",
        ));
    };
    let prior = records[..index]
        .iter()
        .rev()
        .find_map(state)
        .ok_or_else(|| invalid("a genesis observation cannot contain inferred omissions"))?;
    let latest = records.iter().rev().find_map(state).unwrap();
    if target.head_sha != head
        || latest.head_sha != head
        || prior.head_sha == head
        || target.hard_stop.is_some()
    {
        return Err(invalid(
            "correction requires the erroneous observation's current head",
        ));
    }
    let mut without_stop = latest.clone();
    if let Some(stop) = without_stop.hard_stop.take()
        && (stop.code != "review_finding_reopened"
            || stop.attempted_head_sha != head
            || stop.extension_applied)
    {
        return Err(invalid(
            "correction cannot clear an unrelated or extended budget stop",
        ));
    }
    if without_stop != *target {
        return Err(invalid(
            "a later observation changed the state being corrected",
        ));
    }
    Ok((target, prior))
}

fn restore(
    records: &[ReviewStateRecord],
    head: &str,
    receipt: &CorrectionReceipt,
) -> Result<ReviewLoopState, ForgeError> {
    if attempted_inventory::canonicalize(receipt.attempted_findings.as_deref())?
        != receipt.attempted_findings
    {
        return Err(invalid(
            "correction receipt requires canonical attempted proof",
        ));
    }
    let request = CorrectionRequest {
        corrected_record: receipt.corrected_record.clone(),
        reason: receipt.reason.clone(),
        attempted_findings: receipt.attempted_findings.clone(),
    };
    let (target, prior) = context(records, head, &request)?;
    if receipt.restored_findings.is_empty()
        || receipt
            .restored_findings
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid(
            "correction must list sorted unique omitted identities",
        ));
    }
    let mut corrected = target.clone();
    for fingerprint in &receipt.restored_findings {
        let before = prior
            .findings
            .get(fingerprint)
            .filter(|finding| finding.status == ReviewFindingStatus::Open)
            .ok_or_else(|| invalid("restored finding was not open before the observation"))?;
        let mut inferred = before.clone();
        inferred.status = ReviewFindingStatus::Fixed;
        if target.findings.get(fingerprint) != Some(&inferred) {
            return Err(invalid("restored finding is not a chain-proven omission"));
        }
        corrected
            .findings
            .insert(fingerprint.clone(), before.clone());
    }
    let inventory_rows: Vec<_> = corrected
        .findings
        .iter()
        .map(|(key, finding)| ReviewFindingObservation {
            fingerprint: key.clone(),
            root_cause_fingerprint: finding.root_cause_fingerprint.clone(),
            blocking: finding.blocking,
            status: finding.status,
            threads: finding.threads.clone(),
        })
        .collect();
    attempted_inventory::verify(
        &corrected,
        head,
        &inventory_rows,
        receipt.attempted_findings.as_deref(),
        records
            .iter()
            .rev()
            .find_map(state)
            .and_then(|state| state.hard_stop.as_ref()),
    )?;
    let blocking = |state: &ReviewLoopState| {
        state
            .findings
            .values()
            .filter(|finding| finding.status == ReviewFindingStatus::Open && finding.blocking)
            .count()
    };
    if blocking(&corrected) > 0 && blocking(&corrected) >= blocking(prior) {
        corrected.no_progress_rounds = corrected
            .no_progress_rounds
            .max(prior.no_progress_rounds.saturating_add(1));
    }
    if corrected.no_progress_rounds > corrected.budget.max_no_progress_rounds {
        let rows: Vec<_> = corrected
            .findings
            .iter()
            .map(|(key, finding)| ReviewFindingObservation {
                fingerprint: key.clone(),
                root_cause_fingerprint: finding.root_cause_fingerprint.clone(),
                blocking: finding.blocking,
                status: finding.status,
                threads: finding.threads.clone(),
            })
            .collect();
        let error = review_stop(
            "review_no_progress",
            "corrected inventory retains an exhausted no-progress budget",
            format!("no_progress_rounds={}", corrected.no_progress_rounds),
        );
        corrected = record_review_loop_hard_stop(
            &corrected,
            head,
            &rows,
            &records.last().unwrap().record_digest,
            &error,
        )?;
    }
    Ok(corrected)
}

fn matches_inventory(
    state: &ReviewLoopState,
    inventory: &BTreeMap<String, ReviewFindingObservation>,
) -> bool {
    state.findings.len() == inventory.len()
        && state.findings.iter().all(|(key, finding)| {
            inventory.get(key).is_some_and(|row| {
                row.fingerprint == *key
                    && row.status == finding.status
                    && row.blocking == finding.blocking
                    && row.root_cause_fingerprint == finding.root_cause_fingerprint
                    && row.threads == finding.threads
            })
        })
}

pub fn apply(
    chain: &ReviewStateChain,
    head: &str,
    expected_tip: Option<&str>,
    request: &CorrectionRequest,
    observations: &[ReviewFindingObservation],
) -> Result<ReviewLoopTransition, ForgeError> {
    request.validate()?;
    if expected_tip.is_none() || expected_tip != chain.tip_digest.as_deref() {
        return Err(invalid(
            "correction requires an explicit current --expected-state tip",
        ));
    }
    let inventory = canonical_observations(observations)?;
    let attempted_findings =
        attempted_inventory::canonicalize(request.attempted_findings.as_deref())?;
    if let Some(record) = chain
        .records
        .iter()
        .rev()
        .find(|record| state(record).is_some())
        && let ReviewStatePayload::ReviewLoopCorrection { state, correction } = &record.payload
        && correction.corrected_record == request.corrected_record
        && correction.reason == request.reason
        && correction.attempted_findings == attempted_findings
        && state.head_sha == head
        && matches_inventory(state, &inventory)
    {
        if super::super::pr_review_handoff::latest(chain).is_some_and(|handoff| {
            record.assignment_generation != Some(handoff.assignment_generation)
        }) {
            return Err(invalid(
                "a correction retry from an earlier assignment cannot establish a new interval; observe its ordinary inventory first",
            ));
        }
        return Ok(ReviewLoopTransition {
            state: state.clone(),
            changed: false,
            correction: Some(correction.clone()),
            supersession: None,
            acceptance: None,
        });
    }
    let (target, _) = context(&chain.records, head, request)?;
    if target.findings.len() != inventory.len()
        || target
            .findings
            .keys()
            .any(|key| !inventory.contains_key(key))
    {
        return Err(invalid(
            "correction requires the complete finding inventory",
        ));
    }
    let restored_findings = inventory
        .iter()
        .filter(|(key, row)| {
            row.status == ReviewFindingStatus::Open
                && target.findings[*key].status == ReviewFindingStatus::Fixed
        })
        .map(|(key, _)| key.clone())
        .collect();
    let correction = CorrectionReceipt {
        corrected_record: request.corrected_record.clone(),
        reason: request.reason.clone(),
        restored_findings,
        attempted_findings,
    };
    let corrected = restore(&chain.records, head, &correction)?;
    if !matches_inventory(&corrected, &inventory) {
        return Err(invalid(
            "correction may only restore proven omitted findings without changing other dispositions or metadata",
        ));
    }
    Ok(ReviewLoopTransition {
        state: corrected,
        changed: true,
        correction: Some(correction),
        supersession: None,
        acceptance: None,
    })
}

pub(super) fn validate_record(
    previous: &[ReviewStateRecord],
    record: &ReviewStateRecord,
) -> Result<(), ForgeError> {
    if let ReviewStatePayload::ReviewLoopCorrection { state, correction } = &record.payload
        && restore(previous, &record.expected_head, correction)? != *state
    {
        return Err(invalid(
            "correction record changed counters, budgets, or unrelated findings",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn fixture() -> (
        ReviewStateChain,
        CorrectionRequest,
        Vec<ReviewFindingObservation>,
    ) {
        let rows: Vec<_> = (1..=2)
            .map(|i| ReviewFindingObservation {
                fingerprint: format!("correctness:fixture:f{i}"),
                root_cause_fingerprint: None,
                blocking: true,
                status: ReviewFindingStatus::Open,
                threads: vec![format!("thread-{i}")],
            })
            .collect();
        let before = observe_review_loop(None, "head-1", &rows).unwrap().state;
        let genesis = ReviewStateRecord::new(
            "acme/widgets",
            7,
            "head-1",
            0,
            None,
            ReviewStatePayload::ReviewLoop {
                state: before.clone(),
            },
        )
        .unwrap();
        let mut bad = before;
        bad.head_sha = "head-2".into();
        bad.round = 1;
        for finding in bad.findings.values_mut() {
            finding.status = ReviewFindingStatus::Fixed;
        }
        let explicit = bad.findings.get_mut(&rows[1].fingerprint).unwrap();
        explicit.blocking = false;
        explicit.last_seen_head = "head-2".into();
        explicit.seen_count = 2;
        let erroneous = ReviewStateRecord::new(
            "acme/widgets",
            7,
            "head-2",
            1,
            Some(genesis.record_digest.clone()),
            ReviewStatePayload::ReviewLoop { state: bad },
        )
        .unwrap();
        let request = CorrectionRequest {
            corrected_record: erroneous.record_digest.clone(),
            reason: CorrectionReason::OmittedFindings,
            attempted_findings: None,
        };
        let mut rows = rows;
        rows[1].status = ReviewFindingStatus::Fixed;
        rows[1].blocking = false;
        let chain = parse_chain(
            [genesis.marker().unwrap(), erroneous.marker().unwrap()]
                .iter()
                .map(String::as_str),
            "acme/widgets",
            7,
        )
        .unwrap();
        (chain, request, rows)
    }

    fn append(chain: &ReviewStateChain, transition: ReviewLoopTransition) -> ReviewStateRecord {
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            "head-2",
            chain.records.len() as u64,
            chain.tip_digest.clone(),
            transition.into_payload(),
        )
        .unwrap()
    }

    #[test]
    fn correction_preserves_history_counters_budgets_and_blocks_merge() {
        let (chain, request, rows) = fixture();
        let originals: Vec<_> = chain.records.iter().map(|r| r.marker().unwrap()).collect();
        let old = latest_review_loop_state(&chain).unwrap();
        let transition = apply(
            &chain,
            "head-2",
            chain.tip_digest.as_deref(),
            &request,
            &rows,
        )
        .unwrap();
        assert_eq!(transition.state.round, old.round);
        assert_eq!(transition.state.no_progress_rounds, old.no_progress_rounds);
        assert_eq!(transition.state.budget, old.budget);
        assert_eq!(transition.state.extensions, old.extensions);
        assert_eq!(
            transition.state.findings[&rows[0].fingerprint].reopen_count,
            0
        );
        assert_eq!(
            transition.state.findings[&rows[0].fingerprint].seen_count,
            1
        );
        assert_eq!(
            transition.state.findings[&rows[0].fingerprint].last_seen_head,
            "head-1"
        );
        assert!(transition.state.findings[&rows[0].fingerprint].blocking);
        let correction = append(&chain, transition);
        let mut markers = originals.clone();
        markers.push(correction.marker().unwrap());
        let corrected = parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7).unwrap();
        assert_eq!(
            corrected.records[..2]
                .iter()
                .map(|r| r.marker().unwrap())
                .collect::<Vec<_>>(),
            originals
        );
        assert_eq!(
            latest_review_loop_state(&corrected).unwrap().findings[&rows[0].fingerprint].status,
            ReviewFindingStatus::Open
        );
        let retry = apply(
            &corrected,
            "head-2",
            corrected.tip_digest.as_deref(),
            &request,
            &rows,
        )
        .unwrap();
        assert!(!retry.changed);
    }

    #[test]
    fn correction_rejects_stale_tip_head_record_and_partial_inventory() {
        let (chain, request, rows) = fixture();
        for tip in [None, Some("sha256:stale")] {
            assert_eq!(
                apply(&chain, "head-2", tip, &request, &rows)
                    .unwrap_err()
                    .kind(),
                "review_correction_invalid"
            );
        }
        assert!(
            apply(
                &chain,
                "head-3",
                chain.tip_digest.as_deref(),
                &request,
                &rows
            )
            .is_err()
        );
        let absent = CorrectionRequest {
            corrected_record: format!("sha256:{}", "0".repeat(64)),
            ..request.clone()
        };
        assert!(
            apply(
                &chain,
                "head-2",
                chain.tip_digest.as_deref(),
                &absent,
                &rows
            )
            .is_err()
        );
        assert!(
            apply(
                &chain,
                "head-2",
                chain.tip_digest.as_deref(),
                &request,
                &rows[..1]
            )
            .is_err()
        );
    }

    #[test]
    fn correction_cannot_restore_explicit_fixed_or_change_unrelated_metadata() {
        let (chain, request, rows) = fixture();
        let mut genuine = rows.clone();
        genuine[1].status = ReviewFindingStatus::Open;
        genuine[1].blocking = true;
        assert!(
            apply(
                &chain,
                "head-2",
                chain.tip_digest.as_deref(),
                &request,
                &genuine
            )
            .is_err()
        );
        let mut altered = rows.clone();
        altered[0].threads.push("extra-thread".into());
        assert!(
            apply(
                &chain,
                "head-2",
                chain.tip_digest.as_deref(),
                &request,
                &altered
            )
            .is_err()
        );
        let mut downgraded = rows;
        downgraded[0].blocking = false;
        assert!(
            apply(
                &chain,
                "head-2",
                chain.tip_digest.as_deref(),
                &request,
                &downgraded
            )
            .is_err()
        );
    }

    #[test]
    fn chain_rejects_correction_records_that_mutate_budgets_or_hide_history() {
        let (chain, request, rows) = fixture();
        let transition = apply(
            &chain,
            "head-2",
            chain.tip_digest.as_deref(),
            &request,
            &rows,
        )
        .unwrap();
        let mut payload = transition.into_payload();
        if let ReviewStatePayload::ReviewLoopCorrection { state, .. } = &mut payload {
            state.budget.max_auto_reopens_per_fingerprint = 1;
        }
        let forged = ReviewStateRecord::new(
            "acme/widgets",
            7,
            "head-2",
            2,
            chain.tip_digest.clone(),
            payload,
        )
        .unwrap();
        let mut markers: Vec<_> = chain.records.iter().map(|r| r.marker().unwrap()).collect();
        markers.push(forged.marker().unwrap());
        assert_eq!(
            parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7)
                .unwrap_err()
                .kind(),
            "review_correction_invalid"
        );
        assert!(validate_record(&[], &forged).is_err());
    }

    #[test]
    fn only_the_false_reopen_stop_can_be_cleared_without_extension() {
        let (mut chain, request, rows) = fixture();
        let prior = latest_review_loop_state(&chain).unwrap();
        let error = observe_review_loop(Some(prior), "head-2", &rows).unwrap_err();
        assert_eq!(error.kind(), "review_finding_reopened");
        let stopped = record_review_loop_hard_stop(
            prior,
            "head-2",
            &rows,
            chain.tip_digest.as_deref().unwrap(),
            &error,
        )
        .unwrap();
        let record = ReviewStateRecord::new(
            "acme/widgets",
            7,
            "head-2",
            2,
            chain.tip_digest.clone(),
            ReviewStatePayload::ReviewLoop { state: stopped },
        )
        .unwrap();
        chain.tip_digest = Some(record.record_digest.clone());
        chain.records.push(record);
        let transition = apply(
            &chain,
            "head-2",
            chain.tip_digest.as_deref(),
            &request,
            &rows,
        )
        .unwrap();
        assert!(transition.state.hard_stop.is_none());
        assert_eq!(transition.state.budget.max_auto_reopens_per_fingerprint, 0);
        let last = chain.records.last_mut().unwrap();
        if let ReviewStatePayload::ReviewLoop { state } = &mut last.payload {
            state.hard_stop.as_mut().unwrap().code = "review_no_progress".into();
        }
        assert!(
            apply(
                &chain,
                "head-2",
                chain.tip_digest.as_deref(),
                &request,
                &rows
            )
            .is_err()
        );
    }

    #[test]
    fn omission_correction_restores_lost_no_progress_debt_and_keeps_the_stop() {
        let (_, _, mut rows) = fixture();
        rows[1].status = ReviewFindingStatus::Open;
        rows[1].blocking = true;
        let mut previous = None;
        let mut records: Vec<ReviewStateRecord> = Vec::new();
        for head in ["head-1", "head-2", "head-3"] {
            let state = observe_review_loop(previous.as_ref(), head, &rows)
                .unwrap()
                .state;
            records.push(
                ReviewStateRecord::new(
                    "acme/widgets",
                    7,
                    head,
                    records.len() as u64,
                    records.last().map(|r| r.record_digest.clone()),
                    ReviewStatePayload::ReviewLoop {
                        state: state.clone(),
                    },
                )
                .unwrap(),
            );
            previous = Some(state);
        }
        let mut erroneous = previous.unwrap();
        erroneous.head_sha = "head-4".into();
        erroneous.round += 1;
        erroneous.no_progress_rounds = 0;
        for finding in erroneous.findings.values_mut() {
            finding.status = ReviewFindingStatus::Fixed;
        }
        records.push(
            ReviewStateRecord::new(
                "acme/widgets",
                7,
                "head-4",
                3,
                records.last().map(|r| r.record_digest.clone()),
                ReviewStatePayload::ReviewLoop { state: erroneous },
            )
            .unwrap(),
        );
        let request = CorrectionRequest {
            corrected_record: records.last().unwrap().record_digest.clone(),
            reason: CorrectionReason::OmittedFindings,
            attempted_findings: None,
        };
        let markers: Vec<_> = records.iter().map(|r| r.marker().unwrap()).collect();
        let chain = parse_chain(markers.iter().map(String::as_str), "acme/widgets", 7).unwrap();
        let corrected = apply(
            &chain,
            "head-4",
            chain.tip_digest.as_deref(),
            &request,
            &rows,
        )
        .unwrap();
        assert_eq!(corrected.state.no_progress_rounds, 3);
        assert_eq!(corrected.state.budget.max_no_progress_rounds, 2);
        assert_eq!(corrected.state.budget.max_auto_reopens_per_fingerprint, 0);
        assert_eq!(
            corrected.state.hard_stop.as_ref().unwrap().code,
            "review_no_progress"
        );
        let record = ReviewStateRecord::new(
            "acme/widgets",
            7,
            "head-4",
            4,
            chain.tip_digest.clone(),
            corrected.into_payload(),
        )
        .unwrap();
        assert!(state_comment_visible_metadata(&record).contains("paused by its budget stop"));
        let mut complete = markers;
        complete.push(record.marker().unwrap());
        let corrected_chain =
            parse_chain(complete.iter().map(String::as_str), "acme/widgets", 7).unwrap();
        let paused = latest_review_loop_state(&corrected_chain).unwrap();
        let stop = paused.hard_stop.as_ref().unwrap();
        let approved = apply_review_loop_extension(
            paused,
            stop.proposal_digest.clone(),
            "https://github.com/acme/widgets/pull/7#issuecomment-10".into(),
            "review_no_progress",
            "max_no_progress_rounds",
            1,
        )
        .unwrap();
        let resumed = observe_review_loop(Some(&approved), "head-4", &rows).unwrap();
        assert!(
            resumed.changed,
            "approved same-head resume must durably clear the stop"
        );
        assert!(resumed.state.hard_stop.is_none());
        assert_eq!(resumed.state.no_progress_rounds, 3);
        assert_eq!(resumed.state.budget.max_auto_reopens_per_fingerprint, 0);
    }
}
