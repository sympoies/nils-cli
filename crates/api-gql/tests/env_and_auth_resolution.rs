use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[derive(Debug)]
struct CmdOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn api_gql_bin() -> PathBuf {
    if let Ok(bin) =
        std::env::var("CARGO_BIN_EXE_api-gql").or_else(|_| std::env::var("CARGO_BIN_EXE_api_gql"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("api-gql");
    if bin.exists() {
        return bin;
    }

    panic!("api-gql binary path: NotPresent");
}

fn run_api_gql(cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let mut cmd = Command::new(api_gql_bin());
    cmd.current_dir(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("GQL_URL")
        .env_remove("GQL_ENV_DEFAULT")
        .env_remove("GQL_JWT_NAME")
        .env_remove("ACCESS_TOKEN")
        .env_remove("GQL_SCHEMA_FILE")
        .env("GQL_JWT_VALIDATE_ENABLED", "false");

    for (k, v) in envs {
        cmd.env(k, v);
    }

    let output = cmd.output().expect("run api-gql");
    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn write_str(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
}

#[derive(Debug, Clone)]
struct RecordedRequest {
    path: String,
    authorization: Option<String>,
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

fn parse_request_line_and_headers(mut data: Vec<u8>) -> (String, Vec<(String, String)>, Vec<u8>) {
    let mut split_at = None;
    for i in 0..data.len().saturating_sub(3) {
        if &data[i..i + 4] == b"\r\n\r\n" {
            split_at = Some(i);
            break;
        }
    }
    let split_at = split_at.expect("header terminator");
    let rest = data.split_off(split_at + 4);
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

    (path, headers, rest)
}

fn header_value(headers: &[(String, String)], key: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

fn read_exact_bytes(stream: &mut TcpStream, mut already: Vec<u8>, want: usize) -> Vec<u8> {
    while already.len() < want {
        let mut tmp = vec![0u8; (want - already.len()).min(8192)];
        let n = stream.read(&mut tmp).expect("read body");
        if n == 0 {
            break;
        }
        already.extend_from_slice(&tmp[..n]);
    }
    already.truncate(want);
    already
}

fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let mut resp = Vec::new();
    resp.extend_from_slice(format!("HTTP/1.1 {status} {reason}\r\n").as_bytes());
    resp.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    if !content_type.is_empty() {
        resp.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
    }
    resp.extend_from_slice(b"\r\n");
    resp.extend_from_slice(body);
    let _ = stream.write_all(&resp);
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

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    let requests: Arc<Mutex<Vec<RecordedRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let requests_for_thread = Arc::clone(&requests);

    let join = thread::spawn(move || loop {
        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        match listener.accept() {
            Ok((mut stream, _peer)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let header_bytes = read_until_headers_end(&mut stream);
                let (path, headers, rest) = parse_request_line_and_headers(header_bytes);

                if let Some(cl) = header_value(&headers, "content-length") {
                    let want = cl.parse::<usize>().unwrap_or(0);
                    let _body = read_exact_bytes(&mut stream, rest, want);
                }

                let auth = header_value(&headers, "authorization");
                {
                    let mut locked = requests_for_thread.lock().expect("lock");
                    locked.push(RecordedRequest {
                        path,
                        authorization: auth,
                    });
                }

                write_response(
                    &mut stream,
                    200,
                    "application/json",
                    br#"{"data":{"ok":true}}"#,
                );
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    });

    TestServer {
        base_url: format!("http://{addr}"),
        requests,
        shutdown: shutdown_tx,
        join: Some(join),
    }
}

#[test]
fn list_envs_outputs_sorted_deduped_suffixes_from_endpoints_env_and_local() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    write_str(
        &setup_dir.join("endpoints.env"),
        r#"
# comment
export GQL_URL_PROD=http://example.invalid/graphql
GQL_URL_STAGING=http://example.invalid/graphql
"#,
    );
    write_str(
        &setup_dir.join("endpoints.local.env"),
        r#"
GQL_URL_LOCAL=http://example.invalid/graphql
GQL_URL_PROD=http://example.invalid/graphql
"#,
    );

    let out = run_api_gql(
        root,
        &["call", "--config-dir", "setup/graphql", "--list-envs"],
        &[],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);

    let lines: Vec<String> = out
        .stdout
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect();
    assert_eq!(lines, vec!["local", "prod", "staging"]);
}

#[test]
fn env_endpoint_prefers_endpoints_local_over_endpoints_env() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    let server_a = start_server();
    let server_b = start_server();
    write_str(
        &setup_dir.join("endpoints.env"),
        &format!("GQL_URL_STAGING={}/graphql\n", server_a.base_url),
    );
    write_str(
        &setup_dir.join("endpoints.local.env"),
        &format!("GQL_URL_STAGING={}/graphql\n", server_b.base_url),
    );

    write_str(&root.join("q.graphql"), "query Q { ok }\n");
    let out = run_api_gql(
        root,
        &[
            "call",
            "--no-history",
            "--config-dir",
            "setup/graphql",
            "--env",
            "staging",
            "q.graphql",
        ],
        &[],
    );

    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(out.stdout.contains("\"ok\":true"), "stdout={}", out.stdout);

    assert_eq!(server_a.requests.lock().expect("lock").len(), 0);
    assert_eq!(server_b.requests.lock().expect("lock").len(), 1);
}

#[test]
fn unknown_env_error_lists_available_suffixes() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    write_str(
        &setup_dir.join("endpoints.env"),
        "GQL_URL_LOCAL=http://example.invalid/graphql\n",
    );
    write_str(
        &setup_dir.join("endpoints.local.env"),
        "GQL_URL_STAGING=http://example.invalid/graphql\n",
    );
    write_str(&root.join("q.graphql"), "query Q { ok }\n");

    let out = run_api_gql(
        root,
        &[
            "call",
            "--no-history",
            "--config-dir",
            "setup/graphql",
            "--env",
            "does-not-exist",
            "q.graphql",
        ],
        &[],
    );
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("Unknown --env 'does-not-exist'"));
    assert!(out.stderr.contains("available:"));
    assert!(out.stderr.contains("local"));
    assert!(out.stderr.contains("staging"));
}

#[test]
fn jwt_flag_wins_over_env_and_file_profile_selection() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    let server = start_server();
    write_str(
        &setup_dir.join("endpoints.env"),
        &format!("GQL_URL_LOCAL={}/graphql\n", server.base_url),
    );
    write_str(
        &setup_dir.join("jwts.env"),
        r#"
GQL_JWT_NAME=file
GQL_JWT_FILE=file_token
GQL_JWT_ENV=env_token
GQL_JWT_ARG=arg_token
"#,
    );

    write_str(&root.join("q.graphql"), "query Q { ok }\n");
    let out = run_api_gql(
        root,
        &[
            "call",
            "--no-history",
            "--config-dir",
            "setup/graphql",
            "--env",
            "local",
            "--jwt",
            "arg",
            "q.graphql",
        ],
        &[("GQL_JWT_NAME", "env")],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);

    let reqs = server.requests.lock().expect("lock").clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path, "/graphql");
    assert_eq!(reqs[0].authorization.as_deref(), Some("Bearer arg_token"));
}

#[test]
fn jwt_env_profile_selection_wins_over_file_default() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    let server = start_server();
    write_str(
        &setup_dir.join("endpoints.env"),
        &format!("GQL_URL_LOCAL={}/graphql\n", server.base_url),
    );
    write_str(
        &setup_dir.join("jwts.env"),
        r#"
GQL_JWT_NAME=file
GQL_JWT_FILE=file_token
GQL_JWT_ENV=env_token
"#,
    );

    write_str(&root.join("q.graphql"), "query Q { ok }\n");
    let out = run_api_gql(
        root,
        &[
            "call",
            "--no-history",
            "--config-dir",
            "setup/graphql",
            "--env",
            "local",
            "q.graphql",
        ],
        &[("GQL_JWT_NAME", "env")],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);

    let reqs = server.requests.lock().expect("lock").clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].authorization.as_deref(), Some("Bearer env_token"));
}

#[test]
fn jwt_file_profile_selection_is_used_when_no_flag_or_env() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    let server = start_server();
    write_str(
        &setup_dir.join("endpoints.env"),
        &format!("GQL_URL_LOCAL={}/graphql\n", server.base_url),
    );
    write_str(
        &setup_dir.join("jwts.env"),
        r#"
GQL_JWT_NAME=file
GQL_JWT_FILE=file_token
"#,
    );

    write_str(&root.join("q.graphql"), "query Q { ok }\n");
    let out = run_api_gql(
        root,
        &[
            "call",
            "--no-history",
            "--config-dir",
            "setup/graphql",
            "--env",
            "local",
            "q.graphql",
        ],
        &[],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);

    let reqs = server.requests.lock().expect("lock").clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].authorization.as_deref(), Some("Bearer file_token"));
}
