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
/// How long an open PR may read as out of the queue, or a merged PR as having
/// no merge commit, before that reading is final. GitHub removes the queue
/// entry of a PR it merged before the PR's state flips to `MERGED`.
const QUEUE_EXIT_GRACE: Duration = Duration::from_secs(60);
const QUEUE_EXIT_POLL_INTERVAL: Duration = Duration::from_secs(5);

const POLICY_QUERY: &str = "query ForgeMergePolicy($owner:String!,$name:String!,$base:String!,$pr:Int!){repository(owner:$owner,name:$name){mergeQueue(branch:$base){configuration{mergeMethod}} issues(first:20,states:OPEN,labels:[\"merge-freeze\"]){nodes{number title url author{login} createdAt}} pullRequest(number:$pr){id isInMergeQueue state}}}";

/// Freeze-only read for a GitHub host whose schema has no merge queue (older
/// GitHub Enterprise Server). Such a host cannot require a queue.
const FREEZE_QUERY: &str = "query ForgeMergeFreezes($owner:String!,$name:String!){repository(owner:$owner,name:$name){issues(first:20,states:OPEN,labels:[\"merge-freeze\"]){nodes{number title url author{login} createdAt}}}}";

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
    /// The provider already reports the PR as merged (a retried queue merge).
    pub merged: bool,
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
    if lacks_merge_queue_schema(&value) {
        return read_freezes_only(runner, ctx, owner, name);
    }
    reject_errors(&value, "merge policy", policy_unavailable)?;
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
        merged: repository["pullRequest"]["state"]
            .as_str()
            .is_some_and(|state| state.eq_ignore_ascii_case("MERGED")),
    })
}

fn read_freezes_only<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    owner: &str,
    name: &str,
) -> Result<MergePolicy, ForgeError> {
    let call = graphql_call(ctx, FREEZE_QUERY, &[("owner", owner), ("name", name)], &[]);
    let output = runner.run(&call)?;
    let value = parse_json(&output.stdout, "merge freeze")?;
    reject_errors(&value, "merge freeze", policy_unavailable)?;
    let freezes = value["data"]["repository"]["issues"]["nodes"]
        .as_array()
        .ok_or_else(|| policy_unavailable("merge freeze response has no freeze list", None))?
        .iter()
        .filter_map(parse_freeze)
        .collect();
    Ok(MergePolicy {
        freezes,
        queue: None,
        pr_node_id: None,
        in_queue: false,
        merged: false,
    })
}

/// True when every GraphQL error only says the merge-queue fields are absent
/// from the host schema.
fn lacks_merge_queue_schema(value: &serde_json::Value) -> bool {
    value["errors"].as_array().is_some_and(|errors| {
        !errors.is_empty()
            && errors.iter().all(|error| {
                error["message"].as_str().is_some_and(|message| {
                    (message.contains("mergeQueue") || message.contains("isInMergeQueue"))
                        && message.contains("doesn't exist")
                })
            })
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
    reject_errors(&value, "merge queue enqueue", enqueue_rejected)?;
    if value["data"]["enqueuePullRequest"]["mergeQueueEntry"].is_null() {
        return Err(enqueue_rejected(
            "the merge queue did not accept the pull request",
            None,
        ));
    }
    Ok(())
}

/// Poll until the queue merges the PR. Returns the merge commit when the
/// provider reports one.
///
/// A PR that reads `OPEN` with no queue entry, or `MERGED` with no merge
/// commit, may be mid-transition: GitHub drops the entry of a PR it merged
/// before the PR's state flips. Such a reading is re-polled for up to
/// [`QUEUE_EXIT_GRACE`], never past `timeout`, before it is final.
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
    let mut transitional_since = None;
    loop {
        let call = graphql_call(
            ctx,
            POLL_QUERY,
            &[("owner", owner), ("name", name)],
            &[("pr", pr)],
        );
        let output = runner.run(&call)?;
        let value = parse_json(&output.stdout, "merge queue poll")?;
        reject_errors(&value, "merge queue poll", poll_failed)?;
        let pull = &value["data"]["repository"]["pullRequest"];
        let state = pull["state"].as_str().unwrap_or_default();
        let entry = &pull["mergeQueueEntry"];
        let merged = state.eq_ignore_ascii_case("MERGED");
        let merge_commit = pull["mergeCommit"]["oid"]
            .as_str()
            .filter(|oid| !oid.is_empty());
        if merged && let Some(oid) = merge_commit {
            return Ok(Some(oid.to_string()));
        }
        if state.eq_ignore_ascii_case("CLOSED") {
            return Err(dequeued(pr, state));
        }
        if merged || entry.is_null() {
            let now = clock.now();
            let since = *transitional_since.get_or_insert(now);
            if now.duration_since(since) >= QUEUE_EXIT_GRACE
                || now.duration_since(started) >= timeout
            {
                // A merge without a reported commit still landed; the caller
                // reads the commit from the pull request view instead.
                return if merged {
                    Ok(None)
                } else {
                    Err(dequeued(pr, state))
                };
            }
            clock.sleep(QUEUE_EXIT_POLL_INTERVAL);
            continue;
        }
        transitional_since = None;
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
                "the pull request is still queued and will merge unless it is dequeued; the wait timed out before it landed",
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

fn dequeued(pr: u64, state: &str) -> ForgeError {
    ForgeError::runtime_failure(
        schema_err(),
        "merge_queue_dequeued",
        "the pull request left the merge queue without being merged",
        Some(format!("pr={pr}; state={state}")),
    )
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

fn reject_errors(
    value: &serde_json::Value,
    label: &str,
    error: fn(String, Option<String>) -> ForgeError,
) -> Result<(), ForgeError> {
    match value["errors"].as_array() {
        Some(errors) if !errors.is_empty() => Err(error(
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

fn enqueue_rejected(message: impl Into<String>, detail: Option<String>) -> ForgeError {
    ForgeError::runtime_failure(
        schema_err(),
        "merge_queue_enqueue_rejected",
        message,
        detail,
    )
}

fn poll_failed(message: impl Into<String>, detail: Option<String>) -> ForgeError {
    ForgeError::unavailable(schema_err(), "merge_queue_poll_failed", message, detail)
}

fn schema_err() -> String {
    schema_version_for(BINARY, "error", 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::time::Instant;

    use crate::backend::BackendSuccess;
    use crate::provider::{DetectionSource, Provider};

    struct ScriptedRunner {
        responses: RefCell<Vec<String>>,
    }

    impl ScriptedRunner {
        fn new(responses: &[&str]) -> Self {
            Self {
                responses: RefCell::new(responses.iter().rev().map(|r| r.to_string()).collect()),
            }
        }
    }

    impl BackendRunner for ScriptedRunner {
        fn run(&self, _call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
            let mut responses = self.responses.borrow_mut();
            let stdout = if responses.len() > 1 {
                responses.pop().unwrap()
            } else {
                responses.last().cloned().expect("scripted response")
            };
            Ok(BackendSuccess {
                stdout,
                stderr: String::new(),
            })
        }
    }

    /// Advances by the requested sleep instead of sleeping.
    struct StepClock {
        start: Instant,
        offset: Cell<Duration>,
    }

    impl StepClock {
        fn new() -> Self {
            Self {
                start: Instant::now(),
                offset: Cell::new(Duration::ZERO),
            }
        }
    }

    impl Clock for StepClock {
        fn now(&self) -> Instant {
            self.start + self.offset.get()
        }
        fn sleep(&self, dur: Duration) {
            self.offset.set(self.offset.get() + dur);
        }
    }

    fn github() -> ProviderContext {
        ProviderContext {
            provider: Provider::GitHub,
            host: "github.com".into(),
            source: DetectionSource::Flag,
            repo: Some("acme/widgets".into()),
        }
    }

    fn poll(state: &str, entry: &str) -> String {
        format!(
            r#"{{"data":{{"repository":{{"pullRequest":{{"state":"{state}","mergeCommit":null,"mergeQueueEntry":{entry}}}}}}}}}"#
        )
    }

    fn wait(responses: &[&str], timeout: Duration) -> Result<Option<String>, ForgeError> {
        wait_for_merge(
            &ScriptedRunner::new(responses),
            &StepClock::new(),
            &github(),
            "acme",
            "widgets",
            7,
            timeout,
        )
    }

    #[test]
    fn an_unmergeable_queue_entry_reports_failed_queue_checks() {
        let queued = poll("OPEN", r#"{"state":"QUEUED","position":1}"#);
        let unmergeable = poll("OPEN", r#"{"state":"UNMERGEABLE","position":1}"#);
        let err = wait(&[&queued, &unmergeable], Duration::from_secs(600)).unwrap_err();
        assert_eq!(err.kind(), "merge_queue_checks_failed");
    }

    #[test]
    fn a_queue_entry_stuck_past_the_bound_times_out() {
        let queued = poll("OPEN", r#"{"state":"QUEUED","position":3}"#);
        let err = wait(&[&queued], Duration::from_secs(60)).unwrap_err();
        assert_eq!(err.kind(), "merge_queue_timeout");
        assert!(err.message().contains("still queued"), "{}", err.message());
    }

    #[test]
    fn a_closed_pull_request_reports_dequeued() {
        let closed = poll("CLOSED", "null");
        let err = wait(&[&closed], Duration::from_secs(600)).unwrap_err();
        assert_eq!(err.kind(), "merge_queue_dequeued");
    }

    #[test]
    fn a_queue_merge_seen_before_the_state_flips_is_still_a_merge() {
        let queued = poll("OPEN", r#"{"state":"MERGEABLE","position":1}"#);
        let left = poll("OPEN", "null");
        let merged = r#"{"data":{"repository":{"pullRequest":{"state":"MERGED","mergeCommit":{"oid":"abc"},"mergeQueueEntry":null}}}}"#;
        assert_eq!(
            wait(&[&queued, &left, &left, merged], Duration::from_secs(600))
                .unwrap()
                .as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn an_open_pull_request_out_of_the_queue_past_the_grace_reports_dequeued() {
        let queued = poll("OPEN", r#"{"state":"QUEUED","position":1}"#);
        let left = poll("OPEN", "null");
        let runner = ScriptedRunner::new(&[&queued, &left]);
        let clock = StepClock::new();
        let err = wait_for_merge(
            &runner,
            &clock,
            &github(),
            "acme",
            "widgets",
            7,
            Duration::from_secs(600),
        )
        .unwrap_err();
        assert_eq!(err.kind(), "merge_queue_dequeued");
        assert!(
            err.detail()
                .is_some_and(|detail| detail.contains("state=OPEN")),
            "{:?}",
            err.detail()
        );
        let waited = clock.offset.get();
        assert!(
            waited >= QUEUE_POLL_INTERVAL + QUEUE_EXIT_GRACE,
            "dequeued only after the grace: waited {waited:?}"
        );
        assert!(
            waited < QUEUE_POLL_INTERVAL + QUEUE_EXIT_GRACE + QUEUE_POLL_INTERVAL,
            "the grace stays bounded: waited {waited:?}"
        );
    }

    #[test]
    fn a_pull_request_closed_after_leaving_the_queue_reports_dequeued() {
        let queued = poll("OPEN", r#"{"state":"QUEUED","position":1}"#);
        let left = poll("OPEN", "null");
        let closed = poll("CLOSED", "null");
        let err = wait(&[&queued, &left, &closed], Duration::from_secs(600)).unwrap_err();
        assert_eq!(err.kind(), "merge_queue_dequeued");
        assert!(
            err.detail()
                .is_some_and(|detail| detail.contains("state=CLOSED")),
            "{:?}",
            err.detail()
        );
    }

    #[test]
    fn the_grace_never_outlasts_the_queue_timeout() {
        let left = poll("OPEN", "null");
        let err = wait(&[&left], Duration::ZERO).unwrap_err();
        assert_eq!(err.kind(), "merge_queue_dequeued");
    }

    #[test]
    fn a_merge_reported_before_its_commit_waits_briefly_for_the_commit() {
        let merged_pending = poll("MERGED", "null");
        let merged = r#"{"data":{"repository":{"pullRequest":{"state":"MERGED","mergeCommit":{"oid":"abc"},"mergeQueueEntry":null}}}}"#;
        assert_eq!(
            wait(&[&merged_pending, merged], Duration::from_secs(600))
                .unwrap()
                .as_deref(),
            Some("abc")
        );
        assert_eq!(
            wait(&[&merged_pending], Duration::from_secs(600)).unwrap(),
            None,
            "a merge whose commit never appears still succeeds and falls back to the view"
        );
    }

    #[test]
    fn a_poll_graphql_error_has_its_own_kind() {
        let err = wait(
            &[r#"{"data":null,"errors":[{"message":"timeout"}]}"#],
            Duration::from_secs(600),
        )
        .unwrap_err();
        assert_eq!(err.kind(), "merge_queue_poll_failed");
    }

    #[test]
    fn a_merged_pull_request_returns_its_merge_commit() {
        let merged = r#"{"data":{"repository":{"pullRequest":{"state":"MERGED","mergeCommit":{"oid":"abc"},"mergeQueueEntry":null}}}}"#;
        assert_eq!(
            wait(&[merged], Duration::from_secs(60)).unwrap().as_deref(),
            Some("abc")
        );
    }

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
