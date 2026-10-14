use std::ffi::OsString;

use serde_json::Value;

use crate::backend::{BackendCall, BackendProgram, BackendRunner};
use crate::provider::{Provider, ProviderContext};

const FACTS: &str = r#"
__typename
... on Issue {
  id number url title issueState: state stateReason createdAt closedAt updatedAt
  repository { nameWithOwner isFork isArchived }
  author { __typename login }
  labels(first: $first, after: $labelsAfter) {
    totalCount nodes { name } pageInfo { hasNextPage endCursor }
  }
}
... on PullRequest {
  id number url title prState: state createdAt closedAt mergedAt updatedAt additions deletions changedFiles
  repository { nameWithOwner isFork isArchived }
  author { __typename login }
  labels(first: $first, after: $labelsAfter) {
    totalCount nodes { name } pageInfo { hasNextPage endCursor }
  }
  closingIssuesReferences(first: $first, after: $referencesAfter) {
    totalCount nodes { number url repository { nameWithOwner } }
    pageInfo { hasNextPage endCursor }
  }
}
"#;

pub(super) struct Page {
    pub nodes: Vec<Value>,
    pub total_count: u32,
    pub has_next: bool,
    pub end_cursor: Option<String>,
}

pub(super) fn search_call(
    ctx: &ProviderContext,
    query: &str,
    first: u32,
    after: Option<&str>,
) -> BackendCall {
    let graphql = format!(
        r#"query CompletionSearch($searchQuery: String!, $first: Int!, $searchAfter: String, $labelsAfter: String, $referencesAfter: String) {{
      search(query: $searchQuery, type: ISSUE, first: $first, after: $searchAfter) {{
        issueCount nodes {{ {FACTS} }} pageInfo {{ hasNextPage endCursor }}
      }}
    }}"#
    );
    let mut call = api_call(ctx, graphql, first);
    field(&mut call, "searchQuery", query);
    if let Some(after) = after {
        field(&mut call, "searchAfter", after);
    }
    call
}

pub(super) fn node_call(
    ctx: &ProviderContext,
    node: &str,
    first: u32,
    labels_after: Option<&str>,
    references_after: Option<&str>,
) -> BackendCall {
    let graphql = format!(
        r#"query CompletionNode($node: ID!, $first: Int!, $labelsAfter: String, $referencesAfter: String) {{
      node(id: $node) {{ {FACTS} }}
    }}"#
    );
    let mut call = api_call(ctx, graphql, first);
    field(&mut call, "node", node);
    if let Some(after) = labels_after {
        field(&mut call, "labelsAfter", after);
    }
    if let Some(after) = references_after {
        field(&mut call, "referencesAfter", after);
    }
    call
}

fn api_call(ctx: &ProviderContext, query: String, first: u32) -> BackendCall {
    let mut argv = vec![OsString::from("api"), OsString::from("graphql")];
    ctx.push_github_api_hostname(&mut argv);
    argv.extend([
        OsString::from("-f"),
        OsString::from(format!("query={query}")),
        OsString::from("-F"),
        OsString::from(format!("first={first}")),
    ]);
    BackendCall::new(BackendProgram::Gh, argv).with_host(Provider::GitHub, &ctx.host)
}

fn field(call: &mut BackendCall, name: &str, value: &str) {
    call.argv.extend([
        OsString::from("-f"),
        OsString::from(format!("{name}={value}")),
    ]);
}

fn response<R: BackendRunner>(runner: &R, call: &BackendCall) -> Result<Value, &'static str> {
    let output = runner.run(call).map_err(|error| error.kind())?;
    let value: Value =
        serde_json::from_str(output.stdout.trim()).map_err(|_| "provider_response_invalid")?;
    if value
        .get("errors")
        .is_some_and(|errors| !errors.as_array().is_some_and(Vec::is_empty))
    {
        return Err("provider_graphql_error");
    }
    if !value.get("data").is_some_and(Value::is_object) {
        return Err("provider_response_invalid");
    }
    Ok(value)
}

pub(super) fn search<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    query: &str,
    first: u32,
    after: Option<&str>,
) -> Result<Page, &'static str> {
    let value = response(runner, &search_call(ctx, query, first, after))?;
    let search = value
        .pointer("/data/search")
        .ok_or("provider_response_invalid")?;
    let nodes = search
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or("provider_response_invalid")?
        .clone();
    let total_count = search
        .get("issueCount")
        .and_then(Value::as_u64)
        .and_then(|count| count.try_into().ok())
        .ok_or("provider_response_invalid")?;
    let (has_next, end_cursor) = page_info(search)?;
    if nodes.len() > first as usize
        || (has_next && (nodes.is_empty() || end_cursor.as_deref() == after))
    {
        return Err("provider_pagination_invalid");
    }
    Ok(Page {
        nodes,
        total_count,
        has_next,
        end_cursor,
    })
}

pub(super) fn node<R: BackendRunner>(
    runner: &R,
    ctx: &ProviderContext,
    id: &str,
    first: u32,
    labels_after: Option<&str>,
    references_after: Option<&str>,
) -> Result<Value, &'static str> {
    let value = response(
        runner,
        &node_call(ctx, id, first, labels_after, references_after),
    )?;
    let node = value
        .pointer("/data/node")
        .filter(|node| node.is_object())
        .ok_or("provider_node_unavailable")?;
    if node.get("id").and_then(Value::as_str) != Some(id) {
        return Err("provider_node_invalid");
    }
    Ok(node.clone())
}

pub(super) fn page_info(connection: &Value) -> Result<(bool, Option<String>), &'static str> {
    let info = connection
        .get("pageInfo")
        .ok_or("provider_pagination_invalid")?;
    let has_next = info
        .get("hasNextPage")
        .and_then(Value::as_bool)
        .ok_or("provider_pagination_invalid")?;
    let end_cursor = match info.get("endCursor") {
        Some(Value::Null) => None,
        Some(Value::String(cursor)) if !cursor.is_empty() => Some(cursor.clone()),
        _ => return Err("provider_pagination_invalid"),
    };
    if has_next && end_cursor.is_none() {
        return Err("provider_pagination_invalid");
    }
    Ok((has_next, end_cursor))
}
