use std::path::{Path, PathBuf};
use std::process::Command;

use crate::json;

pub struct UsageResponse {
    pub body: String,
}

pub struct UsageRequest {
    pub target_file: PathBuf,
    pub refresh_on_401: bool,
    pub endpoint: String,
    pub api_version: String,
    pub project: String,
    pub connect_timeout_seconds: u64,
    pub max_time_seconds: u64,
}

pub fn fetch_usage(request: &UsageRequest) -> Result<UsageResponse, String> {
    let access_token = read_tokens(&request.target_file)?;
    let mut response = send_request(request, &access_token)?;

    if response.status == 401
        && request.refresh_on_401
        && let Ok(access_token) = read_tokens(&request.target_file)
    {
        response = send_request(request, &access_token)?;
    }

    if response.status != 200 {
        let preview = response
            .body
            .chars()
            .take(200)
            .collect::<String>()
            .replace(['\n', '\r'], " ");
        if preview.is_empty() {
            return Err(format!(
                "gemini-rate-limits: POST {} failed (HTTP {})",
                response.url, response.status
            ));
        }
        return Err(format!(
            "gemini-rate-limits: POST {} failed (HTTP {})\ngemini-rate-limits: body: {}",
            response.url, response.status, preview
        ));
    }

    Ok(UsageResponse {
        body: response.body,
    })
}

pub fn read_tokens(target_file: &Path) -> Result<String, String> {
    let value = json::read_json(target_file).map_err(|err| err.to_string())?;
    let access_token = json::string_at(&value, &["tokens", "access_token"])
        .or_else(|| json::string_at(&value, &["access_token"]))
        .ok_or_else(|| "missing access_token".to_string())?;
    Ok(access_token)
}

fn send_request(request: &UsageRequest, access_token: &str) -> Result<HttpResponse, String> {
    let url = build_usage_url(&request.endpoint, &request.api_version)?;
    let payload = serde_json::json!({
        "project": request.project,
    })
    .to_string();

    let response = Command::new("curl")
        .arg("-sS")
        .arg("--connect-timeout")
        .arg(request.connect_timeout_seconds.max(1).to_string())
        .arg("--max-time")
        .arg(request.max_time_seconds.max(1).to_string())
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Accept: application/json")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg("User-Agent: gemini-cli")
        .arg("-H")
        .arg(format!("Authorization: Bearer {access_token}"))
        .arg("--data")
        .arg(payload)
        .arg(&url)
        .arg("-w")
        .arg("\n__HTTP_STATUS__:%{http_code}")
        .output()
        .map_err(|_| format!("gemini-rate-limits: request failed: {url}"))?;

    if !response.status.success() {
        return Err(format!("gemini-rate-limits: request failed: {url}"));
    }

    let response_text = String::from_utf8_lossy(&response.stdout).to_string();
    let (body, status) = split_http_status_marker(&response_text);
    if status == 0 {
        return Err(format!("gemini-rate-limits: request failed: {url}"));
    }

    Ok(HttpResponse { status, body, url })
}

fn build_usage_url(endpoint: &str, api_version: &str) -> Result<String, String> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() || !(endpoint.starts_with("https://") || endpoint.starts_with("http://"))
    {
        return Err(format!(
            "gemini-rate-limits: unsupported endpoint: {}",
            endpoint
        ));
    }
    let api_version = api_version.trim();
    if api_version.is_empty() {
        return Err("gemini-rate-limits: missing code assist api version".to_string());
    }
    Ok(format!("{endpoint}/{api_version}:retrieveUserQuota"))
}

fn split_http_status_marker(raw: &str) -> (String, u16) {
    let marker = "__HTTP_STATUS__:";
    if let Some(index) = raw.rfind(marker) {
        let body = raw[..index]
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_string();
        let status_raw = raw[index + marker.len()..].trim();
        let status = status_raw.parse::<u16>().unwrap_or(0);
        (body, status)
    } else {
        (raw.to_string(), 0)
    }
}

struct HttpResponse {
    status: u16,
    body: String,
    url: String,
}
