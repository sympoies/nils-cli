//! `pr review-threads resolve` atom — resolve a review thread, optionally
//! posting a reply first.
//!
//! Spec / ops: `cli.forge-cli.pr.review-threads.resolve.v1`. GitHub-first:
//! the thread node id (`PRRT_...` from the read surface) keys ownership and
//! permission reads and the `resolveReviewThread` mutation. With `--note` /
//! `--note-file`, a plain REST comment reply precedes resolution, after checking
//! `viewerCanResolve`. An already-resolved thread succeeds without a second
//! resolution mutation.
//!
//! GitLab and Local have no GitHub-shaped thread-mutation surface, so they
//! return a structured `provider_unsupported` error (GitHub-first in v1).

use std::ffi::OsString;

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;

use crate::backend::{BackendCall, BackendProgram, BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, PrReviewThreadResolveArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::pr_comment::read_body_with_file_flag;
use crate::ops::{pr_review_thread_reply, pr_review_threads, review_state};
use crate::provider::{Provider, ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;
use crate::validations::{no_agent_attribution, no_local_path};

const SCHEMA: &str = "pr.review-threads.resolve";
const SCHEMA_VERSION: u32 = 1;

/// GitHub mutation that resolves a review thread. Idempotent: resolving an
/// already-resolved thread succeeds.
const GITHUB_RESOLVE_MUTATION: &str = "mutation($tid: ID!) { resolveReviewThread(input: {threadId: $tid}) { thread { isResolved } } }";

/// Envelope payload for `cli.forge-cli.pr.review-threads.resolve.v1`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PrReviewThreadResolvePayload {
    pub provider: &'static str,
    pub thread_id: String,
    pub resolved: bool,
    pub replied: bool,
}

pub fn run(
    global: &GlobalFlags,
    args: PrReviewThreadResolveArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let runner = default_runner();
    run_with(&runner, global, args, format, git_remote_url)
}

pub fn run_with<R: BackendRunner, F: Fn(&str) -> Option<String>>(
    runner: &R,
    global: &GlobalFlags,
    args: PrReviewThreadResolveArgs,
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

    // Resolve the optional reply note. An empty note (e.g. blank file) is
    // treated as "no note" so an accidental empty body doesn't post a comment.
    let note = read_body_with_file_flag(
        args.note.as_deref(),
        args.note_file.as_deref(),
        "--note-file",
    )?;
    let note = if note.trim().is_empty() {
        None
    } else {
        no_local_path(&note, "note")?;
        no_agent_attribution(&note, "note")?;
        Some(format!(
            "{}\n{}",
            note.trim_end(),
            review_state::thread_disposition_marker(&args.thread)
        ))
    };

    let reply_call = note
        .as_deref()
        .map(|body| {
            pr_review_thread_reply::build_reply_call(&ctx, args.id, "${root_comment_id}", body)
        })
        .transpose()?;
    let resolve_call = build_resolve_call(&ctx, &args.thread);

    if global.dry_run {
        // Emit the plan(s) and invoke nothing. The resolve call is always
        // planned; the reply call is planned only when a note is supplied.
        let mut plan: Vec<String> = Vec::new();
        if let Some(call) = &reply_call {
            plan.extend(call.plan_argv());
        }
        let resolve_plan = DryRunPayload::new(ctx.provider, &resolve_call);
        plan.extend(resolve_plan.plan.clone());
        return Ok(emit_success(
            schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION),
            pr_review_thread_reply::dry_run_payload(
                &ctx,
                &args.thread,
                DryRunPayload {
                    provider: ctx.provider.as_str(),
                    plan,
                    review_convergence: None,
                },
            ),
            format,
            |p| {
                println!(
                    "would read resolution permissions and reply target: {}",
                    p.target_plan.join(" ")
                );
                println!(
                    "would run after binding root_comment_id and checking permissions: {}",
                    p.mutation.plan.join(" ")
                );
            },
        ));
    }

    pr_review_threads::ensure_thread_belongs_to_pr(runner, &ctx, args.id, &args.thread)?;

    let target = pr_review_thread_reply::read_reply_target(runner, &ctx, &args.thread)?;
    if !target.resolved && !target.can_resolve {
        return Err(ForgeError::validation(
            schema_err(),
            "review_thread_resolve_forbidden",
            "the invoking identity cannot resolve this review thread; use an identity with resolution permission before posting a note",
            Some("field=viewerCanResolve; mutation_started=false".to_string()),
        ));
    }
    let replied = if let Some(body) = note.as_deref() {
        runner.run(&pr_review_thread_reply::build_reply_call(
            &ctx,
            args.id,
            &target.comment_id,
            body,
        )?)?;
        true
    } else {
        false
    };
    if !target.resolved {
        runner.run(&resolve_call)?;
    }

    Ok(emit_success(
        schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION),
        PrReviewThreadResolvePayload {
            provider: ctx.provider.as_str(),
            thread_id: args.thread,
            resolved: true,
            replied,
        },
        format,
        render_text,
    ))
}

/// GitHub-first: GitLab and Local have no GitHub-shaped thread-mutation
/// surface, so they fail closed with a structured `provider_unsupported`
/// error before any backend call.
fn ensure_github(ctx: &ProviderContext) -> Result<(), ForgeError> {
    match ctx.provider {
        Provider::GitHub => Ok(()),
        Provider::GitLab | Provider::Local => Err(ForgeError::provider_unsupported(
            schema_err(),
            format!(
                "pr review-threads resolve is GitHub-only in v1 (provider: {provider})",
                provider = ctx.provider.as_str(),
            ),
            None,
        )),
    }
}

pub(crate) fn build_resolve_call(ctx: &ProviderContext, thread_id: &str) -> BackendCall {
    debug_assert!(matches!(ctx.provider, Provider::GitHub));
    let mut argv = vec![OsString::from("api"), OsString::from("graphql")];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        OsString::from("-f"),
        OsString::from(format!("query={GITHUB_RESOLVE_MUTATION}")),
        OsString::from("-f"),
        OsString::from(format!("tid={thread_id}")),
    ]);
    BackendCall::new(BackendProgram::Gh, argv)
}

fn schema_err() -> String {
    schema_version_for(BINARY, "error", 1)
}

fn render_text(payload: &PrReviewThreadResolvePayload) {
    let replied = if payload.replied {
        " (replied first)"
    } else {
        ""
    };
    println!(
        "resolved {provider} review thread {thread}{replied}",
        provider = payload.provider,
        thread = payload.thread_id,
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

    fn args(
        thread: &str,
        note: Option<&str>,
        note_file: Option<&str>,
    ) -> PrReviewThreadResolveArgs {
        PrReviewThreadResolveArgs {
            id: 7,
            thread: thread.to_string(),
            note: note.map(str::to_string),
            note_file: note_file.map(str::to_string),
        }
    }

    #[test]
    fn build_resolve_call_uses_resolve_review_thread_mutation_with_tid() {
        let call = build_resolve_call(&ctx(Provider::GitHub), "PRRT_abc");
        let argv = call.plan_argv();
        assert_eq!(call.program, BackendProgram::Gh);
        assert!(argv.iter().any(|s| s == "graphql"));
        assert!(argv.iter().any(|s| s.contains("resolveReviewThread")));
        assert!(argv.iter().any(|s| s.contains("threadId")));
        assert!(argv.iter().any(|s| s == "tid=PRRT_abc"));
    }

    #[test]
    fn build_resolve_call_adds_hostname_for_enterprise_host() {
        let mut ctx = ctx(Provider::GitHub);
        ctx.host = "internal.ghe.com".into();
        let argv = build_resolve_call(&ctx, "PRRT_abc").plan_argv();
        let pos = argv
            .iter()
            .position(|s| s == "--hostname")
            .expect("enterprise host must be passed to gh api");
        assert_eq!(argv[pos + 1], "internal.ghe.com");
    }

    #[test]
    fn build_reply_call_uses_plain_rest_reply_with_body() {
        let call = pr_review_thread_reply::build_reply_call(&ctx(Provider::GitHub), 7, "9", "ack")
            .unwrap();
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
        let argv = pr_review_thread_reply::build_reply_call(&ctx, 7, "9", "ack")
            .unwrap()
            .plan_argv();
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
    fn run_with_resolve_only_runs_single_mutation() {
        let runner = ScriptedRunner::new(vec![
            pr_view_json(7),
            github_threads_json(&["PRRT_abc"]),
            reply_target_json(),
            BackendSuccess {
                stdout: r#"{"data":{"resolveReviewThread":{"thread":{"isResolved":true}}}}"#.into(),
                stderr: String::new(),
            },
        ]);
        let code = run_with(
            &runner,
            &global(ProviderFlag::Github, false),
            args("PRRT_abc", None, None),
            OutputFormat::Json,
            |_| Some("git@github.com:acme/widgets.git".into()),
        )
        .expect("resolve only");
        assert_eq!(code, exit::SUCCESS);
        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            4,
            "membership and permission reads precede resolution"
        );
        assert!(calls[3].1.iter().any(|s| s.contains("resolveReviewThread")));
    }

    #[test]
    fn run_with_resolve_rejects_thread_from_another_pr_before_mutating() {
        let runner =
            ScriptedRunner::new(vec![pr_view_json(7), github_threads_json(&["PRRT_other"])]);
        let err = run_with(
            &runner,
            &global(ProviderFlag::Github, false),
            args("PRRT_target", None, None),
            OutputFormat::Json,
            |_| Some("git@github.com:acme/widgets.git".into()),
        )
        .expect_err("thread does not belong to PR");
        assert_eq!(err.kind(), "review_thread_pr_mismatch");
        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "must only view PR and list its threads");
        assert!(
            !calls
                .iter()
                .any(|(_, argv)| argv.iter().any(|s| s.contains("resolveReviewThread"))),
            "resolve mutation must not run when the thread is not on the PR"
        );
    }

    #[test]
    fn run_with_note_replies_before_resolving() {
        let runner = ScriptedRunner::new(vec![
            pr_view_json(7),
            github_threads_json(&["PRRT_abc"]),
            reply_target_json(),
            BackendSuccess {
                stdout: r#"{"data":{"addPullRequestReviewThreadReply":{"comment":{"url":"u"}}}}"#
                    .into(),
                stderr: String::new(),
            },
            BackendSuccess {
                stdout: r#"{"data":{"resolveReviewThread":{"thread":{"isResolved":true}}}}"#.into(),
                stderr: String::new(),
            },
        ]);
        let code = run_with(
            &runner,
            &global(ProviderFlag::Github, false),
            args("PRRT_abc", Some("done, accepted"), None),
            OutputFormat::Json,
            |_| Some("git@github.com:acme/widgets.git".into()),
        )
        .expect("reply then resolve");
        assert_eq!(code, exit::SUCCESS);
        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            5,
            "membership and permission reads precede reply and resolution"
        );
        assert!(
            calls[3]
                .1
                .iter()
                .any(|s| s == "repos/acme/widgets/pulls/7/comments/9/replies")
        );
        assert!(calls[4].1.iter().any(|s| s.contains("resolveReviewThread")));
    }

    #[test]
    fn resolve_permission_is_checked_before_posting_a_note() {
        let runner = ScriptedRunner::new(vec![
            pr_view_json(7), github_threads_json(&["PRRT_abc"]),
            BackendSuccess { stdout: r#"{"data":{"node":{"id":"PRRT_abc","viewerCanResolve":false,"isResolved":false,"comments":{"nodes":[{"fullDatabaseId":"9"}]}}}}"#.into(), stderr: String::new() },
            BackendSuccess { stdout: "{}".into(), stderr: String::new() },
        ]);
        let err = run_with(
            &runner,
            &global(ProviderFlag::Github, false),
            args("PRRT_abc", Some("Fixed in the current head."), None),
            OutputFormat::Json,
            |_| Some("git@github.com:acme/widgets.git".into()),
        )
        .expect_err("permission refusal before note");
        assert_eq!(err.kind(), "review_thread_resolve_forbidden");
        assert_eq!(runner.calls().len(), 3);
        assert!(
            !runner
                .calls()
                .iter()
                .any(|(_, argv)| argv.iter().any(|s| s.contains("mutation(") || s == "POST"))
        );
    }

    #[test]
    fn resolution_note_is_a_plain_comment_reply() {
        let runner = ScriptedRunner::new(vec![pr_view_json(7), github_threads_json(&["PRRT_abc"]),
            BackendSuccess { stdout: r#"{"data":{"node":{"id":"PRRT_abc","viewerCanResolve":true,"isResolved":false,"comments":{"nodes":[{"fullDatabaseId":"9"}]}}}}"#.into(), stderr: String::new() },
            BackendSuccess { stdout: r#"{"html_url":"https://github.com/acme/widgets/pull/7#discussion_r10"}"#.into(), stderr: String::new() },
            BackendSuccess { stdout: r#"{"data":{"resolveReviewThread":{"thread":{"isResolved":true}}}}"#.into(), stderr: String::new() },
        ]);
        run_with(
            &runner,
            &global(ProviderFlag::Github, false),
            args("PRRT_abc", Some("Fixed."), None),
            OutputFormat::Json,
            |_| Some("git@github.com:acme/widgets.git".into()),
        )
        .unwrap();
        let calls = runner.calls();
        assert!(
            calls.iter().any(|(_, argv)| argv
                .iter()
                .any(|s| s == "repos/acme/widgets/pulls/7/comments/9/replies")),
            "{calls:?}"
        );
        assert!(
            !calls.iter().any(|(_, argv)| argv
                .iter()
                .any(|s| s.contains("addPullRequestReviewThreadReply"))),
            "{calls:?}"
        );
    }

    #[test]
    fn run_with_dry_run_plans_nothing() {
        let runner = ScriptedRunner::new(vec![]);
        let code = run_with(
            &runner,
            &global(ProviderFlag::Github, true),
            args("PRRT_abc", Some("note"), None),
            OutputFormat::Json,
            |_| Some("git@github.com:acme/widgets.git".into()),
        )
        .expect("dry-run");
        assert_eq!(code, exit::SUCCESS);
        assert!(
            runner.calls().is_empty(),
            "dry-run must not invoke the backend"
        );
    }

    #[test]
    fn run_with_gitlab_is_provider_unsupported() {
        let runner = ScriptedRunner::new(vec![]);
        let err = run_with(
            &runner,
            &global(ProviderFlag::Gitlab, false),
            args("d_1", None, None),
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
            args("x", None, None),
            OutputFormat::Json,
            |_| None,
        )
        .expect_err("local unsupported");
        assert_eq!(err.kind(), "provider_unsupported");
        assert!(runner.calls().is_empty());
    }
}
