//! Read GitHub's closed and reopened issue timeline entries.

use std::collections::HashSet;
use std::ffi::OsString;

use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::backend::{BackendCall, BackendProgram, BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, IssueEventsArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::{Provider, ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;

const MAX_PAGES: usize = 100;
const QUERY: &str = r#"query($owner: String!, $name: String!, $number: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    issue(number: $number) {
      timelineItems(first: 100, after: $after, itemTypes: [CLOSED_EVENT, REOPENED_EVENT]) {
        nodes {
          __typename
          ... on ClosedEvent {
            actor { login }
            created_at: createdAt
            closer {
              __typename
              ... on Commit { oid url }
              ... on PullRequest { number url }
            }
          }
          ... on ReopenedEvent { actor { login } created_at: createdAt }
        }
        page_info: pageInfo { has_next_page: hasNextPage end_cursor: endCursor }
      }
    }
  }
}"#;

#[derive(Serialize)]
struct IssueEventsPayload<'a> {
    provider: &'static str,
    repo: &'a str,
    number: i32,
    events: Vec<IssueEvent>,
}

#[derive(Serialize)]
struct IssueEvent {
    event: EventKind,
    actor: Option<String>,
    created_at: String,
    commit: Option<Commit>,
    pull_request: Option<PullRequest>,
}

#[derive(Deserialize, Serialize)]
enum EventKind {
    #[serde(rename(deserialize = "ClosedEvent", serialize = "closed"))]
    Closed,
    #[serde(rename(deserialize = "ReopenedEvent", serialize = "reopened"))]
    Reopened,
}

#[derive(Deserialize)]
struct GithubPage {
    nodes: Vec<GithubEvent>,
    page_info: PageInfo,
}

#[derive(Deserialize)]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct GithubEvent {
    #[serde(rename = "__typename")]
    event: EventKind,
    actor: Option<Actor>,
    created_at: String,
    closer: Option<Closer>,
}

#[derive(Deserialize)]
struct Actor {
    login: String,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum Closer {
    Commit(Commit),
    PullRequest(PullRequest),
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Serialize)]
struct Commit {
    oid: String,
    url: String,
}

#[derive(Deserialize, Serialize)]
struct PullRequest {
    number: u64,
    url: String,
}

impl From<GithubEvent> for IssueEvent {
    fn from(row: GithubEvent) -> Self {
        let (commit, pull_request) = match row.closer {
            Some(Closer::Commit(commit)) => (Some(commit), None),
            Some(Closer::PullRequest(pr)) => (None, Some(pr)),
            Some(Closer::Other) | None => (None, None),
        };
        Self {
            event: row.event,
            actor: row.actor.map(|actor| actor.login),
            created_at: row.created_at,
            commit,
            pull_request,
        }
    }
}

pub fn run(
    global: &GlobalFlags,
    args: IssueEventsArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        git_remote_url,
    )?;
    if ctx.provider != Provider::GitHub {
        return Err(unsupported(ctx.provider));
    }
    let repo = ctx.repo.as_deref().ok_or_else(|| {
        ForgeError::validation(
            error_schema(),
            "repo_required",
            "issue events requires --repo owner/name or a GitHub remote",
            None,
        )
    })?;
    let schema = schema_version_for(BINARY, "issue.events", 1);
    if global.dry_run {
        let call = page_call(&ctx, repo, args.id, None);
        return Ok(emit_success(
            schema,
            DryRunPayload::new(ctx.provider, &call),
            format,
            |payload| println!("would run: {}", payload.plan.join(" ")),
        ));
    }

    let payload = IssueEventsPayload {
        provider: ctx.provider.as_str(),
        repo,
        number: args.id,
        events: fetch_events(&default_runner(), &ctx, repo, args.id)?,
    };
    Ok(emit_success(schema, payload, format, render_text))
}

fn fetch_events(
    runner: &impl BackendRunner,
    ctx: &ProviderContext,
    repo: &str,
    number: i32,
) -> Result<Vec<IssueEvent>, ForgeError> {
    let mut events = Vec::new();
    let mut after = None;
    let mut seen_cursors = HashSet::new();
    for _ in 0..MAX_PAGES {
        let output = runner.run(&page_call(ctx, repo, number, after.as_deref()))?;
        let page = parse_page(&output.stdout)?;
        events.extend(page.nodes.into_iter().map(IssueEvent::from));
        if !page.page_info.has_next_page {
            return Ok(events);
        }
        let next = page
            .page_info
            .end_cursor
            .filter(|cursor| !cursor.is_empty())
            .ok_or_else(|| incomplete("GitHub omitted the next page cursor"))?;
        if !seen_cursors.insert(next.clone()) {
            return Err(incomplete("GitHub returned a repeated cursor"));
        }
        after = Some(next);
    }
    Err(incomplete(format!(
        "GitHub timeline exceeded the {MAX_PAGES}-page limit"
    )))
}

fn page_call(ctx: &ProviderContext, repo: &str, number: i32, after: Option<&str>) -> BackendCall {
    let (owner, name) = repo.split_once('/').expect("validated GitHub repository");
    let mut argv = vec![OsString::from("api"), OsString::from("graphql")];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        "-f".into(),
        format!("owner={owner}").into(),
        "-f".into(),
        format!("name={name}").into(),
        "-F".into(),
        format!("number={number}").into(),
        "-f".into(),
        format!("query={QUERY}").into(),
    ]);
    if let Some(after) = after {
        argv.extend(["-f".into(), format!("after={after}").into()]);
    }
    BackendCall::new(BackendProgram::Gh, argv)
}

fn parse_page(stdout: &str) -> Result<GithubPage, ForgeError> {
    let response: Value =
        serde_json::from_str(stdout).map_err(|_| incomplete("GitHub returned invalid JSON"))?;
    if response["errors"]
        .as_array()
        .is_some_and(|errors| !errors.is_empty())
    {
        return Err(incomplete("GitHub returned GraphQL errors"));
    }
    let timeline = response
        .pointer("/data/repository/issue/timelineItems")
        .ok_or_else(|| incomplete("GitHub response omitted the issue timeline"))?;
    GithubPage::deserialize(timeline)
        .map_err(|err| incomplete(format!("GitHub returned an invalid timeline page ({err})")))
}

pub(crate) fn unsupported(provider: Provider) -> ForgeError {
    ForgeError::provider_unsupported(
        error_schema(),
        format!(
            "issue events supports GitHub only (provider: {})",
            provider.as_str()
        ),
        None,
    )
}

fn incomplete(message: impl Into<String>) -> ForgeError {
    ForgeError::validation(error_schema(), "issue_events_incomplete", message, None)
}

fn error_schema() -> String {
    schema_version_for(BINARY, "error", 1)
}

fn render_text(payload: &IssueEventsPayload<'_>) {
    if payload.events.is_empty() {
        println!(
            "No close or reopen events for {}#{}",
            payload.repo, payload.number
        );
    }
    for item in &payload.events {
        let event = match item.event {
            EventKind::Closed => "closed",
            EventKind::Reopened => "reopened",
        };
        let actor = item.actor.as_deref().unwrap_or("unknown");
        let reference = item
            .commit
            .as_ref()
            .map(|commit| &commit.url)
            .or_else(|| item.pull_request.as_ref().map(|pr| &pr.url));
        print!("{} {event} by {actor}", item.created_at);
        if let Some(url) = reference {
            print!(" {url}");
        }
        println!();
    }
}
