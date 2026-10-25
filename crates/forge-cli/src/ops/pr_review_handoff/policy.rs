//! The repository handoff policy is read only from the cached provider base.
use std::process::Command;
use std::time::Duration;

use super::{diagnostics, fail};
use crate::config::ForgeConfig;
use crate::error::ForgeError;
use crate::provider::Provider;

fn git_success(toplevel: &std::path::Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(toplevel)
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(super) fn trusted_supervisor_policy(
    repository: &str,
    provider: Provider,
    provider_host: &str,
    provider_base_sha: &str,
    base_ref: &str,
    remote: &str,
) -> Result<(Vec<String>, Duration), ForgeError> {
    let workdir = std::env::current_dir().map_err(|_| diagnostics::checkout_unavailable())?;
    let toplevel = crate::ops::pr_create::find_git_toplevel(&workdir)
        .ok_or_else(diagnostics::checkout_unavailable)?;
    let remote_url = crate::provider::git_remote_url(remote)
        .ok_or_else(|| diagnostics::remote_unavailable(remote))?;
    let remote_repository = crate::provider::parse_slug(&remote_url)
        .ok_or_else(|| diagnostics::remote_unavailable(remote))?;
    let remote_host = crate::provider::parse_host(&remote_url)
        .ok_or_else(|| diagnostics::remote_unavailable(remote))?;
    let repository_matches = match provider {
        Provider::GitHub => remote_repository.eq_ignore_ascii_case(repository),
        _ => remote_repository == repository,
    };
    if !repository_matches
        || !crate::provider::authorities_equal(provider, provider_host, &remote_host)
    {
        return Err(diagnostics::remote_repository_mismatch(remote));
    }
    let git_ref = format!("refs/remotes/{remote}/{base_ref}");
    let resolved = Command::new("git")
        .args(["rev-parse", "--verify", &format!("{git_ref}^{{commit}}")])
        .current_dir(&toplevel)
        .output()
        .map_err(|_| diagnostics::checkout_unavailable())?;
    if !resolved.status.success() {
        return Err(diagnostics::base_ref_missing(remote, base_ref));
    }
    let cached = String::from_utf8_lossy(&resolved.stdout).trim().to_string();
    if cached != provider_base_sha {
        // The provider base can predate the cached branch tip (the pull request
        // has not been updated onto it); otherwise the cache is behind or moved.
        let provider_base_is_ancestor = git_success(
            &toplevel,
            &["merge-base", "--is-ancestor", provider_base_sha, &cached],
        );
        return Err(if provider_base_is_ancestor {
            diagnostics::provider_base_behind_cache(remote, base_ref, provider_base_sha)
        } else {
            diagnostics::base_ref_stale(remote, base_ref, provider_base_sha)
        });
    }
    let config = ForgeConfig::load_from_git_ref(&workdir, &toplevel, &git_ref);
    if config
        .warnings
        .iter()
        .any(|warning| warning.starts_with("invalid-config-value:review_handoff.stale_after"))
    {
        return Err(diagnostics::policy_config_invalid());
    }
    Ok((
        config.review_handoff_supervisors.unwrap_or_default(),
        config
            .review_handoff_stale_after
            .unwrap_or_else(|| Duration::from_secs(2 * 60 * 60)),
    ))
}

/// Only a supervisor configured at the provider base may take over. An empty
/// list is named separately so the operator knows to configure one.
pub(super) fn require_supervisor(supervisors: &[String], actor: &str) -> Result<(), ForgeError> {
    if supervisors.iter().any(|supervisor| supervisor == actor) {
        return Ok(());
    }
    Err(fail(
        "review_writer_conflict",
        if supervisors.is_empty() {
            "no repository-configured handoff supervisor exists at the provider base; add \
             [review_handoff].supervisors on the base branch before takeover"
        } else {
            "only a repository-configured handoff supervisor may take over"
        },
    ))
}
