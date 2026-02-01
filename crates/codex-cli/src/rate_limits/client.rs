use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::json;
use crate::paths;

pub struct UsageResponse {
    pub body: String,
    pub json: Value,
}

pub struct UsageRequest {
    pub target_file: PathBuf,
    pub refresh_on_401: bool,
    pub base_url: String,
    pub connect_timeout_seconds: u64,
    pub max_time_seconds: u64,
}

pub fn fetch_usage(request: &UsageRequest) -> Result<UsageResponse> {
    let (access_token, account_id) = read_tokens(&request.target_file)?;
    let mut response = send_request(request, &access_token, account_id.as_deref())?;

    if response.status == 401 && request.refresh_on_401 {
        refresh_target(&request.target_file);
        if let Ok((access_token, account_id)) = read_tokens(&request.target_file) {
            response = send_request(request, &access_token, account_id.as_deref())?;
        }
    }

    if response.status != 200 {
        let preview = response
            .body
            .chars()
            .take(200)
            .collect::<String>()
            .replace(['\n', '\r'], " ");
        if preview.is_empty() {
            anyhow::bail!(
                "codex-rate-limits: GET {} failed (HTTP {})",
                response.url,
                response.status
            );
        }
        anyhow::bail!(
            "codex-rate-limits: GET {} failed (HTTP {})\ncodex-rate-limits: body: {}",
            response.url,
            response.status,
            preview
        );
    }

    let json: Value =
        serde_json::from_str(&response.body).context("invalid JSON from usage endpoint")?;

    Ok(UsageResponse {
        body: response.body,
        json,
    })
}

pub fn read_tokens(target_file: &Path) -> Result<(String, Option<String>)> {
    let value = json::read_json(target_file)?;
    let access_token = json::string_at(&value, &["tokens", "access_token"])
        .ok_or_else(|| anyhow::anyhow!("missing access_token"))?;
    let account_id = json::string_at(&value, &["tokens", "account_id"])
        .or_else(|| json::string_at(&value, &["account_id"]));
    Ok((access_token, account_id))
}

fn send_request(
    request: &UsageRequest,
    access_token: &str,
    account_id: Option<&str>,
) -> Result<HttpResponse> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(request.connect_timeout_seconds))
        .timeout(Duration::from_secs(request.max_time_seconds))
        .build()?;

    let url = format!("{}/wham/usage", request.base_url.trim_end_matches('/'));
    let mut req = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .header("User-Agent", "codex-cli");
    if let Some(account_id) = account_id {
        req = req.header("ChatGPT-Account-Id", account_id);
    }

    let resp = req.send();
    let resp = match resp {
        Ok(value) => value,
        Err(_) => {
            anyhow::bail!("codex-rate-limits: request failed: {}", url);
        }
    };

    let status = resp.status().as_u16();
    let body = resp.text().unwrap_or_default();
    Ok(HttpResponse { status, body, url })
}

fn refresh_target(target_file: &Path) {
    if let Some(auth_file) = paths::resolve_auth_file() {
        if auth_file == target_file {
            let _ = crate::auth::refresh::run(&[]);
            return;
        }
    }

    if let Some(secret_dir) = paths::resolve_secret_dir() {
        if let Some(file_name) = target_file.file_name().and_then(|n| n.to_str()) {
            let path = secret_dir.join(file_name);
            if path == target_file {
                let _ = crate::auth::refresh::run(&[file_name.to_string()]);
            }
        }
    }
}

struct HttpResponse {
    status: u16,
    body: String,
    url: String,
}
