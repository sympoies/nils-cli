use std::path::{Path, PathBuf};

use anyhow::Context;
use base64::Engine;

use crate::rest::schema::{RestMultipartPart, RestRequestFile};
use crate::Result;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestExecutedRequest {
    pub method: String,
    pub url: String,
    pub response: RestHttpResponse,
}

fn resolve_part_file_path(request_file: &Path, raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    let base_dir = request_file.parent().unwrap_or_else(|| Path::new("."));
    Ok(base_dir.join(path))
}

fn build_multipart_form(
    request_file: &Path,
    parts: &[RestMultipartPart],
) -> Result<Option<reqwest::blocking::multipart::Form>> {
    if parts.is_empty() {
        return Ok(None);
    }

    let mut form = reqwest::blocking::multipart::Form::new();
    let mut added_parts = 0usize;

    for part in parts {
        let name = part.name.trim();
        if name.is_empty() {
            anyhow::bail!("Multipart part is missing required field: name");
        }

        if let Some(value) = &part.value {
            let value = value.trim();
            if !value.is_empty() {
                form = form.text(name.to_string(), value.to_string());
                added_parts += 1;
                continue;
            }
        }

        if let Some(payload) = &part.base64 {
            let payload = payload.trim();
            if !payload.is_empty() {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(payload)
                    .context("failed to decode multipart base64 payload")?;
                let mut p = reqwest::blocking::multipart::Part::bytes(bytes);
                let filename = part
                    .filename
                    .clone()
                    .unwrap_or_else(|| "rest.multipart.bin".to_string());
                p = p.file_name(filename);
                if let Some(ct) = &part.content_type {
                    p = p
                        .mime_str(ct)
                        .with_context(|| format!("invalid multipart contentType: {ct}"))?;
                }
                form = form.part(name.to_string(), p);
                added_parts += 1;
                continue;
            }
        }

        let Some(file_path_raw) = part.file_path.as_deref() else {
            anyhow::bail!("Multipart part '{name}' must include value, filePath, or base64.");
        };

        let file_path = resolve_part_file_path(request_file, file_path_raw)?;
        if !file_path.is_file() {
            anyhow::bail!(
                "Multipart part '{name}' file not found: {}",
                file_path.display()
            );
        }

        let mut p = reqwest::blocking::multipart::Part::file(&file_path)
            .with_context(|| format!("failed to open multipart file: {}", file_path.display()))?;

        let filename = part.filename.clone().unwrap_or_else(|| {
            file_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("file")
                .to_string()
        });
        p = p.file_name(filename);

        if let Some(ct) = &part.content_type {
            p = p
                .mime_str(ct)
                .with_context(|| format!("invalid multipart contentType: {ct}"))?;
        }

        form = form.part(name.to_string(), p);
        added_parts += 1;
    }

    if added_parts == 0 {
        Ok(None)
    } else {
        Ok(Some(form))
    }
}

pub fn execute_rest_request(
    request_file: &RestRequestFile,
    base_url: &str,
    bearer_token: Option<&str>,
) -> Result<RestExecutedRequest> {
    let req = &request_file.request;

    let base = base_url.trim_end_matches('/');
    let mut url = format!("{base}{}", req.path);
    let query_string = req.query_string();
    if !query_string.is_empty() {
        url.push('?');
        url.push_str(&query_string);
    }

    let method = reqwest::Method::from_bytes(req.method.as_bytes())
        .with_context(|| format!("invalid HTTP method: {}", req.method))?;

    let mut headers = reqwest::header::HeaderMap::new();
    if !req.headers.accept_key_present {
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    }
    if req.body.is_some() {
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    }
    if let Some(token) = bearer_token {
        let value = format!("Bearer {token}");
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&value)
                .context("invalid Authorization header value")?,
        );
    }

    for (k, v) in &req.headers.user_headers {
        let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
            .with_context(|| format!("invalid header name: {k}"))?;
        let value = reqwest::header::HeaderValue::from_str(v)
            .with_context(|| format!("invalid header value: {k}"))?;
        headers.append(name, value);
    }

    let client = reqwest::blocking::Client::new();
    let mut builder = client.request(method, &url).headers(headers);

    if let Some(body) = &req.body {
        let bytes = serde_json::to_vec(body).context("failed to serialize request body as JSON")?;
        builder = builder.body(bytes);
    } else if let Some(parts) = &req.multipart {
        let form = build_multipart_form(&request_file.path, parts)?;
        if let Some(form) = form {
            builder = builder.multipart(form);
        }
    }

    let response = builder
        .send()
        .with_context(|| format!("HTTP request failed: {} {}", req.method, url))?;

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

    Ok(RestExecutedRequest {
        method: req.method.clone(),
        url,
        response: RestHttpResponse {
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
    fn rest_runner_url_construction_includes_sorted_query() {
        let request_file = RestRequestFile {
            path: PathBuf::from("/tmp/req.request.json"),
            request: crate::rest::schema::parse_rest_request_json(serde_json::json!({
                "method": "GET",
                "path": "/health",
                "query": { "b": 1, "a": true }
            }))
            .unwrap(),
        };

        // Not actually sending a request here; just validate the derived URL logic via build_multipart_form
        // by calling execute_rest_request up to the point that constructs the URL would be awkward. Keep this
        // as a lightweight unit check by asserting the computed URL through the public helper path.
        let base = "http://localhost:6700/";
        let req = &request_file.request;
        let base = base.trim_end_matches('/');
        let mut url = format!("{base}{}", req.path);
        let qs = req.query_string();
        if !qs.is_empty() {
            url.push('?');
            url.push_str(&qs);
        }
        assert_eq!(url, "http://localhost:6700/health?a=true&b=1");
    }
}
