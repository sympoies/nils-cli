//! Exact published or draft release lookup within the selected GitHub repository.
use super::super::super::github_write::invalid;
use super::{api, release_path};
use crate::backend::BackendCall;
use crate::error::ForgeError;
use crate::provider::ProviderContext;
use serde_json::Value;

pub(super) fn draft_call(ctx: &ProviderContext, repo: &str, tag: &str) -> BackendCall {
    let (owner, name) = repo.split_once('/').expect("GitHub repository owner/name");
    let mut call = api::call(ctx, "graphql", "POST");
    // GitHub's REST tag-name endpoint is published-only. GraphQL resolves a
    // draft by pending tag; its database ID is then read through REST as usual.
    // Raw fields keep every variable literal, including tag names starting @.
    for value in [
        "query=query($owner:String!,$name:String!,$tag:String!){repository(owner:$owner,name:$name){release(tagName:$tag){databaseId}}}".to_string(),
        format!("owner={owner}"), format!("name={name}"), format!("tag={tag}"),
    ] {
        call.argv.extend(["--raw-field".into(), value.into()]);
    }
    call
}

pub(super) fn find(
    ctx: &ProviderContext,
    repo: &str,
    tag: &str,
    op: &str,
) -> Result<Option<Value>, ForgeError> {
    if let Some(release) = api::get(ctx, op, &release_path(repo, tag))? {
        return Ok(Some(release));
    }
    let value = api::request(op, &draft_call(ctx, repo, tag))?.ok_or_else(|| {
        invalid(
            op,
            "release_response_invalid",
            "draft release lookup returned no GraphQL response; check repository read access",
        )
    })?;
    let repository = &value["data"]["repository"];
    if value
        .get("errors")
        .is_some_and(|errors| errors.as_array().is_none_or(|a| !a.is_empty()))
        || !repository.is_object()
        || repository.get("release").is_none()
    {
        return Err(invalid(
            op,
            "release_response_invalid",
            "draft release lookup failed or returned an incomplete response; check repository read access",
        ));
    }
    if repository["release"].is_null() {
        return Ok(None);
    }
    let id = repository["release"]["databaseId"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            invalid(
                op,
                "release_response_invalid",
                "draft release lookup omitted a positive release database ID",
            )
        })?;
    let release = api::get(ctx, op, &format!("repos/{repo}/releases/{id}"))?.ok_or_else(|| {
        invalid(
            op,
            "release_response_mismatch",
            "release disappeared after draft lookup; inspect the exact tag before retrying",
        )
    })?;
    if release["id"].as_u64() != Some(id) || release["tag_name"].as_str() != Some(tag) {
        return Err(invalid(
            op,
            "release_response_mismatch",
            "draft release identity differs from the selected tag; no mutation is safe",
        ));
    }
    Ok(Some(release))
}
