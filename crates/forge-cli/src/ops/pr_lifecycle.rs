//! Additive lifecycle metadata for the standalone JSON PR read surface.
use serde::Serialize;
use serde_json::Value;
use std::ffi::OsString;

use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::error::ForgeError;
use crate::ops::{
    merge_queue::{self, QueueRead, string, unavailable},
    pr_comments::github_repo_slug_from_url,
    pr_view::PrViewPayload,
};
use crate::provider::ProviderContext;

const READY_FIELDS: &str = "headRefOid ready:timelineItems(itemTypes:[READY_FOR_REVIEW_EVENT],last:1){nodes{... on ReadyForReviewEvent{createdAt}}}";

#[derive(Debug, Default, Clone, Serialize, PartialEq, Eq)]
pub struct PrLifecycle {
    pub created_at: Option<String>,
    pub ready_for_review_at: Option<String>,
    pub merge_queue: Option<QueueRead>,
    pub merge_group: Option<MergeGroup>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MergeGroup {
    pub head_sha: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub checks: Vec<GroupCheck>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GroupCheck {
    pub name: Option<String>,
    pub status: Option<String>,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

pub(crate) fn enrich<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    view: &mut PrViewPayload,
) -> Result<(), ForgeError> {
    let slug = github_repo_slug_from_url(&view.url)
        .ok_or_else(|| unavailable("cannot resolve PR repository for lifecycle read"))?;
    let (owner, name) = slug
        .split_once('/')
        .ok_or_else(|| unavailable("invalid PR repository"))?;
    let pull = merge_queue::read_pull(runner, ctx, owner, name, view.number, READY_FIELDS)?;
    view.lifecycle.ready_for_review_at = pull["ready"]["nodes"]
        .as_array()
        .and_then(|nodes| nodes.last())
        .and_then(|node| string(&node["createdAt"]));
    view.lifecycle.merge_queue = merge_queue::from_pull(&pull);
    if view.lifecycle.merge_queue.is_some()
        && let Some(sha) = string(&pull["mergeQueueEntry"]["headCommit"]["oid"])
        // Never relabel ordinary PR-head checks as merge-group checks.
        && Some(sha.as_str()) != pull["headRefOid"].as_str()
    {
        view.lifecycle.merge_group = Some(read_group(runner, ctx, &slug, &sha)?);
    }
    Ok(())
}

fn read_group<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    slug: &str,
    sha: &str,
) -> Result<MergeGroup, ForgeError> {
    let mut argv = vec![
        OsString::from("api"),
        OsString::from(format!(
            "repos/{slug}/commits/{sha}/check-runs?per_page=100"
        )),
        OsString::from("--paginate"),
        OsString::from("--slurp"),
    ];
    ctx.push_github_api_hostname(&mut argv);
    let output = runner.run(&BackendCall::new(BackendProgram::Gh, argv))?;
    let pages: Value = serde_json::from_str(&output.stdout)
        .map_err(|_| unavailable("invalid merge-group check JSON"))?;
    let mut checks = Vec::new();
    for page in pages
        .as_array()
        .ok_or_else(|| unavailable("missing merge-group check pages"))?
    {
        for check in page["check_runs"]
            .as_array()
            .ok_or_else(|| unavailable("missing merge-group check runs"))?
        {
            checks.push(GroupCheck {
                name: string(&check["name"]),
                status: string(&check["status"]),
                conclusion: string(&check["conclusion"]),
                started_at: string(&check["started_at"]),
                completed_at: string(&check["completed_at"]),
            });
        }
    }
    let started_at = checks
        .iter()
        .filter_map(|check| check.started_at.as_ref())
        .min()
        .cloned();
    // A completed sibling does not mean the entire merge group completed.
    let completed_at = if !checks.is_empty()
        && checks.iter().all(|check| {
            check.status.as_deref() == Some("completed") && check.completed_at.is_some()
        }) {
        checks
            .iter()
            .filter_map(|check| check.completed_at.as_ref())
            .max()
            .cloned()
    } else {
        None
    };
    Ok(MergeGroup {
        head_sha: sha.to_string(),
        started_at,
        completed_at,
        checks,
    })
}
