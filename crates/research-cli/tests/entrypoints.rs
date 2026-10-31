mod support;
use clap::Parser;
use nils_research_cli::{cli::Cli, *};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn http(engine: Engine, path: &str, body: Value) -> (u16, Value) {
    let response = nils_research_cli::http::router(engine)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1_048_576)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
fn without_time(mut value: Value) -> Value {
    if let Some(data) = value.get_mut("data").and_then(Value::as_object_mut) {
        data.remove("retrieved_at");
    }
    value
}

#[tokio::test]
async fn cli_and_http_search_and_brief_share_the_engine_and_literal_schema() {
    for provider in ["openai", "anthropic"] {
        for operation in ["search", "brief"] {
            let cli = Cli::try_parse_from([
                "research-cli",
                operation,
                "example",
                "--provider",
                provider,
                "--language",
                "en",
                "--recency-days",
                "7",
                "--domain",
                "example.com",
                "--limit",
                "1",
            ])
            .unwrap();
            let request = cli.command.into_request().unwrap();
            assert_eq!(request.schema_version, "research.request.v1");
            let (engine, _) = support::engine(request.provider, "search");
            let expected = serde_json::to_value(engine.execute(request.clone()).await).unwrap();
            let (status, actual) = http(
                engine,
                &format!("/v1/{operation}"),
                serde_json::to_value(request).unwrap(),
            )
            .await;
            assert_eq!(status, 200);
            assert_eq!(
                actual["schema_version"],
                format!("cli.research-cli.{operation}.v1")
            );
            assert_eq!(without_time(actual.clone()), without_time(expected));
            let data = actual["data"].as_object().unwrap();
            for key in [
                "query",
                "provider",
                "model",
                "retrieved_at",
                "sources",
                "facts",
                "summary",
                "assessment",
                "unknowns",
                "conflicts",
                "usage",
                "warnings",
                "truncated",
            ] {
                assert!(data.contains_key(key), "{key}");
            }
            for key in ["id", "url", "final_url", "title", "published_at"] {
                assert!(data["sources"][0].get(key).is_some(), "{key}");
            }
            for key in [
                "search_calls",
                "input_tokens",
                "output_tokens",
                "cost_usd_estimate",
                "price_source",
                "raw",
            ] {
                assert!(data["usage"].get(key).is_some(), "{key}");
            }
        }
    }
}

#[tokio::test]
async fn every_operation_shares_structured_validation_errors_and_http_maps_provider_failures() {
    for (operation, path) in [
        (Operation::Search, "search"),
        (Operation::ReadUrl, "read-url"),
        (Operation::Brief, "brief"),
    ] {
        let request = Request::new(operation, "", Provider::Openai);
        let expected =
            serde_json::to_value(Engine::default().execute(request.clone()).await).unwrap();
        let (status, actual) = http(
            Engine::default(),
            &format!("/v1/{path}"),
            serde_json::to_value(request).unwrap(),
        )
        .await;
        assert_eq!(status, 400);
        assert_eq!(actual, expected);
    }
    for provider in [Provider::Openai, Provider::Anthropic] {
        let request = Request::new(Operation::Search, "example", provider);
        let (engine, _) = support::engine(provider, "429");
        let (status, actual) =
            http(engine, "/v1/search", serde_json::to_value(request).unwrap()).await;
        assert_eq!(status, 429);
        assert_eq!(actual["error"]["code"], "rate-limited");
        assert!(!actual.to_string().contains("Synthetic quota"));
    }
    for body in [
        json!({}),
        json!({"schema_version":"unknown","operation":"search","provider":"openai"}),
    ] {
        let (status, actual) = http(Engine::default(), "/v1/search", body).await;
        assert_eq!(status, 400);
        assert_eq!(actual["ok"], false);
    }
}

#[tokio::test]
async fn text_renderer_labels_model_inference_and_never_uses_it_as_a_source() {
    let (engine, _) = support::engine(Provider::Openai, "search");
    let envelope = engine
        .execute(Request::new(Operation::Search, "example", Provider::Openai))
        .await;
    let text = render_text(&envelope);
    assert!(text.contains("Facts\n- The domain is reserved"));
    assert!(text.contains("Assessment (model inference)\n- The cited source"));
    assert!(text.find("Facts\n").unwrap() < text.find("Assessment (model inference)\n").unwrap());
    assert_eq!(envelope.data.unwrap().sources.len(), 1);
}

#[test]
fn cli_process_exit_matrix_and_completion_are_stable_and_secret_free() {
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_research-cli"))
            .args(args)
            .env_remove("OPENAI_API_KEY")
            .env_remove("ANTHROPIC_API_KEY")
            .output()
            .unwrap()
    };
    for args in [vec!["--help"], vec!["--version"]] {
        let out = run(&args);
        assert!(out.status.success());
        if args == ["--help"] {
            assert!(String::from_utf8_lossy(&out.stdout).contains("--version"));
        }
    }
    for (args, exit, code) in [
        (vec!["--format", "json", "oops"], 64, "unknown-subcommand"),
        (
            vec!["--format=json", "search", "example", "--bad=secret-fixture"],
            64,
            "parse-error",
        ),
        (
            vec!["--format", "json", "read-url", "http://127.0.0.1/"],
            65,
            "non-public-url",
        ),
        (
            vec!["--format", "json", "search", "example"],
            69,
            "missing-key",
        ),
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(exit));
        let wire: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(wire["ok"], false);
        assert_eq!(wire["error"]["code"], code);
        assert!(!String::from_utf8_lossy(&output.stdout).contains("secret-fixture"));
    }
    for shell in ["bash", "zsh"] {
        let output = run(&["completion", shell]);
        assert!(output.status.success());
        let script = String::from_utf8(output.stdout).unwrap();
        for flag in ["--provider", "--format", "--max-bytes", "--version"] {
            assert!(script.contains(flag), "{flag}");
        }
    }
}

struct PlainPage;
impl nils_research_cli::fetch::UrlReader for PlainPage {
    fn read<'a>(&'a self, url: &'a str, _: &'a Limits) -> nils_research_cli::fetch::ReadFuture<'a> {
        Box::pin(async move {
            Ok(nils_research_cli::fetch::Page {
                url: url.into(),
                final_url: "https://example.com/final".into(),
                content: "Only bytes actually read.".into(),
                truncated: false,
            })
        })
    }
}
#[tokio::test]
async fn plain_read_cli_and_http_share_content_without_a_provider_request() {
    let cli =
        Cli::try_parse_from(["research-cli", "read-url", "https://example.com/article"]).unwrap();
    let request = cli.command.into_request().unwrap();
    let (_, transport) = support::engine(Provider::Openai, "search");
    let engine = Engine::new(transport.clone(), std::sync::Arc::new(PlainPage));
    let expected = serde_json::to_value(engine.execute(request.clone()).await).unwrap();
    let (status, actual) = http(
        engine,
        "/v1/read-url",
        serde_json::to_value(request).unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(actual["schema_version"], "cli.research-cli.read-url.v1");
    assert_eq!(without_time(actual.clone()), without_time(expected));
    assert_eq!(actual["data"]["model"], Value::Null);
    assert_eq!(actual["data"]["content"], "Only bytes actually read.");
    assert_eq!(
        actual["data"]["sources"][0]["final_url"],
        "https://example.com/final"
    );
    assert_eq!(actual["data"]["assessment"]["generated"], false);
    assert!(transport.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn malformed_oversized_and_wrong_method_http_requests_have_json_errors() {
    for body in ["{".to_owned(), "x".repeat(65_537)] {
        let response = nils_research_cli::http::router(Engine::default())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/search")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema_version"], "cli.research-cli.error.v1");
        assert_eq!(value["error"]["code"], "invalid-request");
    }
    let response = nils_research_cli::http::router(Engine::default())
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/search")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::METHOD_NOT_ALLOWED
    );
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["ok"], false);
}

#[test]
fn serve_bind_failure_uses_a_structured_cli_runtime_error() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_research-cli"))
        .args([
            "--format",
            "json",
            "serve",
            "--bind",
            &listener.local_addr().unwrap().to_string(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let wire: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(wire["schema_version"], "cli.research-cli.error.v1");
    assert_eq!(wire["error"]["code"], "bind-error");
}
