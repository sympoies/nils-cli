use graphql_tools::parser::{parse_query, parse_schema};
use graphql_tools::validation::rules::default_rules_validation_plan;
use graphql_tools::validation::validate::validate;
use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;
use crate::provider::{DetectionSource, Provider};

const SCHEMA: &str = include_str!("completion_schema.graphql");

fn errors(query: &str) -> Vec<String> {
    let schema = parse_schema::<String>(SCHEMA).unwrap().into_static();
    let operation = parse_query::<String>(query).unwrap().into_static();
    validate(&schema, &operation, &default_rules_validation_plan())
        .iter()
        .map(|error| error.message.clone())
        .collect()
}

fn ctx() -> ProviderContext {
    ProviderContext {
        provider: Provider::GitHub,
        host: "github.com".into(),
        source: DetectionSource::Flag,
        repo: None,
    }
}

fn query(call: &crate::backend::BackendCall) -> String {
    call.argv
        .iter()
        .find_map(|arg| arg.to_str()?.strip_prefix("query=").map(str::to_owned))
        .expect("generated GraphQL document")
}

#[test]
fn generated_completion_search_document_is_schema_valid() {
    let query = query(&graphql::search_call(&ctx(), "repo:acme/widget", 2, None));
    assert_eq!(errors(&query), Vec::<String>::new());
}

#[test]
fn generated_completion_node_document_is_schema_valid() {
    let query = query(&graphql::node_call(&ctx(), "I_7", 2, None, None));
    assert_eq!(errors(&query), Vec::<String>::new());
}

#[test]
fn schema_control_rejects_shared_response_names_for_distinct_enum_types() {
    let errors =
        errors("{ node(id: \"I_7\") { ... on Issue { state } ... on PullRequest { state } } }");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("conflict"), "{errors:?}");
    assert!(
        errors[0].contains("IssueState") && errors[0].contains("PullRequestState"),
        "{errors:?}"
    );
}

#[test]
fn schema_control_accepts_distinct_enum_state_aliases() {
    assert_eq!(
        errors(
            "{ node(id: \"I_7\") { ... on Issue { issueState: state } ... on PullRequest { prState: state } } }"
        ),
        Vec::<String>::new()
    );
}

fn assert_canonical_state(kind: CompletionKind, type_name: &str, alias: &str, state: &str) {
    let mut value = json!({
        "__typename": type_name, "id": "NODE_7", "number": 7,
        "url": "https://github.com/acme/widget/issues/7", "title": "A neutral completion",
        "repository": {"nameWithOwner": "acme/widget"}
    });
    value[alias] = json!(state);
    let (item, _, _) = facts::item(&value, kind, None, None).expect("kind-specific wire state");
    assert_eq!(item.state, state);
    let output = serde_json::to_value(item).unwrap();
    assert_eq!(output["state"], state);
    assert!(output.get("issueState").is_none() && output.get("prState").is_none());
}

#[test]
fn issue_state_alias_normalizes_to_the_existing_canonical_field() {
    assert_canonical_state(CompletionKind::Issue, "Issue", "issueState", "CLOSED");
}

#[test]
fn pr_state_alias_normalizes_to_the_existing_canonical_field() {
    assert_canonical_state(CompletionKind::Pr, "PullRequest", "prState", "MERGED");
}
