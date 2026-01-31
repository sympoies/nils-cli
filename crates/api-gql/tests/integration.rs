use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use tempfile::TempDir;

const LOGIN_TOKEN_JWT: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.e30.sig";

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
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

struct CmdOutput {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_api_gql(cwd: &Path, args: &[&str]) -> CmdOutput {
    let output = Command::new(api_gql_bin())
        .current_dir(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run api-gql");

    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn write_file(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, bytes).expect("write");
}

fn write_str(path: &Path, contents: &str) {
    write_file(path, contents.as_bytes());
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

fn parse_headers_and_rest(mut data: Vec<u8>) -> (String, String, Vec<(String, String)>, Vec<u8>) {
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
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let path = target.split('?').next().unwrap_or("").to_string();

    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }

    (method, path, headers, rest)
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

fn take_line(buf: &mut Vec<u8>, stream: &mut TcpStream) -> Option<Vec<u8>> {
    loop {
        if let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") {
            let mut line = buf.drain(..pos).collect::<Vec<u8>>();
            let _ = buf.drain(..2);
            if line.ends_with(b"\r") {
                line.pop();
            }
            return Some(line);
        }

        let mut tmp = [0u8; 4096];
        let n = stream.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
}

fn read_chunked_body(stream: &mut TcpStream, mut buf: Vec<u8>) -> Vec<u8> {
    let mut body = Vec::new();
    while let Some(line) = take_line(&mut buf, stream) {
        let line_str = String::from_utf8_lossy(&line);
        let size_str = line_str.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size_str, 16) else {
            break;
        };
        if size == 0 {
            while let Some(l) = take_line(&mut buf, stream) {
                if l.is_empty() {
                    break;
                }
            }
            break;
        }

        while buf.len() < size + 2 {
            let mut tmp = [0u8; 8192];
            let n = stream.read(&mut tmp).expect("read chunk");
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }

        if buf.len() < size + 2 {
            break;
        }
        body.extend_from_slice(&buf[..size]);
        buf.drain(..size + 2);
    }
    body
}

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");

    let header_bytes = read_until_headers_end(stream);
    let (method, path, headers, rest) = parse_headers_and_rest(header_bytes);

    if header_value(&headers, "expect")
        .is_some_and(|v| v.to_ascii_lowercase().contains("100-continue"))
    {
        let _ = stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        let _ = stream.flush();
    }

    let body = if let Some(te) =
        header_value(&headers, "transfer-encoding").map(|v| v.to_ascii_lowercase())
    {
        if te.contains("chunked") {
            read_chunked_body(stream, rest)
        } else {
            Vec::new()
        }
    } else if let Some(cl) = header_value(&headers, "content-length") {
        let len = cl.parse::<usize>().unwrap_or(0);
        read_exact_bytes(stream, rest, len)
    } else {
        Vec::new()
    };

    RecordedRequest {
        method,
        path,
        headers,
        body,
    }
}

fn body_has_login_query(body: &[u8]) -> bool {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    let Some(q) = v.get("query").and_then(|q| q.as_str()) else {
        return false;
    };
    q.contains("login")
}

fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
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
                let req = read_request(&mut stream);
                {
                    let mut locked = requests_for_thread.lock().expect("lock");
                    locked.push(req.clone());
                }

                match (req.method.as_str(), req.path.as_str()) {
                    ("POST", "/graphql") => {
                        if body_has_login_query(&req.body) {
                            let body = format!(
                                r#"{{"data":{{"login":{{"accessToken":"{LOGIN_TOKEN_JWT}"}}}}}}"#
                            );
                            write_response(&mut stream, 200, "application/json", body.as_bytes());
                        } else {
                            write_response(
                                &mut stream,
                                200,
                                "application/json",
                                br#"{"data":{"ok":true}}"#,
                            );
                        }
                    }
                    ("POST", "/graphql500") => {
                        write_response(&mut stream, 500, "application/json", br#"{"error":"no"}"#);
                    }
                    _ => write_response(&mut stream, 404, "text/plain", b"not found"),
                }
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

#[test]
fn call_vars_min_limit_bumps_nested_limit_fields() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();

    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    let server = start_server();
    write_str(
        &setup_dir.join("endpoints.env"),
        &format!("GQL_URL_LOCAL={}/graphql\n", server.base_url),
    );

    let op = root.join("ops/q.graphql");
    write_str(&op, "query Q($limit: Int) { ok }\n");

    let vars = root.join("ops/q.variables.json");
    write_str(
        &vars,
        r#"{"limit":1,"nested":{"limit":2},"arr":[{"limit":3},{"limit":10}]}"#,
    );

    let out = run_api_gql(
        root,
        &[
            "call",
            "--config-dir",
            "setup/graphql",
            "--env",
            "local",
            "ops/q.graphql",
            "ops/q.variables.json",
        ],
    );

    assert_eq!(
        out.code,
        0,
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout_json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout json");
    assert_eq!(stdout_json["data"]["ok"], true);

    let reqs = server.requests.lock().expect("lock").clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "POST");
    assert_eq!(reqs[0].path, "/graphql");
    let sent: serde_json::Value = serde_json::from_slice(&reqs[0].body).expect("sent json");
    assert_eq!(sent["variables"]["limit"], 5);
    assert_eq!(sent["variables"]["nested"]["limit"], 5);
    assert_eq!(sent["variables"]["arr"][0]["limit"], 5);
    assert_eq!(sent["variables"]["arr"][1]["limit"], 10);
}

#[test]
fn call_non_2xx_exits_1() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/graphql")).expect("mkdir setup");
    let server = start_server();

    let op = root.join("q.graphql");
    write_str(&op, "query Q { ok }\n");

    let out = run_api_gql(
        root,
        &[
            "call",
            "--config-dir",
            "setup/graphql",
            "--url",
            &format!("{}/graphql500", server.base_url),
            "q.graphql",
        ],
    );
    assert_eq!(out.code, 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("HTTP request failed with status"));
}

#[test]
fn call_jwt_profile_auto_login_injects_bearer_token() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();

    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    let server = start_server();
    write_str(
        &setup_dir.join("endpoints.env"),
        &format!("GQL_URL_LOCAL={}/graphql\n", server.base_url),
    );
    write_str(&setup_dir.join("jwts.env"), "GQL_JWT_ADMIN=\n");

    // Selected profile is missing => auto-login uses login.<profile>.graphql
    write_str(
        &setup_dir.join("login.admin.graphql"),
        "query Login { login { accessToken } }\n",
    );

    let op = root.join("q.graphql");
    write_str(&op, "query Q { ok }\n");

    let out = run_api_gql(
        root,
        &[
            "call",
            "--config-dir",
            "setup/graphql",
            "--env",
            "local",
            "--jwt",
            "admin",
            "q.graphql",
        ],
    );

    assert_eq!(
        out.code,
        0,
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let reqs = server.requests.lock().expect("lock").clone();
    assert_eq!(reqs.len(), 2);

    assert!(
        header_value(&reqs[0].headers, "authorization").is_none(),
        "login should not include Authorization"
    );

    let auth = header_value(&reqs[1].headers, "authorization").unwrap_or_default();
    assert_eq!(auth, format!("Bearer {LOGIN_TOKEN_JWT}"));
}

#[test]
fn report_blocks_empty_by_default() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();

    let op = root.join("q.graphql");
    write_str(&op, "query Q { ok }\n");

    let resp = root.join("resp.json");
    write_str(&resp, r#"{"data":{"pageInfo":{"hasNextPage":false}}}"#);

    let out = run_api_gql(
        root,
        &[
            "report",
            "--case",
            "empty",
            "--op",
            "q.graphql",
            "--response",
            "resp.json",
        ],
    );
    assert_eq!(out.code, 1);
    assert!(String::from_utf8_lossy(&out.stderr).contains("no data records"));
}

#[test]
fn report_allow_empty_writes_report() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();

    let op = root.join("q.graphql");
    write_str(&op, "query Q { ok }\n");

    let resp = root.join("resp.json");
    write_str(&resp, r#"{"data":{"pageInfo":{"hasNextPage":false}}}"#);

    let out = run_api_gql(
        root,
        &[
            "report",
            "--case",
            "empty",
            "--op",
            "q.graphql",
            "--response",
            "resp.json",
            "--allow-empty",
        ],
    );
    assert_eq!(out.code, 0);
    let report_path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(!report_path.is_empty());
    assert!(Path::new(&report_path).is_file());
    let contents = std::fs::read_to_string(&report_path).expect("read report");
    assert!(contents.contains("# API Test Report"));
}
