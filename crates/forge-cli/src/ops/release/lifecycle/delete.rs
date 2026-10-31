//! Sequential release deletion and separately authorized exact tag cleanup.
use super::{api, guards, lookup, model};
use crate::cli::ReleaseDeleteArgs;
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::ProviderContext;
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

const OP: &str = "release.delete";
#[derive(Serialize)]
struct Receipt<'a> {
    provider: &'static str,
    repository: &'a str,
    release_id: u64,
    tag: &'a str,
    release_deleted: bool,
    tag_deleted: bool,
}
pub(super) fn failure(
    kind: &'static str,
    message: &str,
    release_deleted: bool,
    tag_deleted: bool,
    cause: Option<&ForgeError>,
) -> ForgeError {
    ForgeError::unavailable_with_details(
        schema_version_for("forge-cli", OP, 1),
        kind,
        message,
        serde_json::json!({"release_deleted":release_deleted, "tag_deleted":tag_deleted,
            "next_action":"inspect_release_and_tag_before_retry", "retryable":false,
            "cause":cause.map(ForgeError::kind)}),
    )
}
pub(super) fn run(
    ctx: &ProviderContext,
    repo: &str,
    args: &ReleaseDeleteArgs,
    release: &model::Release,
    original_tag: Option<model::Object>,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let path = format!("repos/{repo}/releases/{}", release.id);
    let result = api::request(OP, &api::call(ctx, &path, "DELETE")).map_err(|e| {
        failure(
            "release_delete_failed",
            "release deletion failed or its outcome is unknown; tag cleanup was not attempted",
            false,
            false,
            Some(&e),
        )
    })?;
    if result.is_none() {
        return Err(failure(
            "release_delete_failed",
            "release disappeared before DELETE; verify the selected release and tag",
            false,
            false,
            None,
        ));
    }
    // Verify both the selected database identity and the exact tag lookup. A
    // concurrent replacement release under the same tag is not complete absence.
    for lookup in [&path, &super::release_path(repo, &args.tag)] {
        let remaining = if lookup == &path { api::get(ctx, OP, lookup) } else { lookup::find(ctx, repo, &args.tag, OP) }.map_err(|e| failure("release_readback_failed", "DELETE was accepted but release absence could not be verified; tag cleanup was not attempted", false, false, Some(&e)))?;
        if remaining.is_some() {
            return Err(failure(
                "release_readback_mismatch",
                "a release still exists at the selected identity or tag; tag cleanup was not attempted",
                false,
                false,
                None,
            ));
        }
    }
    if args.cleanup_tag {
        let current = model::tag_object(ctx, repo, &args.tag, OP).map_err(|e| {
            failure(
                "release_cleanup_failed",
                "release absence verified; tag preflight failed, so no tag DELETE was attempted",
                true,
                false,
                Some(&e),
            )
        })?;
        if current != original_tag {
            return Err(failure(
                "release_cleanup_failed",
                "release absence verified; tag changed since preflight and was preserved",
                true,
                false,
                None,
            ));
        }
        if current.is_some() {
            let tag_path = format!("repos/{repo}/git/refs/tags/{}", guards::encode(&args.tag));
            let deleted = api::request(OP, &api::call(ctx, &tag_path, "DELETE")).map_err(|e| {
                failure(
                    "release_cleanup_failed",
                    "release absence verified; tag deletion failed or its outcome is unknown",
                    true,
                    false,
                    Some(&e),
                )
            })?;
            if deleted.is_none() {
                return Err(failure(
                    "release_cleanup_failed",
                    "release absence verified; tag disappeared during cleanup, inspect before retrying",
                    true,
                    false,
                    None,
                ));
            }
        }
        let remaining = model::tag_object(ctx, repo, &args.tag, OP).map_err(|e| {
            failure(
                "release_readback_failed",
                "release absence verified; tag absence could not be verified after cleanup",
                true,
                false,
                Some(&e),
            )
        })?;
        if remaining.is_some() {
            return Err(failure(
                "release_readback_mismatch",
                "release absence verified; the exact tag remains after cleanup",
                true,
                false,
                None,
            ));
        }
    }
    Ok(emit_success(
        schema_version_for("forge-cli", OP, 1),
        Receipt {
            provider: "github",
            repository: repo,
            release_id: release.id,
            tag: &args.tag,
            release_deleted: true,
            tag_deleted: args.cleanup_tag,
        },
        format,
        |p| {
            println!(
                "release #{} ({}) absence verified; tag absence verified: {}",
                p.release_id, p.tag, p.tag_deleted
            )
        },
    ))
}
