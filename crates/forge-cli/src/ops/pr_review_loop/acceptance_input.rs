//! Provider approval for a terminal accepted-risk disposition.
use super::*;
use review_state::acceptance::{self, AcceptanceRequest};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptanceEnvelope {
    schema_version: String,
    acceptance: AcceptanceRequest,
    findings: serde_json::Value,
}

pub(super) fn parse(
    value: &serde_json::Value,
) -> Result<
    (
        AcceptanceRequest,
        Vec<review_state::ReviewFindingObservation>,
    ),
    ForgeError,
> {
    let mut value = value.clone();
    if let Some(rows) = value
        .get_mut("acceptance")
        .and_then(|r| r.get_mut("attempted_findings"))
    {
        *rows = serde_json::to_value(observations_from_value(rows)?)
            .map_err(|_| acceptance::invalid("cannot encode attempted inventory"))?;
    }
    let envelope: AcceptanceEnvelope = serde_json::from_value(value)
        .map_err(|_| acceptance::invalid("invalid acceptance envelope fields"))?;
    if envelope.schema_version != acceptance::INPUT_SCHEMA {
        return Err(acceptance::invalid("unsupported acceptance input schema"));
    }
    envelope.acceptance.validate()?;
    Ok((
        envelope.acceptance,
        observations_from_value(&envelope.findings)?,
    ))
}

pub(super) fn transition<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    repository: &str,
    chain: &review_state::ReviewStateChain,
    args: &PrReviewLoopObserveArgs,
    request: &AcceptanceRequest,
    findings: &[review_state::ReviewFindingObservation],
) -> Result<review_state::ReviewLoopTransition, ForgeError> {
    if args.expected_state.is_none()
        || args.expected_state.as_deref() != chain.tip_digest.as_deref()
    {
        return Err(acceptance::invalid(
            "acceptance requires the explicit current state tip",
        ));
    }
    // An identical, already-authorized retry does not require a new approval
    // authored after the acceptance record itself.
    if let Some(review_state::ReviewStatePayload::ReviewLoopAcceptance { acceptance, .. }) = chain
        .records
        .iter()
        .rev()
        .find(|r| review_state::acceptance::record_state(r).is_some())
        .map(|r| &r.payload)
    {
        return acceptance::apply(
            chain,
            &args.expected_head,
            args.expected_state.as_deref(),
            request,
            findings,
            acceptance.approval_reference.clone(),
        );
    }
    let proposal = acceptance::proposal_digest(request, &args.expected_head, findings)?;
    let marker = acceptance::approval_marker(&proposal);
    let id = request
        .approval_comment
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            approval_error(
                format!(
                    "acceptance requires a maintainer approval comment; required_marker={marker}"
                ),
                &marker,
            )
        })?;
    let number = chain
        .records
        .first()
        .ok_or_else(|| acceptance::invalid("acceptance requires an existing stopped state"))?
        .pr;
    let approval = read_approval_comment(runner, ctx, repository, number, id)
        .map_err(|error| map_approval_error(error, &marker))?;
    let state_view = pr_review::read_review_loop_state_view(runner, ctx, repository, number)?;
    ensure_expected_tip(
        state_view.chain.tip_digest.as_deref(),
        chain.tip_digest.as_deref(),
    )?;
    validate_marker_approval(&approval, state_view.tip_created_at.as_deref(), &marker)
        .map_err(|error| map_approval_error(error, &marker))?;
    acceptance::apply(
        chain,
        &args.expected_head,
        args.expected_state.as_deref(),
        request,
        findings,
        approval.url,
    )
}

fn approval_error(message: String, marker: &str) -> ForgeError {
    ForgeError::validation_with_details(
        schema_err(),
        "review_acceptance_approval_invalid",
        message,
        serde_json::json!({"retryable":false,"next_action":"obtain-maintainer-approval","recovery":{"approval_marker":marker,"requires_exact_acceptance_marker":true,"requires_post_stop_approval":true}}),
    )
}

fn map_approval_error(error: ForgeError, marker: &str) -> ForgeError {
    match error {
        ForgeError::Validation {
            message, detail, ..
        } => approval_error(
            match detail {
                Some(detail) => format!("{message}; {detail}"),
                None => message,
            },
            marker,
        ),
        other => other,
    }
}
