//! Recovery guidance shared by live observations and non-mutating preflight.
use super::{ForgeError, ReviewLoopHardStop, state_conflict};

pub(super) fn mismatch_error(
    stop: &ReviewLoopHardStop,
    expected_head: &str,
    observation_digest: &str,
) -> ForgeError {
    let recovery = if stop.attempted_head_sha == expected_head {
        format!(
            "replay the original canonical inventory (observation_digest={}) with \
             forge-cli pr review-loop observe <PR> --expected-head {expected_head} \
             --expected-state <current-tip> --findings-file <original-findings.json>{}",
            stop.observation_digest,
            if stop.extension_applied {
                "; the approved extension is already applied and must not be extended again"
            } else {
                "; first obtain the exact maintainer approval and run forge-cli pr review-loop extend for the current stop"
            }
        )
    } else {
        format!(
            "if {}..{expected_head} is a provider-verified signed fast-forward, prepare a \
             forge-cli.review-loop-supersession.v1 findings file with reason signed-fast-forward \
             and superseded_record set to the latest stopped review-loop record digest (not the handoff tip); \
             run forge-cli pr review-loop observe <PR> --expected-head {expected_head} \
             --expected-state <current-tip> --findings-file <supersession.json>; \
             then obtain fresh maintainer approval for the new proposal and run \
             forge-cli pr review-loop extend <PR> --expected-head {expected_head} \
             --expected-state <new-stop-tip> --stop-code <new-stop-code> \
             --budget-field <new-stop-field> --increment <new-stop-increment> \
             --proposal-digest <new-proposal> --approval-comment <new-approval-id>; \
             resume the identical new inventory with observe at the returned tip; \
             an unsupported head change cannot reuse the old approval",
            stop.attempted_head_sha
        )
    };
    // Preflight serializes Display, which omits detail. Keep the actionable
    // commands in the message so both entrypoints expose the same recovery.
    state_conflict(
        &format!(
            "a different observation cannot replace the durable review hard stop; \
             recovery=run forge-cli pr review-loop inspect <PR> --format json; {recovery}; \
             reviewer rebinding does not clear the stop or grant budget"
        ),
        Some(format!(
            "stop_head={}; attempted_head={expected_head}; stop_observation={}; attempted_observation={observation_digest}; extension_applied={}",
            stop.attempted_head_sha, stop.observation_digest, stop.extension_applied
        )),
    )
}
