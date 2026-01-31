use anyhow::Context;

use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlExecutedRequest {
    pub url: String,
    pub response: GraphqlHttpResponse,
}

fn build_payload(operation: &str, variables: Option<&serde_json::Value>) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "query".to_string(),
        serde_json::Value::String(operation.to_string()),
    );
    if let Some(vars) = variables {
        obj.insert("variables".to_string(), vars.clone());
    }
    serde_json::Value::Object(obj)
}

pub fn execute_graphql_request(
    endpoint_url: &str,
    bearer_token: Option<&str>,
    operation: &str,
    variables: Option<&serde_json::Value>,
) -> Result<GraphqlExecutedRequest> {
    let payload = build_payload(operation, variables);
    let bytes =
        serde_json::to_vec(&payload).context("failed to serialize GraphQL request payload")?;

    let client = reqwest::blocking::Client::new();
    let mut builder = client
        .post(endpoint_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(bytes);

    if let Some(token) = bearer_token {
        builder = builder.header(reqwest::header::AUTHORIZATION, format!("Bearer {token}"));
    }

    let response = builder
        .send()
        .with_context(|| format!("HTTP request failed: POST {endpoint_url}"))?;

    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body = response
        .bytes()
        .context("failed to read response body")?
        .to_vec();

    if !(200..300).contains(&status) {
        anyhow::bail!("HTTP request failed with status {status}.");
    }

    Ok(GraphqlExecutedRequest {
        url: endpoint_url.to_string(),
        response: GraphqlHttpResponse {
            status,
            body,
            content_type,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphql_runner_build_payload_includes_vars_only_when_present() {
        let op = "query { ok }";
        let with_vars = build_payload(op, Some(&serde_json::json!({"a": 1})));
        assert!(with_vars.get("variables").is_some());
        let without_vars = build_payload(op, None);
        assert!(without_vars.get("variables").is_none());
    }
}
