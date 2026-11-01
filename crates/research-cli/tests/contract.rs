use nils_research_cli::{Engine, Operation, Provider, Request};
use pretty_assertions::assert_eq;

#[tokio::test]
async fn invalid_input_has_the_same_cli_http_engine_envelope() {
    let request = Request::new(Operation::ReadUrl, "", Provider::Openai);
    let envelope = Engine::default().execute(request).await;
    let value = serde_json::to_value(envelope).unwrap();
    assert_eq!(value["schema_version"], "cli.research-cli.read-url.v1");
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "invalid-request");
}

#[tokio::test]
async fn nonpublic_read_does_not_require_a_key_or_call_a_model() {
    let mut request = Request::new(Operation::ReadUrl, "", Provider::Anthropic);
    request.urls = vec!["http://127.0.0.1/private".into()];
    let value = serde_json::to_value(Engine::default().execute(request).await).unwrap();
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "non-public-url");
    assert_eq!(value["data"]["model"], serde_json::Value::Null);
}
