use std::cell::RefCell;
use std::collections::VecDeque;

use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;
use crate::backend::{BackendCall, BackendSuccess};
use crate::provider::{DetectionSource, Provider};

struct Fixture {
    responses: RefCell<VecDeque<Value>>,
    calls: RefCell<Vec<Vec<String>>>,
}

impl Fixture {
    fn new(responses: Vec<Value>) -> Self {
        Self {
            responses: RefCell::new(responses.into()),
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl BackendRunner for Fixture {
    fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
        self.calls.borrow_mut().push(
            call.argv
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
        );
        let response = self
            .responses
            .borrow_mut()
            .pop_front()
            .expect("unexpected backend call");
        if response.get("backend_failure") == Some(&Value::Bool(true)) {
            return Err(ForgeError::BackendError {
                schema_version: super::super::schema_err(),
                message: "fixture page refused".into(),
                detail: None,
            });
        }
        Ok(BackendSuccess {
            stdout: response.to_string(),
            stderr: String::new(),
        })
    }
}

fn ctx() -> ProviderContext {
    ProviderContext {
        provider: Provider::GitHub,
        host: "github.com".into(),
        source: DetectionSource::Flag,
        repo: None,
    }
}

fn args(kind: CompletionKind) -> CompletionQueryArgs {
    CompletionQueryArgs {
        query: "repo:acme/widget".into(),
        kind,
        since: "2030-01-01T00:00:00Z".into(),
        until: "2030-01-02T00:00:00Z".into(),
        page_size: 2,
        max_pages: 10,
        max_connection_pages: 10,
        cursor: None,
        exclude_forks: false,
        exclude_archived: false,
    }
}

fn connection(nodes: Vec<Value>, count: usize, next: Option<&str>) -> Value {
    json!({"nodes": nodes, "totalCount": count, "pageInfo": {"hasNextPage": next.is_some(), "endCursor": next}})
}

fn issue(number: u64, completed: &str) -> Value {
    json!({"__typename":"Issue", "id":format!("I_{number}"), "number":number,
        "url":format!("https://github.com/acme/widget/issues/{number}"), "title":"A neutral issue",
        "state":"CLOSED", "stateReason":"COMPLETED", "createdAt":"2029-12-01T00:00:00Z",
        "closedAt":completed, "updatedAt":"2031-02-01T00:00:00Z",
        "repository":{"nameWithOwner":"acme/widget","isFork":false,"isArchived":false},
        "author":{"__typename":"User","login":"contributor"},
        "labels":connection(vec![json!({"name":"raw label"})], 1, None)})
}

fn pr(number: u64) -> Value {
    let mut value = issue(number, "2030-01-01T11:00:00Z");
    value["__typename"] = json!("PullRequest");
    value["id"] = json!(format!("P_{number}"));
    value["url"] = json!(format!("https://github.com/acme/widget/pull/{number}"));
    value["state"] = json!("MERGED");
    value["mergedAt"] = json!("2030-01-01T10:00:00Z");
    value["additions"] = json!(17);
    value["deletions"] = json!(3);
    value["changedFiles"] = json!(2);
    value["closingIssuesReferences"] = connection(Vec::new(), 0, None);
    value
}

fn page(nodes: Vec<Value>, count: u32, next: Option<&str>) -> Value {
    json!({"data":{"search":{"nodes":nodes,"issueCount":count,
        "pageInfo":{"hasNextPage":next.is_some(),"endCursor":next}}}})
}

fn node(value: Value) -> Value {
    json!({"data":{"node":value}})
}

fn read_fixture(fixture: &Fixture, args: CompletionQueryArgs) -> Value {
    let context = ctx();
    let window = Window::new(&args).unwrap();
    let query = window.query(&args, "").unwrap();
    let progress = Cursor::read(
        args.cursor.as_deref(),
        cursor::scope(&context, &query, &args),
    )
    .unwrap();
    let result = read(fixture, &context, "", args, window, query, progress);
    assert!(
        fixture.responses.borrow().is_empty(),
        "unconsumed provider responses"
    );
    serde_json::to_value(result).unwrap()
}

#[test]
fn raw_qualifiers_and_utc_window_reach_the_selected_graphql_boundary() {
    let mut args = args(CompletionKind::Issue);
    args.query = "repo:acme/widget label:\"needs review\" state:closed".into();
    args.since = "2030-01-01T01:00:00+01:00".into();
    let fixture = Fixture::new(vec![page(Vec::new(), 0, None)]);
    let output = read_fixture(&fixture, args);
    assert_eq!(output["pagination"]["complete"], true);
    let calls = fixture.calls.borrow();
    assert!(calls[0].contains(&"searchQuery=repo:acme/widget label:\"needs review\" state:closed is:issue is:closed closed:2030-01-01T00:00:00Z..2030-01-02T00:00:00Z".into()));
    assert_eq!(calls[0][0..2], ["api", "graphql"]);
    assert_eq!(output["window"]["since"], "2030-01-01T00:00:00Z");
}

#[test]
fn half_open_window_uses_completion_instead_of_update_time() {
    let mut args = args(CompletionKind::Issue);
    args.page_size = 3;
    let mut before = issue(1, "2029-12-31T23:59:59Z");
    before["updatedAt"] = json!("2030-01-01T12:00:00Z");
    let fixture = Fixture::new(vec![page(
        vec![
            before,
            issue(2, "2030-01-01T00:00:00Z"),
            issue(3, "2030-01-02T00:00:00Z"),
        ],
        3,
        None,
    )]);
    let output = read_fixture(&fixture, args);
    assert_eq!(output["item_count"], 1);
    assert_eq!(output["items"][0]["number"], 2);
    assert_eq!(output["items"][0]["updated_at"], "2031-02-01T00:00:00Z");
    assert_eq!(output["pagination"]["complete"], true);
}

#[test]
fn subsecond_windows_fetch_a_superset_then_apply_exact_bounds() {
    let mut args = args(CompletionKind::Issue);
    args.since = "2030-01-01T00:00:00.5Z".into();
    args.until = "2030-01-01T00:00:01.5Z".into();
    let fixture = Fixture::new(vec![page(
        vec![
            issue(1, "2030-01-01T00:00:00.4Z"),
            issue(2, "2030-01-01T00:00:01Z"),
        ],
        2,
        None,
    )]);
    let output = read_fixture(&fixture, args);
    assert_eq!(output["item_count"], 1);
    assert!(
        output["effective_query"]
            .as_str()
            .unwrap()
            .contains("closed:2030-01-01T00:00:00Z..2030-01-01T00:00:02Z")
    );
}

#[test]
fn nested_label_and_cross_repository_reference_pages_are_drained() {
    let mut first = pr(4);
    first["labels"] = connection(vec![json!({"name":"first"})], 2, Some("L1"));
    let reference = |number| json!({"number":number,"url":format!("https://github.com/acme/library/issues/{number}"),"repository":{"nameWithOwner":"acme/library"}});
    first["closingIssuesReferences"] = connection(vec![reference(7)], 3, Some("R1"));
    let mut labels = first.clone();
    labels["labels"] = connection(vec![json!({"name":"second"})], 2, None);
    let mut references = first.clone();
    references["closingIssuesReferences"] = connection(vec![reference(7), reference(8)], 3, None);
    let fixture = Fixture::new(vec![
        page(vec![first], 1, None),
        node(labels),
        node(references),
    ]);
    let output = read_fixture(&fixture, args(CompletionKind::Pr));
    let item = &output["items"][0];
    assert_eq!(item["completed_at"], "2030-01-01T10:00:00Z");
    assert_eq!(item["closed_at"], "2030-01-01T11:00:00Z");
    assert_eq!(item["additions"], 17);
    assert_eq!(item["deletions"], 3);
    assert_eq!(item["changed_files"], 2);
    assert_eq!(item["labels"], json!(["first", "second"]));
    assert_eq!(
        item["closing_issues_references"].as_array().unwrap().len(),
        2
    );
    assert_eq!(item["closing_issues_references"][0]["repo"], "acme/library");
    assert_eq!(item["closing_issues_references"][0]["kind"], "issue");
    assert_eq!(item["closing_references_pagination"]["observed_count"], 3);
    assert_eq!(item["closing_references_pagination"]["complete"], true);
    assert_eq!(output["pagination"]["complete"], true);
    let calls = fixture.calls.borrow();
    assert!(calls[1].contains(&"labelsAfter=L1".into()));
    assert!(calls[2].contains(&"referencesAfter=R1".into()));
}

#[test]
fn bot_and_deleted_authors_and_raw_state_reason_are_preserved() {
    let mut bot = issue(1, "2030-01-01T12:00:00Z");
    bot["author"] = json!({"__typename":"Bot","login":"automation[bot]"});
    let mut deleted = issue(2, "2030-01-01T12:00:00Z");
    deleted["author"] = Value::Null;
    deleted["stateReason"] = json!("NOT_PLANNED");
    let output = read_fixture(
        &Fixture::new(vec![page(vec![bot, deleted], 2, None)]),
        args(CompletionKind::Issue),
    );
    assert_eq!(output["items"][0]["author"]["kind"], "Bot");
    assert_eq!(output["items"][1]["author"], Value::Null);
    assert_eq!(output["items"][1]["state_reason"], "NOT_PLANNED");
    assert_eq!(output["pagination"]["complete"], true);
}

#[test]
fn fork_and_archived_filters_use_canonical_repository_facts() {
    let mut args = args(CompletionKind::Issue);
    args.page_size = 3;
    args.exclude_forks = true;
    args.exclude_archived = true;
    let mut fork = issue(1, "2030-01-01T12:00:00Z");
    fork["repository"]["isFork"] = json!(true);
    let mut archived = issue(2, "2030-01-01T12:00:00Z");
    archived["repository"]["isArchived"] = json!(true);
    let output = read_fixture(
        &Fixture::new(vec![page(
            vec![fork, archived, issue(3, "2030-01-01T12:00:00Z")],
            3,
            None,
        )]),
        args,
    );
    assert_eq!(output["item_count"], 1);
    assert_eq!(output["items"][0]["number"], 3);
    assert_eq!(output["items"][0]["repository"]["is_fork"], false);
}

#[test]
fn multiple_search_pages_deduplicate_canonical_entities() {
    let first = issue(1, "2030-01-01T12:00:00Z");
    let fixture = Fixture::new(vec![
        page(vec![first.clone(), first], 3, Some("S1")),
        page(vec![issue(2, "2030-01-01T12:00:00Z")], 3, None),
    ]);
    let output = read_fixture(&fixture, args(CompletionKind::Issue));
    assert_eq!(output["item_count"], 2);
    assert_eq!(output["pagination"]["pages_read"], 2);
    assert_eq!(output["pagination"]["scanned"], 3);
    assert_eq!(output["pagination"]["complete"], true);
}

#[test]
fn bounded_search_pages_resume_without_restarting_the_query() {
    let mut args = args(CompletionKind::Issue);
    args.max_pages = 1;
    let first = read_fixture(
        &Fixture::new(vec![page(
            vec![
                issue(1, "2030-01-01T12:00:00Z"),
                issue(2, "2030-01-01T12:00:00Z"),
            ],
            3,
            Some("S1"),
        )]),
        args.clone(),
    );
    assert_eq!(first["pagination"]["status"], "truncated");
    args.cursor = Some(
        first["pagination"]["next_cursor"]
            .as_str()
            .unwrap()
            .to_string(),
    );
    let fixture = Fixture::new(vec![page(vec![issue(3, "2030-01-01T12:00:00Z")], 3, None)]);
    let resumed = read_fixture(&fixture, args);
    assert_eq!(resumed["pagination"]["complete"], false);
    assert_eq!(resumed["pagination"]["status"], "partial");
    assert_eq!(resumed["pagination"]["traversal_complete"], true);
    assert_eq!(resumed["pagination"]["covered_from_start"], false);
    assert_eq!(resumed["pagination"]["resumed"], true);
    assert_eq!(resumed["pagination"]["scanned"], 3);
    assert_eq!(resumed["item_count"], 1);
    assert!(fixture.calls.borrow()[0].contains(&"searchAfter=S1".into()));
}

#[test]
fn nested_cursor_resume_is_a_fragment_not_whole_connection_completeness() {
    let mut args = args(CompletionKind::Issue);
    args.max_connection_pages = 1;
    let mut first = issue(1, "2030-01-01T12:00:00Z");
    first["labels"] = connection(
        vec![json!({"name":"first"}), json!({"name":"second"})],
        3,
        Some("L1"),
    );
    let output = read_fixture(
        &Fixture::new(vec![page(vec![first.clone()], 1, None)]),
        args.clone(),
    );
    assert_eq!(output["pagination"]["status"], "truncated");
    assert_eq!(output["items"][0]["labels_pagination"]["complete"], false);
    args.cursor = Some(
        output["pagination"]["next_cursor"]
            .as_str()
            .unwrap()
            .to_string(),
    );
    first["labels"] = connection(vec![json!({"name":"third"})], 3, None);
    let output = read_fixture(&Fixture::new(vec![node(first)]), args);
    assert_eq!(output["items"][0]["labels"], json!(["third"]));
    assert_eq!(
        output["items"][0]["labels_pagination"]["start_cursor"],
        "L1"
    );
    assert_eq!(output["items"][0]["labels_pagination"]["complete"], false);
    assert_eq!(
        output["items"][0]["labels_pagination"]["remaining_complete"],
        true
    );
    assert_eq!(output["pagination"]["status"], "partial");
    assert_eq!(output["pagination"]["traversal_complete"], true);
    assert_eq!(output["pagination"]["next_cursor"], Value::Null);
}

#[test]
fn search_ceiling_is_explicit_even_after_all_accessible_rows() {
    let args = args(CompletionKind::Issue);
    let window = Window::new(&args).unwrap();
    let query = window.query(&args, "").unwrap();
    let mut progress = Cursor::new(cursor::scope(&ctx(), &query, &args));
    progress.after = Some("S998".into());
    progress.scanned = 998;
    progress.total_count = Some(1001);
    let fixture = Fixture::new(vec![page(
        vec![
            issue(999, "2030-01-01T12:00:00Z"),
            issue(1000, "2030-01-01T12:00:00Z"),
        ],
        1001,
        Some("S1000"),
    )]);
    let output =
        serde_json::to_value(read(&fixture, &ctx(), "", args, window, query, progress)).unwrap();
    assert_eq!(output["pagination"]["status"], "truncated");
    assert_eq!(output["pagination"]["search_ceiling"], true);
    assert_eq!(output["pagination"]["complete"], false);
    assert_eq!(output["pagination"]["next_cursor"], Value::Null);
}

#[test]
fn failed_nested_page_preserves_observed_labels_and_resume_position() {
    let mut item = issue(1, "2030-01-01T12:00:00Z");
    item["labels"] = connection(vec![json!({"name":"observed"})], 2, Some("L1"));
    let output = read_fixture(
        &Fixture::new(vec![
            page(vec![item], 1, None),
            json!({"backend_failure":true}),
        ]),
        args(CompletionKind::Issue),
    );
    assert_eq!(output["pagination"]["status"], "partial");
    assert_eq!(output["items"][0]["labels"], json!(["observed"]));
    assert_eq!(output["items"][0]["labels_pagination"]["complete"], false);
    assert_eq!(output["pagination"]["failures"][0]["kind"], "backend_error");
    assert!(output["pagination"]["next_cursor"].is_string());
}

#[test]
fn failed_search_page_keeps_prior_items_and_never_claims_complete_empty_data() {
    let output = read_fixture(
        &Fixture::new(vec![
            page(vec![issue(1, "2030-01-01T12:00:00Z")], 2, Some("S1")),
            json!({"backend_failure":true}),
        ]),
        args(CompletionKind::Issue),
    );
    assert_eq!(output["item_count"], 1);
    assert_eq!(output["pagination"]["status"], "partial");
    assert_eq!(output["pagination"]["complete"], false);
    assert!(output["pagination"]["next_cursor"].is_string());
    let empty = read_fixture(
        &Fixture::new(vec![json!({"backend_failure":true})]),
        args(CompletionKind::Issue),
    );
    assert_eq!(empty["item_count"], 0);
    assert_eq!(empty["pagination"]["complete"], false);
}

#[test]
fn graphql_errors_and_missing_search_page_fields_are_not_empty_successes() {
    for response in [
        json!({"errors":[{"message":"fixture refusal"}],"data":{"search":{"nodes":[]}}}),
        json!({"data":{"search":{"nodes":[],"issueCount":0}}}),
        json!({"data":{"search":{"nodes":null,"issueCount":0,"pageInfo":{"hasNextPage":false,"endCursor":null}}}}),
    ] {
        let output = read_fixture(&Fixture::new(vec![response]), args(CompletionKind::Issue));
        assert_eq!(output["pagination"]["complete"], false);
        assert_eq!(output["pagination"]["status"], "partial");
    }
}

#[test]
fn missing_author_statistics_or_connections_remain_explicit_unknowns() {
    let mut value = pr(1);
    value.as_object_mut().unwrap().remove("author");
    value.as_object_mut().unwrap().remove("additions");
    value["labels"] = Value::Null;
    let output = read_fixture(
        &Fixture::new(vec![page(vec![value], 1, None)]),
        args(CompletionKind::Pr),
    );
    assert_eq!(output["items"][0]["labels"], Value::Null);
    assert_eq!(output["items"][0]["additions"], Value::Null);
    assert_eq!(
        output["items"][0]["missing_fields"],
        json!(["author", "additions"])
    );
    assert_eq!(output["pagination"]["status"], "partial");
    assert_eq!(output["pagination"]["complete"], false);
}

#[test]
fn repeated_cursors_and_incomplete_counts_refuse_completeness() {
    for responses in [
        vec![
            page(vec![issue(1, "2030-01-01T12:00:00Z")], 3, Some("S1")),
            page(vec![issue(2, "2030-01-01T12:00:00Z")], 3, Some("S1")),
        ],
        vec![page(Vec::new(), 1, None)],
    ] {
        let output = read_fixture(&Fixture::new(responses), args(CompletionKind::Issue));
        assert_eq!(output["pagination"]["status"], "partial");
        assert_eq!(output["pagination"]["complete"], false);
    }
}

#[test]
fn changing_search_counts_remain_partial_even_when_traversal_ends() {
    let output = read_fixture(
        &Fixture::new(vec![
            page(vec![issue(1, "2030-01-01T12:00:00Z")], 3, Some("S1")),
            page(vec![issue(2, "2030-01-01T12:00:00Z")], 2, None),
        ]),
        args(CompletionKind::Issue),
    );
    assert_eq!(output["item_count"], 2);
    assert_eq!(output["pagination"]["status"], "partial");
    assert_eq!(output["pagination"]["traversal_complete"], false);
    assert_eq!(
        output["pagination"]["failures"][0]["kind"],
        "provider_search_count_mismatch"
    );
}

#[test]
fn continuation_scope_binds_query_window_filters_and_authority() {
    let mut args = args(CompletionKind::Issue);
    let query = Window::new(&args).unwrap().query(&args, "").unwrap();
    let token = Cursor::new(cursor::scope(&ctx(), &query, &args)).encode();
    assert!(Cursor::read(Some(&token), cursor::scope(&ctx(), &query, &args)).is_ok());
    args.exclude_forks = true;
    assert_eq!(
        Cursor::read(Some(&token), cursor::scope(&ctx(), &query, &args))
            .unwrap_err()
            .kind(),
        "completion_cursor_invalid"
    );
    args.exclude_forks = false;
    args.until = "2030-01-03T00:00:00Z".into();
    assert!(Cursor::read(Some(&token), cursor::scope(&ctx(), &query, &args)).is_err());
    let mut other = ctx();
    other.host = "forge.example".into();
    assert!(Cursor::read(Some(&token), cursor::scope(&other, &query, &args)).is_err());
}

#[test]
fn invalid_window_is_rejected_before_provider_work() {
    let mut args = args(CompletionKind::Issue);
    args.until = args.since.clone();
    assert_eq!(
        Window::new(&args).err().unwrap().kind(),
        "completion_window_invalid"
    );
    args.since = "not-a-timestamp".into();
    assert!(Window::new(&args).is_err());
}
