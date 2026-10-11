//! `pr review-threads reply` atom — post a reply onto a review thread without
//! resolving it.
//!
//! Spec / ops: `cli.forge-cli.pr.review-threads.reply.v1`. GitHub-first: the
//! thread node id (`PRRT_...` from the read surface) identifies the root comment.
//! REST's comment-reply endpoint appends a comment without creating a new
//! native review that could supersede an approval. This op never resolves.
//!
//! GitLab and Local have no GitHub-shaped thread-mutation surface, so they
//! return a structured `provider_unsupported` error (GitHub-first in v1).

use std::ffi::OsString;

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendCall, BackendProgram, BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, PrReviewThreadReplyArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::pr_comment::read_body;
use crate::ops::pr_review_threads;
use crate::provider::{Provider, ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;
use crate::validations::{no_agent_attribution, no_local_path};

const SCHEMA: &str = "pr.review-threads.reply";
const SCHEMA_VERSION: u32 = 1;

/// Read only the root comment and permissions for the selected thread.
const GITHUB_REPLY_TARGET_QUERY: &str = "query($tid: ID!) { node(id: $tid) { ... on PullRequestReviewThread { id isResolved viewerCanResolve comments(first: 1) { nodes { fullDatabaseId } } } } }";

/// Offline mutation plan with an explicit read dependency. The REST plan's
/// `${root_comment_id}` is bound from the target query, never sent literally.
#[derive(Serialize)]
pub(crate) struct ThreadMutationDryRunPayload {
    #[serde(flatten)]
    pub mutation: DryRunPayload,
    pub target_plan: Vec<String>,
    pub root_comment_id_source: &'static str,
}

pub(crate) fn dry_run_payload(
    ctx: &ProviderContext,
    thread: &str,
    mutation: DryRunPayload,
) -> ThreadMutationDryRunPayload {
    ThreadMutationDryRunPayload {
        mutation,
        target_plan: build_reply_target_call(ctx, thread).plan_argv(),
        root_comment_id_source: "/data/node/comments/nodes/0/fullDatabaseId",
    }
}

/// Envelope payload for `cli.forge-cli.pr.review-threads.reply.v1`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrReviewThreadReplyPayload {
    pub provider: &'static str,
    pub thread_id: String,
    pub comment_url: String,
}

pub fn run(
    global: &GlobalFlags,
    args: PrReviewThreadReplyArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let runner = default_runner();
    run_with(&runner, global, args, format, git_remote_url)
}

pub fn run_with<R: BackendRunner, F: Fn(&str) -> Option<String>>(
    runner: &R,
    global: &GlobalFlags,
    args: PrReviewThreadReplyArgs,
    format: OutputFormat,
    remote_url_lookup: F,
) -> Result<i32, ForgeError> {
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        remote_url_lookup,
    )?;

    ensure_github(&ctx)?;

    let body = read_body(args.body.as_deref(), args.body_file.as_deref())?;
    if body.trim().is_empty() {
        return Err(ForgeError::validation(
            schema_err(),
            "body_missing_summary",
            "reply body is empty (supply --body or --body-file)",
            None,
        ));
    }
    no_local_path(&body, "reply")?;
    no_agent_attribution(&body, "reply")?;

    let call = build_reply_call(&ctx, args.id, "${root_comment_id}", &body)?;

    if global.dry_run {
        let payload = dry_run_payload(&ctx, &args.thread, DryRunPayload::new(ctx.provider, &call));
        return Ok(emit_success(
            schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION),
            payload,
            format,
            |p| {
                println!("would read reply target: {}", p.target_plan.join(" "));
                println!(
                    "would run after binding root_comment_id: {}",
                    p.mutation.plan.join(" ")
                );
            },
        ));
    }

    pr_review_threads::ensure_thread_belongs_to_pr(runner, &ctx, args.id, &args.thread)?;

    let target = read_reply_target(runner, &ctx, &args.thread)?;
    let call = build_reply_call(&ctx, args.id, &target.comment_id, &body)?;
    let output = runner.run(&call)?;
    let comment_url = parse_comment_url(&output.stdout);

    Ok(emit_success(
        schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION),
        PrReviewThreadReplyPayload {
            provider: ctx.provider.as_str(),
            thread_id: args.thread,
            comment_url,
        },
        format,
        render_text,
    ))
}

/// GitHub-first: GitLab and Local fail closed with a structured
/// `provider_unsupported` error before any backend call.
fn ensure_github(ctx: &ProviderContext) -> Result<(), ForgeError> {
    match ctx.provider {
        Provider::GitHub => Ok(()),
        Provider::GitLab | Provider::Local => Err(ForgeError::provider_unsupported(
            schema_err(),
            format!(
                "pr review-threads reply is GitHub-only in v1 (provider: {provider})",
                provider = ctx.provider.as_str(),
            ),
            None,
        )),
    }
}

pub(crate) struct ReplyTarget {
    pub comment_id: String,
    pub can_resolve: bool,
    pub resolved: bool,
}

/// Read the root comment for REST replies and resolution permission before any
/// mutation. Callers first prove thread membership in the selected PR.
pub(crate) fn read_reply_target<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    thread_id: &str,
) -> Result<ReplyTarget, ForgeError> {
    let output = runner.run(&build_reply_target_call(ctx, thread_id))?;
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).map_err(|_| reply_target_incomplete())?;
    let node = value
        .pointer("/data/node")
        .ok_or_else(reply_target_incomplete)?;
    if node.get("id").and_then(|v| v.as_str()) != Some(thread_id) {
        return Err(reply_target_incomplete());
    }
    let id = node
        .pointer("/comments/nodes/0/fullDatabaseId")
        .ok_or_else(reply_target_incomplete)?;
    let comment_id = id
        .as_u64()
        .or_else(|| id.as_str().and_then(|s| s.parse::<u64>().ok()))
        .filter(|id| *id > 0)
        .ok_or_else(reply_target_incomplete)?
        .to_string();
    Ok(ReplyTarget {
        comment_id,
        can_resolve: node
            .get("viewerCanResolve")
            .and_then(|v| v.as_bool())
            .ok_or_else(reply_target_incomplete)?,
        resolved: node
            .get("isResolved")
            .and_then(|v| v.as_bool())
            .ok_or_else(reply_target_incomplete)?,
    })
}

fn build_reply_target_call(ctx: &ProviderContext, thread_id: &str) -> BackendCall {
    let mut argv = vec![OsString::from("api"), OsString::from("graphql")];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        OsString::from("-f"),
        OsString::from(format!("query={GITHUB_REPLY_TARGET_QUERY}")),
        OsString::from("-f"),
        OsString::from(format!("tid={thread_id}")),
    ]);
    BackendCall::new(BackendProgram::Gh, argv)
}

fn reply_target_incomplete() -> ForgeError {
    ForgeError::validation(
        schema_err(),
        "review_snapshot_incomplete",
        "review thread reply target or viewer permissions are missing",
        None,
    )
}

pub(crate) fn build_reply_call(
    ctx: &ProviderContext,
    number: u64,
    comment_id: &str,
    body: &str,
) -> Result<BackendCall, ForgeError> {
    debug_assert!(matches!(ctx.provider, Provider::GitHub));
    let repository = ctx.repo.as_deref().ok_or_else(|| {
        ForgeError::validation(
            schema_err(),
            "repo_required",
            "GitHub review-thread replies require --repo owner/name or a recognised remote",
            None,
        )
    })?;
    let mut argv = vec![
        OsString::from("api"),
        OsString::from(format!(
            "repos/{repository}/pulls/{number}/comments/{comment_id}/replies"
        )),
    ];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        OsString::from("--method"),
        OsString::from("POST"),
        OsString::from("-f"),
        OsString::from(format!("body={body}")),
    ]);
    Ok(BackendCall::new(BackendProgram::Gh, argv))
}

/// Pull the new comment url out of the mutation response. Best-effort: an
/// absent url yields the empty string rather than an error, since the reply
/// itself succeeded.
fn parse_comment_url(stdout: &str) -> String {
    serde_json::from_str::<serde_json::Value>(stdout.trim())
        .ok()
        .as_ref()
        .and_then(|v| v.get("html_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn schema_err() -> String {
    schema_version_for(BINARY, "error", 1)
}

fn render_text(payload: &PrReviewThreadReplyPayload) {
    println!(
        "replied to {provider} review thread {thread}: {url}",
        provider = payload.provider,
        thread = payload.thread_id,
        url = payload.comment_url,
    );
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use nils_common::cli_contract::{OutputFormat, exit};
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::backend::{BackendOutput, BackendSuccess};
    use crate::cli::ProviderFlag;
    use crate::provider::DetectionSource;

    type RecordedCall = (BackendProgram, Vec<String>);

    struct ScriptedRunner {
        outputs: RefCell<Vec<BackendSuccess>>,
        calls: RefCell<Vec<RecordedCall>>,
    }

    impl ScriptedRunner {
        fn new(outputs: Vec<BackendSuccess>) -> Self {
            Self {
                outputs: RefCell::new(outputs),
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<RecordedCall> {
            self.calls.borrow().clone()
        }
    }

    impl BackendRunner for ScriptedRunner {
        fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
            let argv = call
                .argv
                .iter()
                .map(|os| os.to_string_lossy().into_owned())
                .collect();
            self.calls.borrow_mut().push((call.program, argv));
            Ok(self.outputs.borrow_mut().remove(0))
        }

        fn run_raw(&self, call: &BackendCall) -> Result<BackendOutput, ForgeError> {
            self.run(call).map(|s| BackendOutput {
                exit_code: 0,
                status_success: true,
                stdout: s.stdout,
                stderr: s.stderr,
            })
        }
    }

    fn ctx(provider: Provider) -> ProviderContext {
        ProviderContext {
            provider,
            host: match provider {
                Provider::GitLab => "gitlab.com".into(),
                _ => "github.com".into(),
            },
            source: DetectionSource::Flag,
            repo: Some("acme/widgets".into()),
        }
    }

    fn global(provider: ProviderFlag, dry_run: bool) -> GlobalFlags {
        GlobalFlags {
            format: Some(OutputFormat::Json),
            remote: "origin".into(),
            provider: Some(provider),
            host: None,
            repo: Some("acme/widgets".into()),
            store_root: None,
            dry_run,
        }
    }

    fn args(thread: &str, body: Option<&str>, body_file: Option<&str>) -> PrReviewThreadReplyArgs {
        PrReviewThreadReplyArgs {
            id: 7,
            thread: thread.to_string(),
            body: body.map(str::to_string),
            body_file: body_file.map(str::to_string),
        }
    }

    #[test]
    fn build_reply_call_uses_plain_rest_reply_with_body() {
        let call = build_reply_call(&ctx(Provider::GitHub), 7, "9", "ack").unwrap();
        let argv = call.plan_argv();
        assert_eq!(call.program, BackendProgram::Gh);
        assert!(
            argv.iter()
                .any(|s| s == "repos/acme/widgets/pulls/7/comments/9/replies")
        );
        assert!(argv.iter().any(|s| s == "POST"));
        assert!(argv.iter().any(|s| s == "body=ack"));
        assert!(!argv.iter().any(|s| s.contains("mutation(")));
    }

    #[test]
    fn build_reply_call_adds_hostname_for_enterprise_host() {
        let mut ctx = ctx(Provider::GitHub);
        ctx.host = "internal.ghe.com".into();
        let argv = build_reply_call(&ctx, 7, "9", "ack").unwrap().plan_argv();
        let pos = argv
            .iter()
            .position(|s| s == "--hostname")
            .expect("enterprise host must be passed to gh api");
        assert_eq!(argv[pos + 1], "internal.ghe.com");
    }

    fn reply_target_json() -> BackendSuccess {
        BackendSuccess {
            stdout: r#"{"data":{"node":{"id":"PRRT_abc","viewerCanResolve":true,"isResolved":false,"comments":{"nodes":[{"fullDatabaseId":"9"}]}}}}"#.into(),
            stderr: String::new(),
        }
    }

    fn pr_view_json(number: u64) -> BackendSuccess {
        BackendSuccess {
            stdout: format!(
                r#"{{"number":{number},"url":"https://github.com/acme/widgets/pull/{number}","state":"OPEN","isDraft":false,"title":"demo","headRefName":"feat/x","baseRefName":"main","mergeable":"MERGEABLE","mergedAt":null,"labels":[]}}"#
            ),
            stderr: String::new(),
        }
    }

    fn github_threads_json(ids: &[&str]) -> BackendSuccess {
        let nodes: Vec<String> = ids
            .iter()
            .map(|id| {
                format!(
                    r#"{{"id":"{id}","isResolved":false,"isOutdated":false,"path":"src/lib.rs","diffSide":"RIGHT","line":10,"originalLine":10,"originalStartLine":null,"startDiffSide":null,"startLine":null,"subjectType":"LINE","comments":{{"nodes":[{{"id":"PRRC_1","author":{{"login":"reviewer"}},"body":"finding","createdAt":"t","url":"https://github.com/acme/widgets/pull/7#discussion_r1"}}],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}"#
                )
            })
            .collect();
        BackendSuccess {
            stdout: format!(
                r#"{{"data":{{"repository":{{"pullRequest":{{"headRefOid":"head-7","reviewThreads":{{"nodes":[{}],"pageInfo":{{"hasNextPage":false,"endCursor":null}}}}}}}}}}}}"#,
                nodes.join(",")
            ),
            stderr: String::new(),
        }
    }

    #[test]
    fn run_with_posts_single_reply_and_surfaces_comment_url() {
        let runner = ScriptedRunner::new(vec![
            pr_view_json(7),
            github_threads_json(&["PRRT_abc"]),
            reply_target_json(),
            BackendSuccess {
                stdout: r#"{"html_url":"https://github.com/acme/widgets/pull/7#discussion_r9"}"#
                    .into(),
                stderr: String::new(),
            },
        ]);
        let code = run_with(
            &runner,
            &global(ProviderFlag::Github, false),
            args("PRRT_abc", Some("ack"), None),
            OutputFormat::Json,
            |_| Some("git@github.com:acme/widgets.git".into()),
        )
        .expect("reply");
        assert_eq!(code, exit::SUCCESS);
        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            4,
            "membership and root-comment reads precede the reply"
        );
        assert!(
            calls[3]
                .1
                .iter()
                .any(|s| s == "repos/acme/widgets/pulls/7/comments/9/replies")
        );
    }

    #[test]
    fn run_with_reply_rejects_thread_from_another_pr_before_mutating() {
        let runner =
            ScriptedRunner::new(vec![pr_view_json(7), github_threads_json(&["PRRT_other"])]);
        let err = run_with(
            &runner,
            &global(ProviderFlag::Github, false),
            args("PRRT_target", Some("ack"), None),
            OutputFormat::Json,
            |_| Some("git@github.com:acme/widgets.git".into()),
        )
        .expect_err("thread does not belong to PR");
        assert_eq!(err.kind(), "review_thread_pr_mismatch");
        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "must only view PR and list its threads");
        assert!(
            !calls.iter().any(|(_, argv)| argv
                .iter()
                .any(|s| s == "repos/acme/widgets/pulls/7/comments/9/replies")),
            "reply mutation must not run when the thread is not on the PR"
        );
    }

    #[test]
    fn run_with_rejects_empty_body() {
        let runner = ScriptedRunner::new(vec![]);
        let err = run_with(
            &runner,
            &global(ProviderFlag::Github, false),
            args("PRRT_abc", Some("   "), None),
            OutputFormat::Json,
            |_| Some("git@github.com:acme/widgets.git".into()),
        )
        .expect_err("empty body");
        assert_eq!(err.kind(), "body_missing_summary");
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn run_with_dry_run_plans_nothing() {
        let runner = ScriptedRunner::new(vec![]);
        let code = run_with(
            &runner,
            &global(ProviderFlag::Github, true),
            args("PRRT_abc", Some("ack"), None),
            OutputFormat::Json,
            |_| Some("git@github.com:acme/widgets.git".into()),
        )
        .expect("dry-run");
        assert_eq!(code, exit::SUCCESS);
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn run_with_gitlab_is_provider_unsupported() {
        let runner = ScriptedRunner::new(vec![]);
        let err = run_with(
            &runner,
            &global(ProviderFlag::Gitlab, false),
            args("d_1", Some("ack"), None),
            OutputFormat::Json,
            |_| Some("git@gitlab.com:acme/widgets.git".into()),
        )
        .expect_err("gitlab unsupported");
        assert_eq!(err.kind(), "provider_unsupported");
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn run_with_local_is_provider_unsupported() {
        let runner = ScriptedRunner::new(vec![]);
        let err = run_with(
            &runner,
            &global(ProviderFlag::Local, false),
            args("x", Some("ack"), None),
            OutputFormat::Json,
            |_| None,
        )
        .expect_err("local unsupported");
        assert_eq!(err.kind(), "provider_unsupported");
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn parse_comment_url_extracts_url_or_empty() {
        assert_eq!(parse_comment_url(r#"{"html_url":"u"}"#), "u");
        assert_eq!(parse_comment_url("{}"), "");
        assert_eq!(parse_comment_url("not json"), "");
    }
}
