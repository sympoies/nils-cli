//! Shared, read-only queue snapshot for PR lifecycle reads and queue callers.
//! No enqueue/dequeue mutation is performed here.
use serde::Serialize;
use serde_json::Value;

use crate::backend::BackendRunner;
use crate::error::ForgeError;
use crate::ops::merge_policy;
use crate::provider::{Provider, ProviderContext};

/// A current provider queue entry. The enclosing option is null when the PR
/// is not queued or the host cannot expose a queue. State is provider-native.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QueueRead {
    pub queued: bool,
    pub position: Option<u64>,
    pub enqueued_at: Option<String>,
    pub state: Option<String>,
    /// The PR head observed on the queued entry's pull request, distinct from
    /// the synthetic merge-group commit. Mutation callers must revalidate it.
    pub head_sha: Option<String>,
}

const QUEUE_FIELDS: &str = "isInMergeQueue mergeQueueEntry{state position enqueuedAt headCommit{oid} pullRequest{headRefOid}}";

/// Share the existing merge-policy poll shape, adding provider entry metadata.
pub(crate) fn query(operation: &str, extra: &str, queue: bool) -> String {
    let fields = if queue { QUEUE_FIELDS } else { "" };
    format!(
        "query {operation}($owner:String!,$name:String!,$pr:Int!){{repository(owner:$owner,name:$name){{pullRequest(number:$pr){{state mergeCommit{{oid}} {fields} {extra}}}}}}}"
    )
}

/// Read the same queue object exposed by `pr view`. Intended for queue
/// consumers, including guarded dequeue preflight, without lifecycle reads.
pub fn read<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    owner: &str,
    name: &str,
    pr: u64,
) -> Result<Option<QueueRead>, ForgeError> {
    if ctx.provider != Provider::GitHub {
        return Ok(None);
    }
    Ok(from_pull(&read_pull(runner, ctx, owner, name, pr, "")?))
}

pub(crate) fn read_pull<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    owner: &str,
    name: &str,
    pr: u64,
    extra: &str,
) -> Result<Value, ForgeError> {
    let mut queue = true;
    loop {
        let query = query("ForgePrLifecycle", extra, queue);
        let call = merge_policy::graphql_call(
            ctx,
            &query,
            &[("owner", owner), ("name", name)],
            &[("pr", pr)],
        );
        let output = match runner.run(&call) {
            Ok(output) => output,
            Err(ForgeError::BackendError {
                detail: Some(ref detail),
                ..
            }) if queue && unsupported_backend(detail) => {
                queue = false;
                continue;
            }
            Err(error) => return Err(error),
        };
        let value: Value = serde_json::from_str(&output.stdout)
            .map_err(|_| unavailable("invalid queue-read JSON"))?;
        if let Some(errors) = value["errors"]
            .as_array()
            .filter(|errors| !errors.is_empty())
        {
            if queue
                && errors
                    .iter()
                    .all(|error| error["message"].as_str().is_some_and(unsupported_message))
            {
                queue = false;
                continue;
            }
            return Err(unavailable("provider rejected the queue read"));
        }
        let pull = &value["data"]["repository"]["pullRequest"];
        if !pull.is_object() {
            return Err(unavailable("queue-read response has no pull request"));
        }
        return Ok(pull.clone());
    }
}

fn unsupported_backend(detail: &str) -> bool {
    if let Ok(value) = serde_json::from_str::<Value>(detail) {
        return value["errors"].as_array().is_some_and(|errors| {
            !errors.is_empty()
                && errors
                    .iter()
                    .all(|error| error["message"].as_str().is_some_and(unsupported_message))
        });
    }
    let lines: Vec<_> = detail
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    !lines.is_empty() && lines.iter().all(|line| unsupported_message(line))
}

fn unsupported_message(message: &str) -> bool {
    (message.contains("mergeQueueEntry") || message.contains("isInMergeQueue"))
        && (message.contains("doesn't exist") || message.contains("Cannot query field"))
}

pub(crate) fn from_pull(pull: &Value) -> Option<QueueRead> {
    let entry = &pull["mergeQueueEntry"];
    if !entry.is_object() || pull["isInMergeQueue"].as_bool() == Some(false) {
        return None;
    }
    Some(QueueRead {
        queued: true,
        position: entry["position"].as_u64(),
        enqueued_at: string(&entry["enqueuedAt"]),
        state: string(&entry["state"]),
        head_sha: string(&entry["pullRequest"]["headRefOid"]),
    })
}

pub(crate) fn string(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn unavailable(message: &str) -> ForgeError {
    ForgeError::unavailable(
        "cli.forge-cli.error.v1",
        "pr_lifecycle_unavailable",
        message,
        None,
    )
}
