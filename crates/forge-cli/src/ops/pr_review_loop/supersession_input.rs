//! Provider evidence for an explicit stopped-attempt supersession.
use super::*;
use review_state::supersession::{self, SupersessionRequest};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SupersessionEnvelope {
    pub schema_version: String,
    pub supersession: SupersessionRequest,
    pub findings: serde_json::Value,
}

pub(super) fn transition<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    repository: &str,
    chain: &review_state::ReviewStateChain,
    args: &PrReviewLoopObserveArgs,
    request: &SupersessionRequest,
    findings: &[review_state::ReviewFindingObservation],
) -> Result<review_state::ReviewLoopTransition, ForgeError> {
    if args.expected_state.is_none()
        || args.expected_state.as_deref() != chain.tip_digest.as_deref()
    {
        return Err(supersession::invalid(
            "supersession requires the explicit current state tip",
        ));
    }
    let source = supersession::source_head(chain, request)?;
    if !supersession::valid_sha(source)
        || !supersession::valid_sha(&args.expected_head)
        || source == args.expected_head
    {
        return Err(supersession::invalid(
            "supersession requires distinct full commit IDs",
        ));
    }
    let commits = verified_fast_forward(runner, ctx, repository, source, &args.expected_head)?;
    supersession::apply(
        chain,
        &args.expected_head,
        args.expected_state.as_deref(),
        request,
        findings,
        &commits,
    )
}

fn verified_fast_forward<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    repository: &str,
    from: &str,
    to: &str,
) -> Result<Vec<String>, ForgeError> {
    let mut commits = Vec::new();
    let mut expected_total = None;
    for page in 1..=supersession::MAX_VERIFIED_COMMITS.div_ceil(100) {
        let mut argv = vec![OsString::from("api")];
        ctx.push_github_api_hostname(&mut argv);
        argv.push(
            format!("repos/{repository}/compare/{from}...{to}?per_page=100&page={page}").into(),
        );
        let output = runner.run(&BackendCall::new(BackendProgram::Gh, argv))?;
        let value: serde_json::Value =
            serde_json::from_str(output.stdout.trim()).map_err(|_| {
                supersession::invalid("provider fast-forward response is not valid JSON")
            })?;
        let total = value["total_commits"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= supersession::MAX_VERIFIED_COMMITS as u64)
            .ok_or_else(|| {
                supersession::invalid("signed fast-forward must contain between 1 and 100 commits")
            })?;
        if value["status"] != "ahead"
            || value["behind_by"] != 0
            || value["ahead_by"].as_u64() != Some(total)
            || value.pointer("/base_commit/sha").and_then(|v| v.as_str()) != Some(from)
            || value
                .pointer("/merge_base_commit/sha")
                .and_then(|v| v.as_str())
                != Some(from)
            || expected_total.is_some_and(|expected| expected != total)
        {
            return Err(supersession::invalid(
                "provider comparison does not prove the stopped head is an ancestor",
            ));
        }
        expected_total = Some(total);
        let rows = value["commits"]
            .as_array()
            .filter(|rows| !rows.is_empty() && rows.len() <= 100)
            .ok_or_else(|| {
                supersession::invalid("provider fast-forward commit page is incomplete")
            })?;
        for row in rows {
            if row
                .pointer("/commit/verification/verified")
                .and_then(|v| v.as_bool())
                != Some(true)
                || row
                    .pointer("/commit/verification/reason")
                    .and_then(|v| v.as_str())
                    != Some("valid")
            {
                return Err(supersession::invalid(
                    "every fast-forward commit requires a verified valid signature",
                ));
            }
            let sha = row["sha"]
                .as_str()
                .filter(|sha| supersession::valid_sha(sha))
                .ok_or_else(|| {
                    supersession::invalid("provider fast-forward commit ID is invalid")
                })?;
            if sha == from || commits.iter().any(|old| old == sha) {
                return Err(supersession::invalid(
                    "provider fast-forward proof repeats a commit",
                ));
            }
            commits.push(sha.to_string());
        }
        if commits.len() as u64 > total {
            return Err(supersession::invalid(
                "provider fast-forward proof exceeds its count",
            ));
        }
        if commits.len() as u64 == total {
            if commits.last().map(String::as_str) != Some(to) {
                return Err(supersession::invalid(
                    "provider fast-forward proof does not end at the expected head",
                ));
            }
            return Ok(commits);
        }
        if rows.len() != 100 {
            return Err(supersession::invalid(
                "provider fast-forward proof is truncated",
            ));
        }
    }
    Err(supersession::invalid(
        "provider fast-forward proof exceeds its bounded limit",
    ))
}
