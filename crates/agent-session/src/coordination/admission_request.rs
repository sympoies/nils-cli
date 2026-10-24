//! Shared admission validation and digest, also used by non-executing preparation.
use super::*;

pub(in crate::coordination) struct AdmissionRequest {
    pub targets: Vec<Scope>,
    pub provider_targets: Vec<ProviderRef>,
    pub pull_request_targets: Vec<PullRequestTarget>,
    pub checkouts: Vec<CheckoutBinding>,
    pub descendant: Option<DescendantIdentity>,
    pub token_digest: String,
    pub digest: String,
}

pub(in crate::coordination) fn prepare(
    args: &WorkContextAdmitArgs,
) -> Result<AdmissionRequest, CliError> {
    validate_operation_kind(&args.operation)?;
    let execution_token =
        read_private_text(&args.execution_token_file, 256, "invalid-execution-token")?;
    validate_execution_token(&execution_token)?;
    let input: OperationTargetsInput =
        read_bounded_json(&args.targets_file, 64 * 1024, "invalid-scope")?;
    if input.schema_version != "agent-session.operation-targets.v1" {
        return Err(CliError::data(
            "invalid-scope",
            "operation target schema is unsupported",
            None,
        ));
    }
    validate_descendant(input.descendant.as_ref())?;
    let targets = canonicalize_targets(input.targets)?;
    let provider_targets = canonicalize_provider_refs(input.provider_refs)?;
    let pull_request_targets = canonicalize_pull_request_targets(input.pull_requests)?;
    if targets.is_empty() && provider_targets.is_empty() && pull_request_targets.is_empty() {
        return Err(CliError::data(
            "invalid-scope",
            "operation targets must name at least one filesystem or provider mutation",
            None,
        ));
    }
    let mut digest_input = json!({
        "claim": args.claim,
        "if_revision": args.if_revision,
        "operation": args.operation,
        "targets": targets,
        "provider_targets": provider_targets,
        "checkouts": input.checkouts,
        "descendant": input.descendant,
        "execution_token_digest": digest_bytes(execution_token.as_bytes()),
    });
    // Omitted when empty so requests without pull-request targets keep the
    // digest released builds computed for them.
    if !pull_request_targets.is_empty() {
        digest_input["pull_request_targets"] = json!(pull_request_targets);
    }
    let digest = request_digest("work-context-admit", &digest_input);
    Ok(AdmissionRequest {
        targets,
        provider_targets,
        pull_request_targets,
        checkouts: input.checkouts,
        descendant: input.descendant,
        token_digest: digest_bytes(execution_token.as_bytes()),
        digest,
    })
}
