use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::json;

pub struct UsageResponse {
    pub body: String,
}

pub struct UsageRequest {
    pub target_file: PathBuf,
    pub refresh_on_401: bool,
    pub base_url: String,
    pub connect_timeout_seconds: u64,
    pub max_time_seconds: u64,
}

pub fn fetch_usage(request: &UsageRequest) -> Result<UsageResponse, String> {
    let (access_token, account_id) = read_tokens(&request.target_file)?;
    let mut response = send_request(request, &access_token, account_id.as_deref())?;

    if response.status == 401
        && request.refresh_on_401
        && let Ok((access_token, account_id)) = read_tokens(&request.target_file)
    {
        response = send_request(request, &access_token, account_id.as_deref())?;
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
                "gemini-rate-limits: GET {} failed (HTTP {})",
                response.url, response.status
            ));
        }
        return Err(format!(
            "gemini-rate-limits: GET {} failed (HTTP {})\ngemini-rate-limits: body: {}",
            response.url, response.status, preview
        ));
    }

    Ok(UsageResponse {
        body: response.body,
    })
}

pub fn read_tokens(target_file: &Path) -> Result<(String, Option<String>), String> {
    let value = json::read_json(target_file).map_err(|err| err.to_string())?;
    let access_token = json::string_at(&value, &["tokens", "access_token"])
        .ok_or_else(|| "missing access_token".to_string())?;
    let account_id = json::string_at(&value, &["tokens", "account_id"])
        .or_else(|| json::string_at(&value, &["account_id"]));
    Ok((access_token, account_id))
}

fn send_request(
    request: &UsageRequest,
    access_token: &str,
    account_id: Option<&str>,
) -> Result<HttpResponse, String> {
    let (host_port, base_path) = parse_http_base_url(&request.base_url)?;
    let usage_path = if base_path.is_empty() {
        "/wham/usage".to_string()
    } else {
        format!("{}/wham/usage", base_path)
    };
    let url = format!("http://{host_port}{usage_path}");

    let mut addrs = host_port
        .to_socket_addrs()
        .map_err(|_| format!("gemini-rate-limits: request failed: {url}"))?;
    let addr = addrs
        .next()
        .ok_or_else(|| format!("gemini-rate-limits: request failed: {url}"))?;

    let connect_timeout = Duration::from_secs(request.connect_timeout_seconds.max(1));
    let read_timeout = Duration::from_secs(request.max_time_seconds.max(1));
    let mut stream = TcpStream::connect_timeout(&addr, connect_timeout)
        .map_err(|_| format!("gemini-rate-limits: request failed: {url}"))?;
    let _ = stream.set_read_timeout(Some(read_timeout));
    let _ = stream.set_write_timeout(Some(read_timeout));

    let mut request_text = format!(
        "GET {usage_path} HTTP/1.1\r\nHost: {host_port}\r\nAccept: application/json\r\nUser-Agent: gemini-cli\r\nAuthorization: Bearer {access_token}\r\nConnection: close\r\n"
    );
    if let Some(account_id) = account_id {
        request_text.push_str(&format!("ChatGPT-Account-Id: {account_id}\r\n"));
    }
    request_text.push_str("\r\n");

    stream
        .write_all(request_text.as_bytes())
        .map_err(|_| format!("gemini-rate-limits: request failed: {url}"))?;

    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|_| format!("gemini-rate-limits: request failed: {url}"))?;

    let response_text = String::from_utf8_lossy(&bytes).to_string();
    let status = parse_status(&response_text)
        .ok_or_else(|| format!("gemini-rate-limits: request failed: {url}"))?;
    let body = extract_body(&response_text);

    Ok(HttpResponse { status, body, url })
}

fn parse_http_base_url(raw: &str) -> Result<(String, String), String> {
    let trimmed = raw.trim();
    let without_scheme = trimmed
        .strip_prefix("http://")
        .ok_or_else(|| format!("gemini-rate-limits: unsupported base url: {trimmed}"))?;

    let (host_port, rest) = match without_scheme.split_once('/') {
        Some((host, tail)) => (host.to_string(), format!("/{}", tail.trim_matches('/'))),
        None => (without_scheme.to_string(), String::new()),
    };

    if host_port.is_empty() {
        return Err(format!(
            "gemini-rate-limits: unsupported base url: {trimmed}"
        ));
    }

    let base_path = if rest == "/" { String::new() } else { rest };
    Ok((host_port, base_path))
}

fn parse_status(response_text: &str) -> Option<u16> {
    let first_line = response_text.lines().next()?;
    let mut parts = first_line.split_whitespace();
    let _http = parts.next()?;
    let code = parts.next()?.parse::<u16>().ok()?;
    Some(code)
}

fn extract_body(response_text: &str) -> String {
    if let Some(index) = response_text.find("\r\n\r\n") {
        return response_text[index + 4..].to_string();
    }
    if let Some(index) = response_text.find("\n\n") {
        return response_text[index + 2..].to_string();
    }
    String::new()
}

struct HttpResponse {
    status: u16,
    body: String,
    url: String,
}
