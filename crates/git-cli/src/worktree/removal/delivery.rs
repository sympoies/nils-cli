//! Opt-in, bounded forge evidence for rewritten deliveries. Never fetch objects.
use super::{DeliveryProof, git, probe};
use serde_json::Value;
use std::path::Path;

fn query(target: &Path, args: &[&str]) -> Option<Value> {
    let output = probe("forge-cli", args, target).ok()?;
    if !output.status.success() {
        return None;
    }
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    (value["ok"] == true).then(|| value["data"].clone())
}

pub(super) fn lookup(target: &Path, head: &str, branch: Option<&str>) -> Option<DeliveryProof> {
    let branch = branch?;
    let reference = git(
        target,
        &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    )
    .ok()?;
    let default = reference.strip_prefix("refs/remotes/origin/")?;
    let default_head = git(
        target,
        &["rev-parse", "--verify", &format!("{reference}^{{commit}}")],
    )
    .ok()?;
    if !git(
        target,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .ok()?
    .is_empty()
    {
        return None;
    }
    let list = query(
        target,
        &[
            "pr", "list", "--state", "merged", "--head", branch, "--base", default, "--limit",
            "30", "--format", "json",
        ],
    )?;
    for item in list["items"].as_array()?.iter().take(30) {
        let Some(number) = item["number"].as_u64().filter(|n| *n > 0) else {
            continue;
        };
        let Some(pr) = query(
            target,
            &["pr", "view", &number.to_string(), "--format", "json"],
        ) else {
            continue;
        };
        if pr["number"].as_u64() != Some(number)
            || pr["state"] != "merged"
            || pr["head"] != branch
            || pr["base"] != default
            || !pr["merged_at"].as_str().is_some_and(|s| !s.is_empty())
        {
            continue;
        }
        let Some(sha) = pr["head_sha"]
            .as_str()
            .filter(|s| matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit()))
        else {
            continue;
        };
        // Resolve even an equal SHA: unavailable objects never constitute proof.
        if git(
            target,
            &["rev-parse", "--verify", &format!("{sha}^{{commit}}")],
        )
        .ok()
        .as_deref()
            != Some(sha)
        {
            continue;
        }
        if !probe(
            "git",
            &[
                "--no-replace-objects",
                "merge-base",
                "--is-ancestor",
                head,
                sha,
            ],
            target,
        )
        .ok()?
        .status
        .success()
        {
            continue;
        }
        if git(target, &["symbolic-ref", "--quiet", "--short", "HEAD"])
            .ok()
            .as_deref()
            != Some(branch)
            || git(target, &["rev-parse", "HEAD"]).ok().as_deref() != Some(head)
            || !git(
                target,
                &["status", "--porcelain=v1", "--untracked-files=all"],
            )
            .ok()?
            .is_empty()
        {
            return None;
        }
        return Some(DeliveryProof {
            basis: "merged-pull-request",
            default_branch: default.to_owned(),
            default_head,
            pull_request: Some(number),
            head_sha: Some(sha.to_owned()),
        });
    }
    None
}
