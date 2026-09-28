//! GitHub merge policy for `pr merge`: repository-wide merge freezes and the
//! base branch's merge queue.
//!
//! A freeze is an open issue labelled [`FREEZE_LABEL`] in the target
//! repository. It lives on the provider, so every host and human sees the same
//! state, and it works on private repositories whose plan offers no branch
//! protection, rulesets, or merge queue. `pr merge` fails closed with
//! `merge_freeze_active` while one is open.
//!
//! When the base branch requires a merge queue, the direct merge API is
//! rejected by the provider, so `pr merge` enqueues the verified head instead
//! and waits, bounded, for the queue to merge it.

use std::ffi::OsString;
use std::time::Duration;

use nils_common::cli_contract::schema_version_for;
use serde::Serialize;

use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::cli::BINARY;
use crate::config::MergeMethod;
use crate::error::ForgeError;
use crate::ops::pr_wait_checks::Clock;
use crate::provider::ProviderContext;

/// Label whose open issues are active merge freezes.
pub const FREEZE_LABEL: &str = "merge-freeze";

/// Default bound on waiting for a merge queue to merge an enqueued PR.
pub const DEFAULT_QUEUE_TIMEOUT: Duration = Duration::from_secs(45 * 60);
const QUEUE_POLL_INTERVAL: Duration = Duration::from_secs(15);

const POLICY_QUERY: &str = "query ForgeMergePolicy($owner:String!,$name:String!,$base:String!,$pr:Int!){repository(owner:$owner,name:$name){mergeQueue(branch:$base){configuration{mergeMethod}} issues(first:20,states:OPEN,labels:[\"merge-freeze\"]){nodes{number title url author{login} createdAt}} pullRequest(number:$pr){id isInMergeQueue}}}";

const ENQUEUE_MUTATION: &str = "mutation ForgeEnqueuePullRequest($pullRequestId:ID!,$expectedHeadOid:GitObjectID!){enqueuePullRequest(input:{pullRequestId:$pullRequestId,expectedHeadOid:$expectedHeadOid}){mergeQueueEntry{state position}}}";

const POLL_QUERY: &str = "query ForgeMergeQueuePoll($owner:String!,$name:String!,$pr:Int!){repository(owner:$owner,name:$name){pullRequest(number:$pr){state mergeCommit{oid} mergeQueueEntry{state position}}}}";

/// One active merge freeze.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Freeze {
    pub number: u64,
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

impl Freeze {
    /// One-line description used in error details.
    pub fn describe(&self) -> String {
        format!(
            "#{} {} (by {}, since {}) {}",
            self.number,
            self.title,
            self.author.as_deref().unwrap_or("unknown"),
            self.created_at.as_deref().unwrap_or("unknown"),
            self.url
        )
    }
}

/// Recorded bypass of active freezes by their holder.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FreezeOverride {
    pub issues: Vec<u64>,
    pub reason: String,
}

/// The merge queue required by the base branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeQueue {
    /// The queue's configured merge method; `None` when the provider did not
    /// report a recognised one.
    pub method: Option<MergeMethod>,
}

/// Provider-side merge policy for one PR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePolicy {
    pub freezes: Vec<Freeze>,
    pub queue: Option<MergeQueue>,
    pub pr_node_id: Option<String>,
    pub in_queue: bool,
}

pub fn read_policy<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    owner: &str,
    name: &str,
    base: &str,
    pr: u64,
) -> Result<MergePolicy, ForgeError> {
    let call = graphql_call(
        ctx,
        POLICY_QUERY,
        &[("owner", owner), ("name", name), ("base", base)],
        &[("pr", pr)],
    );
    let output = runner.run(&call)?;
    let value = parse_json(&output.stdout, "merge policy")?;
    reject_errors(&value, "merge policy")?;
    let repository = &value["data"]["repository"];
    if repository.is_null() {
        return Err(policy_unavailable(
            "merge policy response has no repository",
            None,
        ));
    }
    let freezes = repository["issues"]["nodes"]
        .as_array()
        .ok_or_else(|| policy_unavailable("merge policy response has no freeze list", None))?
        .iter()
        .filter_map(parse_freeze)
        .collect();
    let queue = (!repository["mergeQueue"].is_null()).then(|| MergeQueue {
        method: repository["mergeQueue"]["configuration"]["mergeMethod"]
            .as_str()
            .and_then(parse_method),
    });
    Ok(MergePolicy {
        freezes,
        queue,
        pr_node_id: repository["pullRequest"]["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        in_queue: repository["pullRequest"]["isInMergeQueue"]
            .as_bool()
            .unwrap_or(false),
    })
}

/// Active freezes are refused unless the caller names every one of them.
pub fn enforce_freeze(
    freezes: &[Freeze],
    allowed: &[u64],
    reason: Option<&str>,
) -> Result<Option<FreezeOverride>, ForgeError> {
    if freezes.is_empty() {
        return Ok(None);
    }
    let uncovered: Vec<&Freeze> = freezes
        .iter()
        .filter(|freeze| !allowed.contains(&freeze.number))
        .collect();
    if uncovered.is_empty()
        && let Some(reason) = reason
    {
        return Ok(Some(FreezeOverride {
            issues: freezes.iter().map(|freeze| freeze.number).collect(),
            reason: reason.to_string(),
        }));
    }
    let blocking = if uncovered.is_empty() {
        freezes.iter().collect()
    } else {
        uncovered
    };
    Err(ForgeError::validation(
        schema_err(),
        "merge_freeze_active",
        "the repository has an active merge freeze; wait for it to end or, as its holder, pass --allow-merge-freeze <issue> with a reason",
        Some(
            blocking
                .iter()
                .map(|freeze| freeze.describe())
                .collect::<Vec<_>>()
                .join("; "),
        ),
    ))
}

/// The explicit `--method` must match the queue's; the queue decides how the
/// PR lands, so a different explicit method would be silently ignored.
pub fn resolve_queue_method(
    queue: &MergeQueue,
    explicit: Option<MergeMethod>,
    fallback: MergeMethod,
) -> Result<MergeMethod, ForgeError> {
    match (queue.method, explicit) {
        (Some(queue_method), Some(wanted)) if queue_method != wanted => {
            Err(ForgeError::validation(
                schema_err(),
                "merge_queue_method_mismatch",
                format!(
                    "the base branch merge queue merges with {queue:?}, not the requested {wanted:?}",
                    queue = queue_method.as_str(),
                    wanted = wanted.as_str(),
                ),
                None,
            ))
        }
        (Some(queue_method), _) => Ok(queue_method),
        (None, explicit) => Ok(explicit.unwrap_or(fallback)),
    }
}

pub fn enqueue<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    pr_node_id: &str,
    expected_head: &str,
) -> Result<(), ForgeError> {
    let call = graphql_call(
        ctx,
        ENQUEUE_MUTATION,
        &[
            ("pullRequestId", pr_node_id),
            ("expectedHeadOid", expected_head),
        ],
        &[],
    );
    let output = runner.run(&call)?;
    let value = parse_json(&output.stdout, "merge queue enqueue")?;
    reject_errors(&value, "merge queue enqueue")?;
    if value["data"]["enqueuePullRequest"]["mergeQueueEntry"].is_null() {
        return Err(ForgeError::runtime_failure(
            schema_err(),
            "merge_queue_dequeued",
            "the merge queue did not accept the pull request",
            None,
        ));
    }
    Ok(())
}

/// Poll until the queue merges the PR. Returns the merge commit when the
/// provider reports one.
pub fn wait_for_merge<R: BackendRunner, C: Clock>(
    runner: &R,
    clock: &C,
    ctx: &ProviderContext,
    owner: &str,
    name: &str,
    pr: u64,
    timeout: Duration,
) -> Result<Option<String>, ForgeError> {
    let started = clock.now();
    loop {
        let call = graphql_call(
            ctx,
            POLL_QUERY,
            &[("owner", owner), ("name", name)],
            &[("pr", pr)],
        );
        let output = runner.run(&call)?;
        let value = parse_json(&output.stdout, "merge queue poll")?;
        reject_errors(&value, "merge queue poll")?;
        let pull = &value["data"]["repository"]["pullRequest"];
        let state = pull["state"].as_str().unwrap_or_default();
        if state.eq_ignore_ascii_case("MERGED") {
            return Ok(pull["mergeCommit"]["oid"].as_str().map(str::to_string));
        }
        let entry = &pull["mergeQueueEntry"];
        if state.eq_ignore_ascii_case("CLOSED") || entry.is_null() {
            return Err(ForgeError::runtime_failure(
                schema_err(),
                "merge_queue_dequeued",
                "the pull request left the merge queue without being merged",
                Some(format!("pr={pr}; state={state}")),
            ));
        }
        if entry["state"]
            .as_str()
            .is_some_and(|state| state.eq_ignore_ascii_case("UNMERGEABLE"))
        {
            return Err(ForgeError::runtime_failure(
                schema_err(),
                "merge_queue_checks_failed",
                "the merge queue reports the pull request as unmergeable",
                Some(format!("pr={pr}")),
            ));
        }
        if clock.now().duration_since(started) >= timeout {
            return Err(ForgeError::unavailable(
                schema_err(),
                "merge_queue_timeout",
                "the merge queue did not merge the pull request before the timeout",
                Some(format!(
                    "pr={pr}; entry_state={}; timeout_secs={}",
                    entry["state"].as_str().unwrap_or("unknown"),
                    timeout.as_secs()
                )),
            ));
        }
        clock.sleep(QUEUE_POLL_INTERVAL);
    }
}

pub(crate) fn parse_freeze(node: &serde_json::Value) -> Option<Freeze> {
    Some(Freeze {
        number: node["number"].as_u64()?,
        title: node["title"].as_str().unwrap_or_default().to_string(),
        url: node["url"].as_str().unwrap_or_default().to_string(),
        author: node["author"]["login"].as_str().map(str::to_string),
        created_at: node["createdAt"].as_str().map(str::to_string),
    })
}

fn parse_method(value: &str) -> Option<MergeMethod> {
    match value.to_ascii_uppercase().as_str() {
        "SQUASH" => Some(MergeMethod::Squash),
        "MERGE" => Some(MergeMethod::Merge),
        "REBASE" => Some(MergeMethod::Rebase),
        _ => None,
    }
}

fn graphql_call(
    ctx: &ProviderContext,
    query: &str,
    strings: &[(&str, &str)],
    ints: &[(&str, u64)],
) -> BackendCall {
    let mut argv = vec![OsString::from("api"), OsString::from("graphql")];
    ctx.push_github_api_hostname(&mut argv);
    argv.push(OsString::from("-f"));
    argv.push(OsString::from(format!("query={query}")));
    for (key, value) in strings {
        argv.push(OsString::from("-f"));
        argv.push(OsString::from(format!("{key}={value}")));
    }
    for (key, value) in ints {
        argv.push(OsString::from("-F"));
        argv.push(OsString::from(format!("{key}={value}")));
    }
    BackendCall::new(BackendProgram::Gh, argv)
}

fn parse_json(stdout: &str, label: &str) -> Result<serde_json::Value, ForgeError> {
    serde_json::from_str(stdout.trim()).map_err(|error| {
        policy_unavailable(
            format!("{label} response is not valid JSON"),
            Some(error.to_string()),
        )
    })
}

fn reject_errors(value: &serde_json::Value, label: &str) -> Result<(), ForgeError> {
    match value["errors"].as_array() {
        Some(errors) if !errors.is_empty() => Err(policy_unavailable(
            format!("{label} returned GraphQL errors"),
            Some(
                errors
                    .iter()
                    .filter_map(|error| error["message"].as_str())
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        )),
        _ => Ok(()),
    }
}

fn policy_unavailable(message: impl Into<String>, detail: Option<String>) -> ForgeError {
    ForgeError::unavailable(schema_err(), "merge_policy_unavailable", message, detail)
}

fn schema_err() -> String {
    schema_version_for(BINARY, "error", 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn freeze(number: u64) -> Freeze {
        Freeze {
            number,
            title: format!("Merge freeze {number}"),
            url: format!("https://github.com/acme/widgets/issues/{number}"),
            author: Some("holder".into()),
            created_at: None,
        }
    }

    #[test]
    fn no_freeze_passes_without_an_override() {
        assert_eq!(enforce_freeze(&[], &[], None).unwrap(), None);
    }

    #[test]
    fn an_override_must_name_every_active_freeze_and_carry_a_reason() {
        let active = [freeze(42), freeze(43)];
        assert_eq!(
            enforce_freeze(&active, &[42], Some("holder"))
                .unwrap_err()
                .kind(),
            "merge_freeze_active"
        );
        assert_eq!(
            enforce_freeze(&active, &[42, 43], None).unwrap_err().kind(),
            "merge_freeze_active"
        );
        let granted = enforce_freeze(&active, &[43, 42], Some("holder")).unwrap();
        assert_eq!(
            granted,
            Some(FreezeOverride {
                issues: vec![42, 43],
                reason: "holder".into()
            })
        );
    }

    #[test]
    fn queue_method_wins_unless_an_explicit_method_contradicts_it() {
        let queue = MergeQueue {
            method: Some(MergeMethod::Squash),
        };
        assert_eq!(
            resolve_queue_method(&queue, None, MergeMethod::Merge).unwrap(),
            MergeMethod::Squash
        );
        assert_eq!(
            resolve_queue_method(&queue, Some(MergeMethod::Rebase), MergeMethod::Squash)
                .unwrap_err()
                .kind(),
            "merge_queue_method_mismatch"
        );
        let unknown = MergeQueue { method: None };
        assert_eq!(
            resolve_queue_method(&unknown, Some(MergeMethod::Rebase), MergeMethod::Squash).unwrap(),
            MergeMethod::Rebase
        );
    }
}
