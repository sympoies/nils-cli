use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

struct CmdOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn api_rest_bin() -> PathBuf {
    if let Ok(bin) =
        std::env::var("CARGO_BIN_EXE_api-rest").or_else(|_| std::env::var("CARGO_BIN_EXE_api_rest"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("api-rest");
    if bin.exists() {
        return bin;
    }

    panic!("api-rest binary path: NotPresent");
}

fn run_api_rest(cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let mut cmd = Command::new(api_rest_bin());
    cmd.current_dir(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Keep tests hermetic and deterministic: avoid inheriting user proxy or REST_* env vars.
    for key in [
        "REST_URL",
        "REST_TOKEN_NAME",
        "REST_HISTORY_ENABLED",
        "REST_HISTORY_FILE",
        "REST_HISTORY_LOG_URL_ENABLED",
        "REST_ENV_DEFAULT",
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        cmd.env_remove(key);
    }
    cmd.env("NO_PROXY", "127.0.0.1,localhost");
    cmd.env("no_proxy", "127.0.0.1,localhost");

    for (k, v) in envs {
        cmd.env(k, v);
    }

    let output = cmd.output().expect("run api-rest");
    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn write_json(path: &Path, value: &serde_json::Value) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

fn write_file(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, contents.as_bytes()).expect("write file");
}

fn write_health_request(root: &Path) {
    let request_file = root.join("requests/health.request.json");
    write_json(
        &request_file,
        &serde_json::json!({
            "method": "GET",
            "path": "/health",
            "expect": { "status": 200 }
        }),
    );
}

fn read_until_headers_end(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
                if buf.len() > 64 * 1024 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn write_json_response(stream: &mut TcpStream, body: &[u8]) {
    let mut resp = Vec::new();
    resp.extend_from_slice(b"HTTP/1.1 200 OK\r\n");
    resp.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    resp.extend_from_slice(b"Content-Type: application/json\r\n");
    resp.extend_from_slice(b"\r\n");
    resp.extend_from_slice(body);
    let _ = stream.write_all(&resp);
    let _ = stream.flush();
}

struct TestServer {
    base_url: String,
    hits: Arc<AtomicUsize>,
    shutdown: mpsc::Sender<()>,
    join: Option<thread::JoinHandle<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

fn start_json_server(body: &'static [u8]) -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let addr = listener.local_addr().expect("addr");
    let base_url = format!("http://{addr}");

    let hits = Arc::new(AtomicUsize::new(0));
    let hits_for_thread = Arc::clone(&hits);
    let (tx, rx) = mpsc::channel::<()>();
    let body = body.to_vec();

    let join = thread::spawn(move || loop {
        if rx.try_recv().is_ok() {
            break;
        }

        match listener.accept() {
            Ok((mut stream, _peer)) => {
                hits_for_thread.fetch_add(1, Ordering::SeqCst);
                read_until_headers_end(&mut stream);
                write_json_response(&mut stream, &body);
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    });

    TestServer {
        base_url,
        hits,
        shutdown: tx,
        join: Some(join),
    }
}

#[test]
fn endpoint_precedence_url_wins_even_if_env_and_rest_url_set() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/rest")).expect("mkdir setup/rest");
    write_health_request(root);

    let server_url = start_json_server(br#"{"server":"explicit-url"}"#);
    let server_rest_url = start_json_server(br#"{"server":"rest-url"}"#);

    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--no-history",
            "--url",
            &server_url.base_url,
            "--env",
            "staging",
            "requests/health.request.json",
        ],
        &[("REST_URL", &server_rest_url.base_url)],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(
        out.stdout.contains("\"explicit-url\""),
        "stdout={}",
        out.stdout
    );
    assert_eq!(server_url.hits.load(Ordering::SeqCst), 1);
    assert_eq!(server_rest_url.hits.load(Ordering::SeqCst), 0);
}

#[test]
fn endpoint_precedence_env_wins_over_rest_url() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/rest");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup/rest");
    write_health_request(root);

    let server_env = start_json_server(br#"{"server":"env"}"#);
    let server_rest_url = start_json_server(br#"{"server":"rest-url"}"#);

    write_file(
        &setup_dir.join("endpoints.env"),
        &format!("REST_URL_STAGING={}\n", server_env.base_url),
    );

    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--no-history",
            "--env",
            "staging",
            "requests/health.request.json",
        ],
        &[("REST_URL", &server_rest_url.base_url)],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(out.stdout.contains("\"env\""), "stdout={}", out.stdout);
    assert_eq!(server_env.hits.load(Ordering::SeqCst), 1);
    assert_eq!(server_rest_url.hits.load(Ordering::SeqCst), 0);
}

#[test]
fn env_as_url_https_does_not_require_endpoints_env() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/rest")).expect("mkdir setup/rest");
    write_health_request(root);

    // Bind+close to find an unused local port; request should fail, but must not mention endpoints.env.
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port();
    let https_base = format!("https://127.0.0.1:{port}");

    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--no-history",
            "--env",
            &https_base,
            "requests/health.request.json",
        ],
        &[],
    );
    assert_eq!(out.code, 1);
    assert!(
        out.stderr
            .contains("HTTP request failed: GET https://127.0.0.1"),
        "stderr={}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("endpoints.env not found"),
        "stderr={}",
        out.stderr
    );
}

#[test]
fn endpoint_precedence_rest_url_wins_over_rest_env_default() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/rest");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup/rest");
    write_health_request(root);

    let server_rest_url = start_json_server(br#"{"server":"rest-url"}"#);
    let server_default = start_json_server(br#"{"server":"default-env"}"#);

    write_file(
        &setup_dir.join("endpoints.env"),
        &format!(
            "REST_ENV_DEFAULT=staging\nREST_URL_STAGING={}\n",
            server_default.base_url
        ),
    );

    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--no-history",
            "requests/health.request.json",
        ],
        &[("REST_URL", &server_rest_url.base_url)],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(out.stdout.contains("\"rest-url\""), "stdout={}", out.stdout);
    assert_eq!(server_rest_url.hits.load(Ordering::SeqCst), 1);
    assert_eq!(server_default.hits.load(Ordering::SeqCst), 0);
}

#[test]
fn rest_env_default_selects_endpoint_from_files() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/rest");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup/rest");
    write_health_request(root);

    let server_default = start_json_server(br#"{"server":"default-env"}"#);
    write_file(
        &setup_dir.join("endpoints.env"),
        &format!(
            "REST_ENV_DEFAULT=staging\nREST_URL_STAGING={}\n",
            server_default.base_url
        ),
    );

    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--no-history",
            "requests/health.request.json",
        ],
        &[],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(
        out.stdout.contains("\"default-env\""),
        "stdout={}",
        out.stdout
    );
    assert_eq!(server_default.hits.load(Ordering::SeqCst), 1);
}

#[test]
fn missing_endpoints_env_errors_for_named_env() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/rest")).expect("mkdir setup/rest");
    write_health_request(root);

    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--no-history",
            "--env",
            "staging",
            "requests/health.request.json",
        ],
        &[],
    );
    assert_eq!(out.code, 1);
    assert!(
        out.stderr
            .contains("endpoints.env not found (expected under setup/rest/)"),
        "stderr={}",
        out.stderr
    );
}

#[test]
fn unknown_env_lists_available_suffixes_including_local_env() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/rest");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup/rest");
    write_health_request(root);

    write_file(
        &setup_dir.join("endpoints.env"),
        "REST_URL_STAGING=http://example.invalid\n",
    );
    write_file(
        &setup_dir.join("endpoints.local.env"),
        "REST_URL_DEV=http://example.invalid\n",
    );

    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--no-history",
            "--env",
            "prod",
            "requests/health.request.json",
        ],
        &[],
    );
    assert_eq!(out.code, 1);
    assert!(
        out.stderr.contains("Unknown --env 'prod'"),
        "stderr={}",
        out.stderr
    );
    assert!(
        out.stderr.contains("available: dev staging"),
        "stderr={}",
        out.stderr
    );
}
