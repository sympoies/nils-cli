use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[derive(Debug, Clone)]
struct RecordedRequest {
    headers: Vec<(String, String)>,
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

struct CmdOutput {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_api_rest(cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let mut cmd = Command::new(api_rest_bin());
    cmd.current_dir(cwd)
        .env_clear()
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (k, v) in envs {
        cmd.env(k, v);
    }

    let output = cmd.output().expect("run api-rest");

    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn write_json(path: &Path, value: &serde_json::Value) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

fn write_file(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write file");
}

fn read_until_headers_end(stream: &mut TcpStream) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream.read(&mut tmp).expect("read");
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    buf
}

fn parse_headers(mut data: Vec<u8>) -> (String, Vec<(String, String)>) {
    let mut split_at = None;
    for i in 0..data.len().saturating_sub(3) {
        if &data[i..i + 4] == b"\r\n\r\n" {
            split_at = Some(i);
            break;
        }
    }
    let split_at = split_at.expect("header terminator");
    data.truncate(split_at);

    let header_text = String::from_utf8_lossy(&data);
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let _method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let path = target.split('?').next().unwrap_or("").to_string();

    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }

    (path, headers)
}

fn header_value(headers: &[(String, String)], key: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

fn write_response(stream: &mut TcpStream, status: u16, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let mut resp = Vec::new();
    resp.extend_from_slice(format!("HTTP/1.1 {status} {reason}\r\n").as_bytes());
    resp.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    resp.extend_from_slice(b"Content-Type: application/json\r\n");
    resp.extend_from_slice(b"\r\n");
    resp.extend_from_slice(body);
    let _ = std::io::Write::write_all(stream, &resp);
}

struct TestServer {
    base_url: String,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
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

fn start_server() -> TestServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let addr = listener.local_addr().expect("addr");
    let base_url = format!("http://{addr}");

    let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel::<()>();
    let requests_for_thread = Arc::clone(&requests);

    let join = thread::spawn(move || loop {
        if rx.try_recv().is_ok() {
            break;
        }

        match listener.accept() {
            Ok((mut stream, _peer)) => {
                let _ = stream.set_nonblocking(false);
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("timeout");

                let header_bytes = read_until_headers_end(&mut stream);
                let (_path, headers) = parse_headers(header_bytes);
                {
                    let mut locked = requests_for_thread.lock().expect("lock");
                    locked.push(RecordedRequest { headers });
                }

                write_response(&mut stream, 200, br#"{"ok":true}"#);
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    });

    TestServer {
        base_url,
        requests,
        shutdown: tx,
        join: Some(join),
    }
}

fn write_health_request(root: &Path) {
    write_json(
        &root.join("requests/health.request.json"),
        &serde_json::json!({
            "method": "GET",
            "path": "/health",
            "expect": { "status": 200 }
        }),
    );
}

#[test]
fn token_profile_cli_wins_over_env_and_file() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/rest")).expect("mkdir setup");
    write_health_request(root);

    write_file(
        &root.join("setup/rest/tokens.env"),
        "REST_TOKEN_NAME=file\nREST_TOKEN_FILE=file-token\nREST_TOKEN_ENV=env-token\nREST_TOKEN_CLI=cli-token\n",
    );

    let server = start_server();
    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--url",
            &server.base_url,
            "--token",
            "cli",
            "requests/health.request.json",
        ],
        &[
            ("REST_TOKEN_NAME", "env"),
            ("REST_JWT_VALIDATE_ENABLED", "false"),
            ("REST_HISTORY_ENABLED", "false"),
        ],
    );
    assert_eq!(
        out.code,
        0,
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"ok\""));

    let reqs = server.requests.lock().expect("lock").clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        header_value(&reqs[0].headers, "authorization").unwrap_or_default(),
        "Bearer cli-token"
    );
}

#[test]
fn token_profile_env_wins_over_tokens_file() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/rest")).expect("mkdir setup");
    write_health_request(root);

    write_file(
        &root.join("setup/rest/tokens.env"),
        "REST_TOKEN_NAME=file\nREST_TOKEN_FILE=file-token\nREST_TOKEN_ENV=env-token\n",
    );

    let server = start_server();
    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--url",
            &server.base_url,
            "requests/health.request.json",
        ],
        &[
            ("REST_TOKEN_NAME", "env"),
            ("REST_JWT_VALIDATE_ENABLED", "false"),
            ("REST_HISTORY_ENABLED", "false"),
        ],
    );
    assert_eq!(
        out.code,
        0,
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"ok\""));

    let reqs = server.requests.lock().expect("lock").clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        header_value(&reqs[0].headers, "authorization").unwrap_or_default(),
        "Bearer env-token"
    );
}

#[test]
fn token_profile_from_tokens_local_env_is_used_when_cli_and_env_missing() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/rest")).expect("mkdir setup");
    write_health_request(root);

    write_file(
        &root.join("setup/rest/tokens.env"),
        "REST_TOKEN_NAME=file\nREST_TOKEN_FILE=file-token\n",
    );
    write_file(
        &root.join("setup/rest/tokens.local.env"),
        "REST_TOKEN_NAME=local\nREST_TOKEN_LOCAL=local-token\n",
    );

    let server = start_server();
    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--url",
            &server.base_url,
            "requests/health.request.json",
        ],
        &[
            ("REST_JWT_VALIDATE_ENABLED", "false"),
            ("REST_HISTORY_ENABLED", "false"),
        ],
    );
    assert_eq!(
        out.code,
        0,
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"ok\""));

    let reqs = server.requests.lock().expect("lock").clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        header_value(&reqs[0].headers, "authorization").unwrap_or_default(),
        "Bearer local-token"
    );
}

#[test]
fn unknown_token_profile_error_lists_available_suffixes() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/rest")).expect("mkdir setup");
    write_health_request(root);

    write_file(
        &root.join("setup/rest/tokens.env"),
        "REST_TOKEN_NAME=alpha\nREST_TOKEN_ALPHA=alpha-token\n",
    );
    write_file(
        &root.join("setup/rest/tokens.local.env"),
        "REST_TOKEN_BETA=beta-token\n",
    );

    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--token",
            "missing",
            "requests/health.request.json",
        ],
        &[
            ("REST_JWT_VALIDATE_ENABLED", "false"),
            ("REST_HISTORY_ENABLED", "false"),
        ],
    );
    assert_eq!(out.code, 1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Token profile 'missing' is empty/missing"));
    assert!(stderr.contains("available: alpha beta"));
    assert!(!stderr.contains("available: name"));
}

#[test]
fn token_profile_selected_without_tokens_files_shows_available_none() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/rest")).expect("mkdir setup");
    write_health_request(root);

    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--token",
            "cli",
            "requests/health.request.json",
        ],
        &[
            ("REST_JWT_VALIDATE_ENABLED", "false"),
            ("REST_HISTORY_ENABLED", "false"),
        ],
    );
    assert_eq!(out.code, 1);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Token profile 'cli' is empty/missing"));
    assert!(stderr.contains("available: none"));
}

#[test]
fn access_token_fallback_works_without_tokens_files_when_no_profile_selected() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/rest")).expect("mkdir setup");
    write_health_request(root);

    let server = start_server();
    let out = run_api_rest(
        root,
        &[
            "call",
            "--config-dir",
            "setup/rest",
            "--url",
            &server.base_url,
            "requests/health.request.json",
        ],
        &[
            ("ACCESS_TOKEN", "access-fallback-token"),
            ("REST_JWT_VALIDATE_ENABLED", "false"),
            ("REST_HISTORY_ENABLED", "false"),
        ],
    );
    assert_eq!(
        out.code,
        0,
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"ok\""));

    let reqs = server.requests.lock().expect("lock").clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        header_value(&reqs[0].headers, "authorization").unwrap_or_default(),
        "Bearer access-fallback-token"
    );
}
