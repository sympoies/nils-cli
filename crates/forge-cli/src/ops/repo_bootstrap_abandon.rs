//! Abandon incomplete bootstrap state while holding its operation lock.
use super::repo_bootstrap::{self, acquire_bootstrap_lock, bootstrap_state_dir, load_receipt};
use crate::cli::{BINARY, GlobalFlags, ProviderFlag, RepoBootstrapArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use std::fs;

pub(crate) fn dispatch(
    global: &GlobalFlags,
    args: RepoBootstrapArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    if !args.abandon {
        return repo_bootstrap::run(global, args, format);
    }
    let fail = |message: &str| {
        ForgeError::validation(
            schema_version_for(BINARY, "error", 1),
            "bootstrap_abandon_failed",
            message,
            None,
        )
    };
    if global.dry_run {
        return Err(fail("--abandon cannot be combined with --dry-run"));
    }
    let (provider, host) = match &global.provider {
        Some(ProviderFlag::Github) | Some(ProviderFlag::Gitlab) => {
            let context = crate::provider::detect(
                global.provider_hint(),
                &global.remote,
                global.repo.as_deref(),
                |_| None,
            )?;
            (
                if matches!(global.provider, Some(ProviderFlag::Github)) {
                    "github"
                } else {
                    "gitlab"
                },
                Some(context.host),
            )
        }
        Some(ProviderFlag::Named(name)) => (name.as_str(), None),
        _ => return Err(fail("bootstrap abandonment requires a supported provider")),
    };
    let (owner, repo) = if provider == "gitlab" {
        repo_bootstrap::gitlab::repo_parts(global)?
    } else {
        crate::forgejo::repo_parts(global)?
    };
    let dir = bootstrap_state_dir(provider, host.as_deref(), &owner, &repo)?;
    let _lock = acquire_bootstrap_lock(&dir)?;
    let receipt_path = dir.join("receipt.json");
    let receipt =
        load_receipt(&receipt_path)?.ok_or_else(|| fail("no bootstrap receipt exists"))?;
    if receipt.complete {
        return Err(fail("a completed bootstrap receipt cannot be abandoned"));
    }
    let archive = dir.join("abandoned");
    fs::create_dir(&archive)
        .map_err(|_| fail("cannot create archive; an earlier archive may exist"))?;
    let checkout = dir.join("checkout");
    if checkout.exists() {
        fs::rename(&checkout, archive.join("checkout"))
            .map_err(|_| fail("cannot archive checkout"))?;
    }
    fs::rename(&receipt_path, archive.join("receipt.json"))
        .map_err(|_| fail("cannot archive receipt"))?;
    let payload = serde_json::json!({"abandoned": true, "archive": archive, "remote_deleted": false, "message": "Inspect the remote; remove or reuse it if empty."});
    Ok(emit_success(
        schema_version_for(BINARY, "repo.bootstrap.abandon", 1),
        payload,
        format,
        |_| println!("Receipt abandoned. Remove or reuse the empty remote repository yourself."),
    ))
}
