//! Exact attempted-inventory evidence for a historical alias-bearing stop.
use super::*;

pub(super) fn canonicalize(
    rows: Option<&[ReviewFindingObservation]>,
) -> Result<Option<Vec<ReviewFindingObservation>>, ForgeError> {
    rows.map(|rows| canonical_observations(rows).map(|map| map.into_values().collect()))
        .transpose()
}

pub(super) fn verify(
    corrected: &ReviewLoopState,
    head: &str,
    canonical_rows: &[ReviewFindingObservation],
    proof: Option<&[ReviewFindingObservation]>,
    stop: Option<&ReviewLoopHardStop>,
) -> Result<(), ForgeError> {
    let Some(stop) = stop else {
        return if proof.is_some() {
            Err(invalid(
                "attempted findings proof requires a historical false-reopen stop",
            ))
        } else {
            Ok(())
        };
    };
    let attempted = canonical_observations(proof.unwrap_or(canonical_rows))?;
    if attempted.len() != corrected.findings.len()
        || corrected.findings.iter().any(|(key, finding)| {
            attempted.get(key).is_none_or(|row| {
                row.root_cause_fingerprint != finding.root_cause_fingerprint
                    || row.status != finding.status
                    || row.blocking != finding.blocking
                    || row.threads != finding.threads
            })
        })
        || stop.observation_digest != review_observation_digest(head, &attempted)?
    {
        return Err(invalid(
            "correction requires the identical attempted inventory; retain original alias rows as attempted_findings proof",
        ));
    }
    Ok(())
}
