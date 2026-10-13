//! Completion facts through the selected provider boundary, with explicit
//! coverage of the search and every nested connection.

mod cursor;
mod facts;
mod graphql;
#[cfg(test)]
mod review_regressions;
#[cfg(test)]
mod tests;

use std::collections::HashSet;

use jiff::Timestamp;
use nils_common::cli_contract::OutputFormat;
use serde::Serialize;
use serde_json::Value;

use crate::backend::BackendRunner;
use crate::cli::{CompletionKind, CompletionQueryArgs, GlobalFlags};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::provider::ProviderContext;

use cursor::{Cursor, Pending};
use facts::{Connection, Item, Relation};

#[derive(Serialize)]
pub(super) struct Window {
    #[serde(serialize_with = "serialize_timestamp")]
    since: Timestamp,
    #[serde(serialize_with = "serialize_timestamp")]
    until: Timestamp,
}

fn serialize_timestamp<S: serde::Serializer>(
    timestamp: &Timestamp,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&timestamp.to_string())
}

impl Window {
    fn new(args: &CompletionQueryArgs) -> Result<Self, ForgeError> {
        let invalid = || {
            ForgeError::validation(
                super::schema_err(),
                "completion_window_invalid",
                "completion window requires valid RFC 3339 timestamps with since before until",
                None,
            )
        };
        let since = args.since.parse().map_err(|_| invalid())?;
        let until = args.until.parse().map_err(|_| invalid())?;
        if since >= until {
            return Err(invalid());
        }
        Ok(Self { since, until })
    }

    fn query(&self, args: &CompletionQueryArgs, repo: &str) -> Result<String, ForgeError> {
        // Search indexes may only resolve whole seconds. Fetch a superset and
        // enforce the precise half-open interval on canonical completion facts.
        let lower =
            Timestamp::from_second(self.since.as_second()).map_err(|_| super::missing("since"))?;
        let upper = Timestamp::from_second(
            self.until.as_second()
                + if self.until.subsec_nanosecond() != 0 {
                    1
                } else {
                    0
                },
        )
        .map_err(|_| super::missing("until"))?;
        let (selection, time_field) = match args.kind {
            CompletionKind::Issue => ("is:issue is:closed", "closed"),
            CompletionKind::Pr => ("is:pr is:merged", "merged"),
        };
        let scope = if repo.is_empty() {
            String::new()
        } else {
            format!(" repo:{repo}")
        };
        Ok(format!(
            "{}{} {selection} {time_field}:{lower}..{upper}",
            args.query, scope
        )
        .trim()
        .to_string())
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Failure {
    connection: &'static str,
    node_id: Option<String>,
    kind: &'static str,
}

#[derive(Serialize)]
pub(super) struct Pagination {
    status: &'static str,
    complete: bool,
    traversal_complete: bool,
    covered_from_start: bool,
    resumed: bool,
    start_cursor: Option<String>,
    next_cursor: Option<String>,
    pages_read: u32,
    scanned: u32,
    total_count: Option<u32>,
    search_ceiling: bool,
    pending_connections: usize,
    reasons: Vec<&'static str>,
    failures: Vec<Failure>,
}

#[derive(Serialize)]
pub(super) struct Payload {
    provider: &'static str,
    host: String,
    repo: String,
    kind: CompletionKind,
    query: String,
    effective_query: String,
    window: Window,
    item_count: usize,
    items: Vec<Item>,
    pagination: Pagination,
}

pub(super) fn run<R: BackendRunner>(
    runner: &R,
    global: &GlobalFlags,
    ctx: &ProviderContext,
    repo: &str,
    args: CompletionQueryArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let window = Window::new(&args)?;
    let query = window.query(&args, repo)?;
    let progress = Cursor::read(args.cursor.as_deref(), cursor::scope(ctx, &query, &args))?;
    if global.dry_run {
        let call = if let Some(pending) = progress.pending.first() {
            graphql::node_call(
                ctx,
                &pending.node_id,
                args.page_size,
                pending.labels_after.as_deref(),
                pending.references_after.as_deref(),
            )
        } else {
            graphql::search_call(ctx, &query, args.page_size, progress.after.as_deref())
        };
        return Ok(super::emit_search_dry_run(
            super::schema_ok("search.completions"),
            ctx,
            call,
            format,
        ));
    }
    let payload = read(runner, ctx, repo, args, window, query, progress);
    Ok(emit_success(
        super::schema_ok("search.completions"),
        payload,
        format,
        |payload| {
            println!(
                "{}@{} completions {}: {} item(s), {}",
                payload.provider,
                payload.host,
                payload.repo,
                payload.item_count,
                payload.pagination.status
            );
            for item in &payload.items {
                println!(
                    "{} {:?} #{} {} {}",
                    item.repo,
                    item.kind,
                    item.number,
                    item.completed_at.as_deref().unwrap_or("unknown"),
                    item.url
                );
            }
        },
    ))
}

fn read<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    repo: &str,
    args: CompletionQueryArgs,
    window: Window,
    query: String,
    mut progress: Cursor,
) -> Payload {
    let covered_from_start = progress.scanned == 0 && progress.pending.is_empty();
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut failures = Vec::new();
    let mut pages = 0;
    let pending = std::mem::take(&mut progress.pending);
    for pending in pending {
        match graphql::node(
            runner,
            ctx,
            &pending.node_id,
            args.page_size,
            pending.labels_after.as_deref(),
            pending.references_after.as_deref(),
        ) {
            Ok(node) => match process_node(
                runner,
                ctx,
                &args,
                &window,
                &node,
                (
                    pending.labels_after.as_deref(),
                    pending.references_after.as_deref(),
                ),
                &mut failures,
            ) {
                Ok((item, continuation)) => {
                    if let Some(item) = item {
                        insert_item(&mut items, &mut seen, item);
                    }
                    if let Some(pending) = continuation {
                        progress.pending.push(pending);
                    }
                }
                Err(kind) => {
                    failures.push(Failure {
                        connection: "node",
                        node_id: Some(pending.node_id.clone()),
                        kind,
                    });
                    progress.pending.push(pending);
                }
            },
            Err(kind) => {
                failures.push(Failure {
                    connection: "node",
                    node_id: Some(pending.node_id.clone()),
                    kind,
                });
                progress.pending.push(pending);
            }
        }
    }
    let mut search_cursors = HashSet::new();
    while progress.pending.is_empty()
        && !progress.search_done
        && pages < args.max_pages
        && progress.scanned < 1000
    {
        if !search_cursors.insert(progress.after.clone()) {
            failures.push(Failure {
                connection: "search",
                node_id: None,
                kind: "provider_pagination_invalid",
            });
            break;
        }
        let first = args.page_size.min(1000 - progress.scanned);
        let page = match graphql::search(runner, ctx, &query, first, progress.after.as_deref()) {
            Ok(page) => page,
            Err(kind) => {
                failures.push(Failure {
                    connection: "search",
                    node_id: None,
                    kind,
                });
                break;
            }
        };
        pages += 1;
        if progress
            .total_count
            .is_some_and(|count| count != page.total_count)
        {
            failures.push(Failure {
                connection: "search",
                node_id: None,
                kind: "provider_search_count_mismatch",
            });
        }
        progress.total_count = Some(progress.total_count.unwrap_or(0).max(page.total_count));
        let mut page_valid = true;
        for node in &page.nodes {
            match process_node(
                runner,
                ctx,
                &args,
                &window,
                node,
                (None, None),
                &mut failures,
            ) {
                Ok((item, continuation)) => {
                    if let Some(item) = item {
                        insert_item(&mut items, &mut seen, item);
                    }
                    if let Some(pending) = continuation {
                        progress.pending.push(pending);
                    }
                }
                Err(kind) => {
                    failures.push(Failure {
                        connection: "search",
                        node_id: None,
                        kind,
                    });
                    page_valid = false;
                    break;
                }
            }
        }
        if !page_valid {
            break;
        }
        let scanned = progress.scanned + page.nodes.len() as u32;
        if (!page.has_next && scanned != page.total_count.min(1000))
            || (scanned == 1000 && page.has_next && page.total_count <= 1000)
        {
            failures.push(Failure {
                connection: "search",
                node_id: None,
                kind: "provider_search_count_mismatch",
            });
            break;
        }
        progress.scanned = scanned;
        progress.after = page.end_cursor;
        progress.search_done = !page.has_next || scanned == 1000;
    }
    let ceiling = progress.total_count.is_some_and(|count| count > 1000);
    let truncated = ceiling || !progress.search_done || !progress.pending.is_empty();
    let fragment = items.iter().any(|item| {
        !item.labels_pagination.covered_from_start
            || !item.closing_references_pagination.covered_from_start
    });
    let missing = items.iter().any(|item| !item.missing_fields.is_empty());
    let partial = !failures.is_empty() || missing || fragment || !covered_from_start;
    let mut reasons = Vec::new();
    if ceiling {
        reasons.push("search_ceiling");
    }
    if !progress.search_done {
        reasons.push("search_pages_remaining");
    }
    if !progress.pending.is_empty() {
        reasons.push("nested_pages_remaining");
    }
    if !failures.is_empty() {
        reasons.push("read_failure");
    }
    if missing {
        reasons.push("missing_fields");
    }
    if fragment {
        reasons.push("nested_continuation_fragment");
    }
    if !covered_from_start {
        reasons.push("search_continuation_fragment");
    }
    let status = if partial {
        "partial"
    } else if truncated {
        "truncated"
    } else {
        "complete"
    };
    let traversal_complete =
        progress.search_done && progress.pending.is_empty() && failures.is_empty() && !ceiling;
    let next_cursor = if !progress.search_done || !progress.pending.is_empty() {
        Some(progress.encode())
    } else {
        None
    };
    Payload {
        provider: ctx.provider.as_str(),
        host: ctx.host.clone(),
        repo: repo.to_string(),
        kind: args.kind,
        query: args.query,
        effective_query: query,
        window,
        item_count: items.len(),
        items,
        pagination: Pagination {
            status,
            complete: status == "complete",
            traversal_complete,
            covered_from_start,
            resumed: args.cursor.is_some(),
            start_cursor: args.cursor,
            next_cursor,
            pages_read: pages,
            scanned: progress.scanned,
            total_count: progress.total_count,
            search_ceiling: ceiling,
            pending_connections: progress.pending.len(),
            reasons,
            failures,
        },
    }
}

fn insert_item(items: &mut Vec<Item>, seen: &mut HashSet<String>, item: Item) {
    let key = format!(
        "{}:{:?}:{}",
        item.repo.to_ascii_lowercase(),
        item.kind,
        item.number
    );
    if seen.insert(key) {
        items.push(item);
    }
}

fn process_node<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    args: &CompletionQueryArgs,
    window: &Window,
    value: &Value,
    starts: (Option<&str>, Option<&str>),
    failures: &mut Vec<Failure>,
) -> Result<(Option<Item>, Option<Pending>), &'static str> {
    let (labels_after, references_after) = starts;
    let (mut item, mut labels, mut references) =
        facts::item(value, args.kind, labels_after, references_after)?;
    item.in_window = item
        .completed_at
        .as_ref()
        .and_then(|time| time.parse::<Timestamp>().ok())
        .map(|time| time >= window.since && time < window.until);
    if item.in_window == Some(false)
        || (args.exclude_forks && item.repository.is_fork == Some(true))
        || (args.exclude_archived && item.repository.is_archived == Some(true))
    {
        return Ok((None, None));
    }
    drain(
        runner,
        ctx,
        args,
        &item.node_id,
        &mut labels,
        Relation::Labels,
        failures,
    );
    if args.kind == CompletionKind::Pr {
        drain(
            runner,
            ctx,
            args,
            &item.node_id,
            &mut references,
            Relation::References,
            failures,
        );
    }
    let pending = if !labels.coverage.remaining_complete || !references.coverage.remaining_complete
    {
        Some(Pending {
            node_id: item.node_id.clone(),
            labels_after: labels
                .coverage
                .next_cursor
                .clone()
                .or_else(|| labels_after.map(str::to_string)),
            references_after: references
                .coverage
                .next_cursor
                .clone()
                .or_else(|| references_after.map(str::to_string)),
        })
    } else {
        None
    };
    labels.deduplicate();
    references.deduplicate();
    item.labels = labels.values;
    item.labels_pagination = labels.coverage;
    item.closing_issues_references = references.values;
    item.closing_references_pagination = references.coverage;
    Ok((Some(item), pending))
}

fn drain<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    args: &CompletionQueryArgs,
    node_id: &str,
    connection: &mut Connection,
    relation: Relation,
    failures: &mut Vec<Failure>,
) {
    let mut visited = HashSet::new();
    while connection.coverage.failure.is_none()
        && connection.coverage.has_next_page == Some(true)
        && connection.coverage.pages_read < args.max_connection_pages
    {
        let after = connection
            .coverage
            .next_cursor
            .clone()
            .expect("validated next-page cursor");
        if !visited.insert(after.clone()) {
            connection.coverage.failure = Some("provider_pagination_invalid");
            break;
        }
        let (labels_after, references_after) = match relation {
            Relation::Labels => (Some(after.as_str()), None),
            Relation::References => (None, Some(after.as_str())),
        };
        match graphql::node(
            runner,
            ctx,
            node_id,
            args.page_size,
            labels_after,
            references_after,
        ) {
            Ok(value) => connection.append(Connection::parse(
                value.get(relation.field()),
                relation,
                Some(&after),
            )),
            Err(kind) => {
                connection.coverage.failure = Some(kind);
                connection.coverage.complete = false;
            }
        }
    }
    if let Some(kind) = connection.coverage.failure {
        failures.push(Failure {
            connection: relation.field(),
            node_id: Some(node_id.to_string()),
            kind,
        });
    }
}
