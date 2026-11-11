//! Delete only preflighted assets of the release selected by tag.
use super::{api, guards, lookup};
use crate::cli::{GlobalFlags, ReleaseDeleteAssetArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::github_write::{invalid, target};
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const OP: &str = "release.delete-asset";
#[derive(Deserialize, Serialize)]
struct Asset {
    #[serde(skip_serializing)]
    id: u64,
    name: String,
    size: u64,
    digest: Option<String>,
}
#[derive(Deserialize)]
struct Release {
    id: u64,
    tag_name: String,
    assets: Vec<Asset>,
}
#[derive(Serialize)]
struct ResultPayload {
    provider: &'static str,
    repository: String,
    tag: String,
    dry_run: bool,
    assets: Vec<Asset>,
}
pub(super) fn run(
    global: &GlobalFlags,
    args: ReleaseDeleteAssetArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    guards::tag(&args.tag, OP)?;
    let mut names = HashSet::new();
    for name in &args.assets {
        if name.is_empty() || !names.insert(name.as_str()) {
            return Err(invalid(
                OP,
                "release_asset_invalid",
                "asset names must be nonempty and unique",
            ));
        }
    }
    let mut expectations = std::collections::HashMap::new();
    for guard in &args.expect_sha256 {
        let (name, digest) = guard
            .split_once('=')
            .ok_or_else(|| invalid(OP, "release_asset_guard_invalid", "expected NAME=SHA256"))?;
        if !names.contains(name)
            || digest.len() != 64
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            || expectations
                .insert(name, digest.to_ascii_lowercase())
                .is_some()
        {
            return Err(invalid(
                OP,
                "release_asset_guard_invalid",
                "each guard must name a requested asset and contain a unique SHA-256 digest",
            ));
        }
    }
    let ctx = target(global, OP)?;
    let repo = ctx.repo.as_deref().expect("resolved repository");
    let value = lookup::find(&ctx, repo, &args.tag, OP)?.ok_or_else(|| {
        invalid(
            OP,
            "release_not_found",
            "no release exists for the selected tag",
        )
    })?;
    let release: Release = serde_json::from_value(value).map_err(|_| {
        invalid(
            OP,
            "release_response_invalid",
            "provider release response is incomplete",
        )
    })?;
    if release.id == 0 || release.tag_name != args.tag {
        return Err(invalid(
            OP,
            "release_response_mismatch",
            "provider returned a different release",
        ));
    }
    let mut selected = Vec::new();
    for name in &args.assets {
        let mut found = release.assets.iter().filter(|a| &a.name == name);
        let asset = found.next().ok_or_else(|| {
            invalid(
                OP,
                "release_asset_not_found",
                "requested asset was not found",
            )
        })?;
        if asset.id == 0 || found.next().is_some() {
            return Err(invalid(
                OP,
                "release_response_invalid",
                "asset identity is ambiguous",
            ));
        }
        if let Some(expected) = expectations.get(name.as_str()) {
            let digest = asset
                .digest
                .as_deref()
                .and_then(|d| d.strip_prefix("sha256:"));
            if !digest.is_some_and(|d| d.eq_ignore_ascii_case(expected)) {
                return Err(invalid(
                    OP,
                    "release_asset_digest_mismatch",
                    "asset SHA-256 digest does not match the expected value",
                ));
            }
        }
        selected.push(Asset {
            id: asset.id,
            name: asset.name.clone(),
            size: asset.size,
            digest: asset.digest.clone(),
        });
    }
    if !global.dry_run {
        for (index, asset) in selected.iter().enumerate() {
            let call = api::call(
                &ctx,
                &format!("repos/{repo}/releases/assets/{}", asset.id),
                "DELETE",
            );
            let result = api::request(OP, &call);
            if !matches!(result, Ok(Some(_))) {
                return Err(ForgeError::unavailable_with_details(
                    schema_version_for("forge-cli", OP, 1),
                    "release_asset_delete_failed",
                    "asset deletion failed or its outcome is unknown; inspect the release before retrying",
                    serde_json::json!({
                        "deleted_assets": &selected[..index],
                        "failed_asset": asset,
                        "failed_asset_outcome": "unknown",
                        "next_action": "inspect_release_before_retry",
                        "retryable": false
                    }),
                ));
            }
        }
    }
    Ok(emit_success(
        schema_version_for("forge-cli", OP, 1),
        ResultPayload {
            provider: "github",
            repository: repo.into(),
            tag: args.tag,
            dry_run: global.dry_run,
            assets: selected,
        },
        format,
        |p| {
            for asset in &p.assets {
                println!(
                    "{} {} size={} digest={}",
                    if p.dry_run {
                        "would delete:"
                    } else {
                        "deleted:"
                    },
                    asset.name,
                    asset.size,
                    asset.digest.as_deref().unwrap_or("null")
                );
            }
        },
    ))
}
