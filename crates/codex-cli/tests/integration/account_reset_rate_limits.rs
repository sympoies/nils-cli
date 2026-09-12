use codex_cli::rate_limits::client::{ResetCreditRequest, consume_reset_credit};
use nils_common::provider_usage::ProviderUsageReason;
use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use nils_test_support::http::{HttpResponse, LoopbackServer};
use nils_test_support::write_exe;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

const IDEMPOTENCY_KEY: &str = "8ae96ff3-3425-4f4c-8772-b6fd61502868";

fn codex_cli_bin() -> PathBuf {
    bin::resolve("codex-cli")
}

fn run(args: &[&str], secret_dir: &Path, base_url: &str) -> CmdOutput {
    let options = CmdOptions::new()
        .with_env("CODEX_SECRET_DIR", secret_dir.to_string_lossy().as_ref())
        .with_env("CODEX_CHATGPT_BASE_URL", base_url)
        .with_env("CODEX_AUTO_REFRESH_ENABLED", "false");
    cmd::run_with(&codex_cli_bin(), args, &options)
}

fn write_secret(secret_dir: &Path) {
    fs::create_dir_all(secret_dir).expect("secret dir");
    fs::write(
        secret_dir.join("alpha.json"),
        r#"{"tokens":{"access_token":"tok-alpha","account_id":"acct_001"}}"#,
    )
    .expect("secret");
}

fn read_http_request(stream: &mut impl Read) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let header_end = header_end + 4;
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::trim)
                    .map(str::to_owned)
            })
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if bytes.len() >= header_end + content_length {
            break;
        }
    }
    String::from_utf8(bytes).expect("utf8 request")
}

fn reset_request(target_file: PathBuf, base_url: String) -> ResetCreditRequest {
    ResetCreditRequest {
        target_file,
        refresh_on_401: false,
        suppress_auth_refresh_output: true,
        base_url,
        connect_timeout_seconds: 1,
        max_time_seconds: 3,
        idempotency_key: IDEMPOTENCY_KEY.to_string(),
    }
}

#[test]
fn reset_rate_limits_posts_exact_idempotent_request_and_projects_success() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    write_secret(&secrets);
    fs::rename(secrets.join("alpha.json"), secrets.join("Alpha Team.json")).expect("rename secret");
    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "POST",
        "/wham/rate-limit-reset-credits/consume",
        HttpResponse::new(
            200,
            r#"{"code":"reset","windows_reset":2,"credit":{"id":"must-not-leak"}}"#,
        ),
    );

    let output = run(
        &[
            "account",
            "reset-rate-limits",
            "--yes",
            "--idempotency-key",
            IDEMPOTENCY_KEY,
            "--format",
            "json",
            "Alpha Team.json",
        ],
        &secrets,
        &server.url(),
    );

    assert_eq!(output.code, 0, "{}", output.stderr_text());
    let payload: Value = serde_json::from_str(&output.stdout_text()).expect("json");
    assert_eq!(
        payload["schema_version"],
        "codex-cli.account.reset-rate-limits.v1"
    );
    assert_eq!(payload["command"], "account reset-rate-limits");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["provider"], "codex");
    assert!(payload["result"].get("name").is_none());
    assert!(payload["result"].get("target_file").is_none());
    assert_eq!(payload["result"]["outcome"], "reset");
    assert_eq!(payload["result"]["windows_reset"], 2);
    assert!(!output.stdout_text().contains("Alpha Team"));
    assert!(!output.stderr_text().contains("Alpha Team"));
    assert!(!output.stdout_text().contains("must-not-leak"));
    assert!(!output.stdout_text().contains("acct_001"));
    assert!(!output.stdout_text().contains("tok-alpha"));
    assert!(
        !output
            .stdout_text()
            .contains(dir.path().to_string_lossy().as_ref())
    );

    let requests = server.take_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/wham/rate-limit-reset-credits/consume");
    assert_eq!(
        requests[0].header_value("authorization"),
        Some("Bearer tok-alpha".to_string())
    );
    assert_eq!(
        requests[0].header_value("chatgpt-account-id"),
        Some("acct_001".to_string())
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body_text()).expect("request json"),
        serde_json::json!({"redeem_request_id": IDEMPOTENCY_KEY})
    );
}

#[test]
fn reset_rate_limits_json_stays_parseable_across_successful_auth_refresh() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let auth_file = dir.path().join("auth.json");
    fs::write(
        &auth_file,
        r#"{"tokens":{"access_token":"stale-token","account_id":"acct_001"}}"#,
    )
    .expect("auth file");
    let stubs = dir.path().join("stubs");
    fs::create_dir_all(&stubs).expect("stubs");
    write_exe(
        &stubs,
        "ssh",
        r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$REMOTE_AUTH_PAYLOAD"
"#,
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let base_url = format!("http://{}", listener.local_addr().expect("address"));
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in [(401, ""), (200, r#"{"code":"reset","windows_reset":1}"#)] {
            let (mut stream, _) = listener.accept().expect("accept");
            requests.push(read_http_request(&mut stream));
            let reason = if status == 200 { "OK" } else { "Unauthorized" };
            write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            )
            .expect("write response");
        }
        requests
    });

    let remote_payload = r#"{"tokens":{"access_token":"fresh-token","account_id":"acct_001"},"last_refresh":"2026-09-12T12:00:00Z"}"#;
    let options = CmdOptions::new()
        .with_path_prepend(&stubs)
        .with_env("CODEX_AUTH_FILE", auth_file.to_string_lossy().as_ref())
        .with_env("CODEX_AUTO_REFRESH_ENABLED", "true")
        .with_env("CODEX_AUTH_REMOTE_SSH", "auth-host")
        .with_env("CODEX_AUTH_REMOTE_NAME", "alpha")
        .with_env("CODEX_CHATGPT_BASE_URL", &base_url)
        .with_env("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", "1")
        .with_env("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", "3")
        .with_env("REMOTE_AUTH_PAYLOAD", remote_payload);
    let output = cmd::run_with(
        &codex_cli_bin(),
        &[
            "account",
            "reset-rate-limits",
            "--yes",
            "--idempotency-key",
            IDEMPOTENCY_KEY,
            "--format",
            "json",
        ],
        &options,
    );

    assert_eq!(output.code, 0, "{}", output.stderr_text());
    let payload: Value = serde_json::from_str(&output.stdout_text()).expect("single JSON envelope");
    assert_eq!(payload["result"]["outcome"], "reset");
    assert!(!output.stdout_text().contains("remote-refreshed"));
    let requests = server.join().expect("server thread");
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.contains(IDEMPOTENCY_KEY))
    );
    assert!(requests[0].contains("authorization: Bearer stale-token"));
    assert!(requests[1].contains("authorization: Bearer fresh-token"));
}

#[test]
fn reset_rate_limits_text_suppresses_nested_auth_refresh_paths() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let auth_file = dir.path().join("sentinel-private-auth.json");
    fs::write(
        &auth_file,
        r#"{"tokens":{"access_token":"stale-token","account_id":"acct_001"}}"#,
    )
    .expect("auth file");
    let stubs = dir.path().join("stubs");
    fs::create_dir_all(&stubs).expect("stubs");
    write_exe(
        &stubs,
        "ssh",
        r#"#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$REMOTE_AUTH_PAYLOAD"
"#,
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let base_url = format!("http://{}", listener.local_addr().expect("address"));
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in [(401, ""), (200, r#"{"code":"reset","windows_reset":1}"#)] {
            let (mut stream, _) = listener.accept().expect("accept");
            requests.push(read_http_request(&mut stream));
            let reason = if status == 200 { "OK" } else { "Unauthorized" };
            write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            )
            .expect("write response");
        }
        requests
    });

    let remote_payload = r#"{"tokens":{"access_token":"fresh-token","account_id":"acct_001"},"last_refresh":"2026-09-12T12:00:00Z"}"#;
    let options = CmdOptions::new()
        .with_path_prepend(&stubs)
        .with_env("CODEX_AUTH_FILE", auth_file.to_string_lossy().as_ref())
        .with_env("CODEX_AUTO_REFRESH_ENABLED", "true")
        .with_env("CODEX_AUTH_REMOTE_SSH", "auth-host")
        .with_env("CODEX_AUTH_REMOTE_NAME", "alpha")
        .with_env("CODEX_CHATGPT_BASE_URL", &base_url)
        .with_env("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", "1")
        .with_env("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", "3")
        .with_env("REMOTE_AUTH_PAYLOAD", remote_payload);
    let output = cmd::run_with(
        &codex_cli_bin(),
        &[
            "account",
            "reset-rate-limits",
            "--yes",
            "--idempotency-key",
            IDEMPOTENCY_KEY,
        ],
        &options,
    );

    assert_eq!(output.code, 0, "{}", output.stderr_text());
    assert!(output.stdout_text().contains("Consumed an earned reset"));
    assert!(!output.stdout_text().contains("sentinel-private-auth"));
    let sentinel = auth_file.to_string_lossy();
    assert!(!output.stdout_text().contains(sentinel.as_ref()));
    assert!(!output.stderr_text().contains(sentinel.as_ref()));
    assert!(!output.stdout_text().contains("remote-refreshed"));
    assert_eq!(server.join().expect("server thread").len(), 2);
}

#[test]
fn reset_rate_limits_requires_yes_and_canonical_uuid_before_posting() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    write_secret(&secrets);
    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "POST",
        "/wham/rate-limit-reset-credits/consume",
        HttpResponse::new(200, r#"{"code":"reset","windows_reset":1}"#),
    );

    let missing_yes = run(
        &[
            "account",
            "reset-rate-limits",
            "--idempotency-key",
            IDEMPOTENCY_KEY,
            "--format",
            "json",
            "alpha.json",
        ],
        &secrets,
        &server.url(),
    );
    assert_eq!(missing_yes.code, 64);
    let payload: Value = serde_json::from_str(&missing_yes.stdout_text()).expect("json");
    assert_eq!(payload["error"]["code"], "confirmation-required");
    assert!(server.take_requests().is_empty());

    let bad_uuid = run(
        &[
            "account",
            "reset-rate-limits",
            "--yes",
            "--idempotency-key",
            "not-a-uuid",
            "--format",
            "json",
            "alpha.json",
        ],
        &secrets,
        &server.url(),
    );
    assert_eq!(bad_uuid.code, 64);
    let payload: Value = serde_json::from_str(&bad_uuid.stdout_text()).expect("json");
    assert_eq!(payload["error"]["code"], "invalid-idempotency-key");
    assert!(server.take_requests().is_empty());

    for unsafe_secret in ["alpha\n.json", "alpha\u{1b}.json"] {
        let unsafe_name = run(
            &[
                "account",
                "reset-rate-limits",
                "--yes",
                "--idempotency-key",
                IDEMPOTENCY_KEY,
                "--format",
                "json",
                unsafe_secret,
            ],
            &secrets,
            &server.url(),
        );
        assert_eq!(unsafe_name.code, 64);
        let payload: Value = serde_json::from_str(&unsafe_name.stdout_text()).expect("json");
        assert_eq!(payload["error"]["code"], "invalid-secret-name");
        assert!(server.take_requests().is_empty());
    }
}

#[test]
fn reset_rate_limits_rejects_api_key_auth_without_posting() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secret dir");
    fs::write(
        secrets.join("alpha.json"),
        r#"{"OPENAI_API_KEY":"must-not-leak"}"#,
    )
    .expect("secret");
    let server = LoopbackServer::new().expect("server");
    let output = run(
        &[
            "account",
            "reset-rate-limits",
            "--yes",
            "--idempotency-key",
            IDEMPOTENCY_KEY,
            "--format",
            "json",
            "alpha.json",
        ],
        &secrets,
        &server.url(),
    );
    assert_eq!(output.code, 2);
    let payload: Value = serde_json::from_str(&output.stdout_text()).expect("json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "chatgpt-auth-required");
    assert!(!output.stdout_text().contains("must-not-leak"));
    assert!(server.take_requests().is_empty());
}

#[test]
fn reset_rate_limits_projects_every_domain_outcome_without_exposing_provider_fields() {
    for outcome in ["reset", "nothing_to_reset", "no_credit", "already_redeemed"] {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secrets = dir.path().join("secrets");
        write_secret(&secrets);
        let server = LoopbackServer::new().expect("server");
        server.add_route(
            "POST",
            "/wham/rate-limit-reset-credits/consume",
            HttpResponse::new(
                200,
                serde_json::json!({
                    "code": outcome,
                    "windows_reset": 0,
                    "credit": {"id": "opaque-credit-id"}
                })
                .to_string(),
            ),
        );
        let output = run(
            &[
                "account",
                "reset-rate-limits",
                "--yes",
                "--idempotency-key",
                IDEMPOTENCY_KEY,
                "--format",
                "json",
                "alpha.json",
            ],
            &secrets,
            &server.url(),
        );
        assert_eq!(output.code, 0, "{outcome}: {}", output.stderr_text());
        let payload: Value = serde_json::from_str(&output.stdout_text()).expect("json");
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["result"]["outcome"], outcome);
        assert!(!output.stdout_text().contains("opaque-credit-id"));
    }
}

#[test]
fn reset_client_retries_unauthorized_with_the_same_idempotency_key() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let target = dir.path().join("alpha.json");
    fs::write(&target, r#"{"tokens":{"access_token":"tok-alpha"}}"#).expect("secret");
    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "POST",
        "/wham/rate-limit-reset-credits/consume",
        HttpResponse::new(401, ""),
    );
    let error = consume_reset_credit(&ResetCreditRequest {
        target_file: target,
        refresh_on_401: true,
        suppress_auth_refresh_output: true,
        base_url: server.url(),
        connect_timeout_seconds: 1,
        max_time_seconds: 3,
        idempotency_key: IDEMPOTENCY_KEY.to_string(),
    })
    .expect_err("401 should remain an error");
    assert_eq!(error.reason(), ProviderUsageReason::AuthExpired);
    let requests = server.take_requests();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| {
        serde_json::from_str::<Value>(&request.body_text()).expect("request json")
            == serde_json::json!({"redeem_request_id": IDEMPOTENCY_KEY})
    }));
}

#[test]
fn reset_client_classifies_http_failures_without_forwarding_bodies() {
    for (status, expected) in [
        (403, ProviderUsageReason::PermissionDenied),
        (429, ProviderUsageReason::RateLimited),
        (503, ProviderUsageReason::ServiceUnavailable),
    ] {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let target = dir.path().join("alpha.json");
        fs::write(&target, r#"{"tokens":{"access_token":"tok-alpha"}}"#).expect("secret");
        let server = LoopbackServer::new().expect("server");
        server.add_route(
            "POST",
            "/wham/rate-limit-reset-credits/consume",
            HttpResponse::new(status, "sensitive-provider-body"),
        );
        let error =
            consume_reset_credit(&reset_request(target, server.url())).expect_err("HTTP failure");
        assert_eq!(error.reason(), expected);
        assert!(!error.to_string().contains("sensitive-provider-body"));
    }
}

#[test]
fn reset_client_rejects_malformed_or_negative_success_payloads() {
    for body in [
        "not-json",
        r#"{"code":"future"}"#,
        r#"{"code":"reset","windows_reset":-1}"#,
    ] {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let target = dir.path().join("alpha.json");
        fs::write(&target, r#"{"tokens":{"access_token":"tok-alpha"}}"#).expect("secret");
        let server = LoopbackServer::new().expect("server");
        server.add_route(
            "POST",
            "/wham/rate-limit-reset-credits/consume",
            HttpResponse::new(200, body),
        );
        let error = consume_reset_credit(&reset_request(target, server.url()))
            .expect_err("malformed response");
        assert_eq!(error.reason(), ProviderUsageReason::Unknown);
        assert!(!error.to_string().contains(body));
    }
}

#[test]
fn reset_client_classifies_request_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let base_url = format!("http://{}", listener.local_addr().expect("address"));
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        thread::sleep(Duration::from_secs(2));
    });
    let dir = tempfile::TempDir::new().expect("tempdir");
    let target = dir.path().join("alpha.json");
    fs::write(&target, r#"{"tokens":{"access_token":"tok-alpha"}}"#).expect("secret");
    let mut request = reset_request(target, base_url);
    request.max_time_seconds = 1;
    let error = consume_reset_credit(&request).expect_err("timeout");
    assert_eq!(error.reason(), ProviderUsageReason::Timeout);
    server.join().expect("server thread");
}
