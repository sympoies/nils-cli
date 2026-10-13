use std::collections::HashSet;

use serde::Serialize;
use serde_json::{Value, json};

use crate::cli::CompletionKind;

#[derive(Debug, Serialize)]
pub(super) struct Author {
    pub login: Option<String>,
    pub kind: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct Repository {
    pub name_with_owner: String,
    pub is_fork: Option<bool>,
    pub is_archived: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Coverage {
    pub start_cursor: Option<String>,
    pub end_cursor: Option<String>,
    pub next_cursor: Option<String>,
    pub has_next_page: Option<bool>,
    pub total_count: Option<u64>,
    pub observed_count: usize,
    pub pages_read: u32,
    pub complete: bool,
    pub remaining_complete: bool,
    pub covered_from_start: bool,
    pub failure: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub(super) struct Item {
    pub node_id: String,
    pub repo: String,
    pub repository: Repository,
    pub kind: CompletionKind,
    pub number: u64,
    pub url: String,
    pub title: String,
    pub state: String,
    pub state_reason: Option<String>,
    pub created_at: Option<String>,
    pub closed_at: Option<String>,
    pub merged_at: Option<String>,
    pub completed_at: Option<String>,
    pub updated_at: Option<String>,
    pub author: Option<Author>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub changed_files: Option<u64>,
    pub labels: Option<Vec<Value>>,
    pub labels_pagination: Coverage,
    pub closing_issues_references: Option<Vec<Value>>,
    pub closing_references_pagination: Coverage,
    pub missing_fields: Vec<String>,
    pub in_window: Option<bool>,
}

pub(super) fn item(
    value: &Value,
    kind: CompletionKind,
    labels_after: Option<&str>,
    references_after: Option<&str>,
) -> Result<(Item, Connection, Connection), &'static str> {
    let expected = match kind {
        CompletionKind::Issue => "Issue",
        CompletionKind::Pr => "PullRequest",
    };
    if value.get("__typename").and_then(Value::as_str) != Some(expected) {
        return Err("provider_item_kind_invalid");
    }
    let id = required_string(value, "id")?;
    let repo = value
        .get("repository")
        .ok_or("provider_item_identity_invalid")?;
    let repo_name = required_string(repo, "nameWithOwner")?;
    if super::super::split_slug(&repo_name).is_none() {
        return Err("provider_item_identity_invalid");
    }
    let number = value
        .get("number")
        .and_then(Value::as_u64)
        .filter(|number| *number > 0)
        .ok_or("provider_item_identity_invalid")?;
    let url = required_string(value, "url")?;
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .ok_or("provider_item_identity_invalid")?
        .to_string();
    let state = required_string(value, "state")?;
    let mut missing = Vec::new();
    let created_at = timestamp(value, "createdAt", false, &mut missing);
    let closed_at = timestamp(value, "closedAt", true, &mut missing);
    let updated_at = timestamp(value, "updatedAt", false, &mut missing);
    let merged_at = if kind == CompletionKind::Pr {
        timestamp(value, "mergedAt", true, &mut missing)
    } else {
        None
    };
    let completed_at = match kind {
        CompletionKind::Issue => closed_at.clone(),
        CompletionKind::Pr => merged_at.clone(),
    };
    if completed_at.is_none() {
        missing.push("completion_timestamp".into());
    }
    let state_reason = if kind == CompletionKind::Issue {
        optional_string(value, "stateReason", true, &mut missing)
    } else {
        None
    };
    let author = match value.get("author") {
        Some(Value::Null) => None,
        Some(author) if author.is_object() => Some(Author {
            login: optional_string(author, "login", false, &mut missing),
            kind: optional_string(author, "__typename", false, &mut missing),
        }),
        _ => {
            missing.push("author".into());
            None
        }
    };
    let repository = Repository {
        name_with_owner: repo_name.clone(),
        is_fork: optional_bool(repo, "isFork", &mut missing),
        is_archived: optional_bool(repo, "isArchived", &mut missing),
    };
    let labels = Connection::parse(value.get("labels"), Relation::Labels, labels_after);
    let references = if kind == CompletionKind::Pr {
        Connection::parse(
            value.get("closingIssuesReferences"),
            Relation::References,
            references_after,
        )
    } else {
        Connection::not_applicable()
    };
    let (additions, deletions, changed_files) = if kind == CompletionKind::Pr {
        (
            optional_u64(value, "additions", &mut missing),
            optional_u64(value, "deletions", &mut missing),
            optional_u64(value, "changedFiles", &mut missing),
        )
    } else {
        (None, None, None)
    };
    let item = Item {
        node_id: id,
        repo: repo_name,
        repository,
        kind,
        number,
        url,
        title,
        state,
        state_reason,
        created_at,
        closed_at,
        merged_at,
        completed_at,
        updated_at,
        author,
        additions,
        deletions,
        changed_files,
        labels: labels.values.clone(),
        labels_pagination: labels.coverage.clone(),
        closing_issues_references: references.values.clone(),
        closing_references_pagination: references.coverage.clone(),
        missing_fields: missing,
        in_window: None,
    };
    Ok((item, labels, references))
}

fn required_string(value: &Value, field: &str) -> Result<String, &'static str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or("provider_item_identity_invalid")
}

fn optional_string(
    value: &Value,
    field: &str,
    nullable: bool,
    missing: &mut Vec<String>,
) -> Option<String> {
    match value.get(field) {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Null) if nullable => None,
        _ => {
            missing.push(field.to_string());
            None
        }
    }
}

fn timestamp(
    value: &Value,
    field: &str,
    nullable: bool,
    missing: &mut Vec<String>,
) -> Option<String> {
    let result = optional_string(value, field, nullable, missing);
    if result
        .as_ref()
        .is_some_and(|value| value.parse::<jiff::Timestamp>().is_err())
    {
        missing.push(field.to_string());
    }
    result
}

fn optional_bool(value: &Value, field: &str, missing: &mut Vec<String>) -> Option<bool> {
    let result = value.get(field).and_then(Value::as_bool);
    if result.is_none() {
        missing.push(field.to_string());
    }
    result
}

fn optional_u64(value: &Value, field: &str, missing: &mut Vec<String>) -> Option<u64> {
    let result = value.get(field).and_then(Value::as_u64);
    if result.is_none() {
        missing.push(field.to_string());
    }
    result
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Relation {
    Labels,
    References,
}

impl Relation {
    pub fn field(self) -> &'static str {
        match self {
            Self::Labels => "labels",
            Self::References => "closingIssuesReferences",
        }
    }
}

pub(super) struct Connection {
    pub values: Option<Vec<Value>>,
    pub coverage: Coverage,
}

impl Connection {
    pub fn parse(value: Option<&Value>, relation: Relation, after: Option<&str>) -> Self {
        let mut result = Self {
            values: None,
            coverage: Coverage {
                start_cursor: after.map(str::to_string),
                end_cursor: None,
                next_cursor: None,
                has_next_page: None,
                total_count: None,
                observed_count: 0,
                pages_read: 0,
                complete: false,
                remaining_complete: false,
                covered_from_start: after.is_none(),
                failure: Some("provider_connection_invalid"),
            },
        };
        let Some(value) = value else {
            return result;
        };
        let Some(nodes) = value.get("nodes").and_then(Value::as_array) else {
            return result;
        };
        let Some(count) = value.get("totalCount").and_then(Value::as_u64) else {
            return result;
        };
        let Ok((has_next, end_cursor)) = super::graphql::page_info(value) else {
            return result;
        };
        let entries = nodes
            .iter()
            .map(|node| match relation {
                Relation::Labels => required_string(node, "name").map(Value::String),
                Relation::References => {
                    let number = node
                        .get("number")
                        .and_then(Value::as_u64)
                        .filter(|number| *number > 0)
                        .ok_or("provider_connection_invalid")?;
                    let repo = node
                        .pointer("/repository/nameWithOwner")
                        .and_then(Value::as_str)
                        .ok_or("provider_connection_invalid")?;
                    if super::super::split_slug(repo).is_none() {
                        return Err("provider_connection_invalid");
                    }
                    let url = required_string(node, "url")?;
                    Ok(json!({"repo": repo, "kind": "issue", "number": number, "url": url}))
                }
            })
            .collect::<Result<Vec<_>, &'static str>>();
        let Ok(entries) = entries else {
            return result;
        };
        if has_next && (nodes.is_empty() || end_cursor.as_deref() == after) {
            return result;
        }
        result.values = Some(entries);
        result.coverage.end_cursor = end_cursor.clone();
        result.coverage.next_cursor = if has_next { end_cursor } else { None };
        result.coverage.has_next_page = Some(has_next);
        result.coverage.total_count = Some(count);
        result.coverage.observed_count = nodes.len();
        result.coverage.pages_read = 1;
        result.coverage.failure = None;
        result.finish();
        result
    }

    pub fn not_applicable() -> Self {
        Self {
            values: None,
            coverage: Coverage {
                start_cursor: None,
                end_cursor: None,
                next_cursor: None,
                has_next_page: Some(false),
                total_count: Some(0),
                observed_count: 0,
                pages_read: 0,
                complete: true,
                remaining_complete: true,
                covered_from_start: true,
                failure: None,
            },
        }
    }

    pub fn append(&mut self, page: Self) {
        if page.coverage.failure.is_some() || page.coverage.total_count != self.coverage.total_count
        {
            self.coverage.failure = Some("provider_connection_invalid");
            self.coverage.complete = false;
            self.coverage.remaining_complete = false;
            return;
        }
        self.values
            .as_mut()
            .expect("valid first connection page")
            .extend(page.values.expect("valid connection page"));
        self.coverage.observed_count += page.coverage.observed_count;
        self.coverage.pages_read += 1;
        self.coverage.end_cursor = page.coverage.end_cursor;
        self.coverage.next_cursor = page.coverage.next_cursor;
        self.coverage.has_next_page = page.coverage.has_next_page;
        self.finish();
    }

    pub fn finish(&mut self) {
        self.coverage.remaining_complete = self.coverage.failure.is_none()
            && self.coverage.has_next_page == Some(false)
            && (!self.coverage.covered_from_start
                || self.coverage.total_count == Some(self.coverage.observed_count as u64));
        self.coverage.complete =
            self.coverage.covered_from_start && self.coverage.remaining_complete;
        if self.coverage.failure.is_none()
            && self.coverage.has_next_page == Some(false)
            && !self.coverage.remaining_complete
        {
            self.coverage.failure = Some("provider_connection_count_mismatch");
        }
    }

    pub fn deduplicate(&mut self) {
        if let Some(values) = self.values.as_mut() {
            let mut seen = HashSet::new();
            values.retain(|value| {
                let key = if let Some(label) = value.as_str() {
                    label.to_string()
                } else {
                    format!(
                        "{}#{}",
                        value["repo"]
                            .as_str()
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                        value["number"]
                    )
                };
                seen.insert(key)
            });
        }
    }
}
