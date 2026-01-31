use anyhow::Context;

use crate::rest::schema::RestCleanup;
use crate::Result;

pub fn render_cleanup_path(cleanup: &RestCleanup, main_response_body: &[u8]) -> Result<String> {
    let response_json: Option<serde_json::Value> = serde_json::from_slice(main_response_body).ok();

    let mut path = cleanup.path_template.clone();
    for (key, expr) in &cleanup.vars {
        let lines = response_json
            .as_ref()
            .and_then(|json| crate::jq::query_raw(json, expr).ok())
            .unwrap_or_default();
        let value = lines.first().cloned().unwrap_or_default();
        let value = value.trim().to_string();
        if value.is_empty() || value == "null" {
            anyhow::bail!("cleanup var '{key}' is empty");
        }
        path = path.replace(&format!("{{{{{key}}}}}"), &value);
    }

    if !path.starts_with('/') {
        anyhow::bail!("cleanup.pathTemplate must resolve to an absolute path (starts with /)");
    }

    Ok(path)
}

pub fn execute_cleanup(
    cleanup: &RestCleanup,
    base_url: &str,
    bearer_token: Option<&str>,
    main_response_body: &[u8],
) -> Result<()> {
    let cleanup_path = render_cleanup_path(cleanup, main_response_body)?;
    let base = base_url.trim_end_matches('/');
    let cleanup_url = format!("{base}{cleanup_path}");

    let method = reqwest::Method::from_bytes(cleanup.method.as_bytes())
        .with_context(|| format!("invalid cleanup HTTP method: {}", cleanup.method))?;

    let client = reqwest::blocking::Client::new();
    let mut builder = client.request(method, &cleanup_url);
    if let Some(token) = bearer_token {
        let value = format!("Bearer {token}");
        builder = builder.header(reqwest::header::AUTHORIZATION, value);
    }

    let response = builder
        .send()
        .with_context(|| format!("cleanup request failed: {} {}", cleanup.method, cleanup_url))?;

    let got = response.status().as_u16();
    let expected = cleanup.expect_status;
    if got != expected {
        anyhow::bail!(
            "cleanup failed: expected {expected} but got {got} ({} {cleanup_url})",
            cleanup.method
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn rest_cleanup_renders_path_and_substitutes_vars() {
        let cleanup = crate::rest::schema::parse_rest_request_json(serde_json::json!({
            "method": "GET",
            "path": "/x",
            "cleanup": {
                "pathTemplate": "/files/{{key}}",
                "vars": { "key": ".key" }
            }
        }))
        .unwrap()
        .cleanup
        .unwrap();

        let body = serde_json::to_vec(&serde_json::json!({"key": "abc"})).unwrap();
        let path = render_cleanup_path(&cleanup, &body).unwrap();
        assert_eq!(path, "/files/abc");
    }

    #[test]
    fn rest_cleanup_var_null_is_error() {
        let cleanup = crate::rest::schema::parse_rest_request_json(serde_json::json!({
            "method": "GET",
            "path": "/x",
            "cleanup": {
                "pathTemplate": "/files/{{key}}",
                "vars": { "key": ".missing" }
            }
        }))
        .unwrap()
        .cleanup
        .unwrap();

        let body = serde_json::to_vec(&serde_json::json!({"key": "abc"})).unwrap();
        let err = render_cleanup_path(&cleanup, &body).unwrap_err();
        assert!(err.to_string().contains("cleanup var 'key' is empty"));
    }

    #[test]
    fn rest_cleanup_requires_absolute_path() {
        let cleanup = crate::rest::schema::parse_rest_request_json(serde_json::json!({
            "method": "GET",
            "path": "/x",
            "cleanup": {
                "pathTemplate": "{{key}}",
                "vars": { "key": ".key" }
            }
        }))
        .unwrap()
        .cleanup
        .unwrap();

        let body = serde_json::to_vec(&serde_json::json!({"key": "abc"})).unwrap();
        let err = render_cleanup_path(&cleanup, &body).unwrap_err();
        assert!(err.to_string().contains("absolute path"));
    }
}
