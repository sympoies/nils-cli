use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

const SECRET_TOKEN: &str = "VERY_SECRET_TOKEN";

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    _body: Vec<u8>,
}

fn api_test_bin() -> PathBuf {
    if let Ok(bin) =
        std::env::var("CARGO_BIN_EXE_api-test").or_else(|_| std::env::var("CARGO_BIN_EXE_api_test"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("api-test");
    if bin.exists() {
        return bin;
    }

    panic!("api-test binary path: NotPresent");
}

struct CmdOutput {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_api_test(cwd: &Path, args: &[&str]) -> CmdOutput {
    run_api_test_with_env(cwd, args, &[])
}

fn run_api_test_with_env(cwd: &Path, args: &[&str], env: &[(&str, &str)]) -> CmdOutput {
    let mut cmd = Command::new(api_test_bin());
    cmd.current_dir(cwd)
        .env_remove("ACCESS_TOKEN")
        .env_remove("SERVICE_TOKEN")
        .env_remove("REST_TOKEN_NAME")
        .env_remove("GQL_JWT_NAME")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run api-test");

    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

fn write_str(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
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

fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("timeout");

    let header_bytes = read_until_headers_end(stream);
    let (method, path, headers, rest) = parse_headers_and_rest(header_bytes);

    let body = if let Some(cl) = header_value(&headers, "content-length") {
        let len = cl.parse::<usize>().unwrap_or(0);
        read_exact_bytes(stream, rest, len)
    } else {
        Vec::new()
    };

    RecordedRequest {
        method,
        path,
        headers,
        _body: body,
    }
}

fn write_response(stream: &mut TcpStream, status: u16, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "OK",
    };
    let mut resp = Vec::new();
    resp.extend_from_slice(format!("HTTP/1.1 {status} {reason}\r\n").as_bytes());
    resp.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    resp.extend_from_slice(b"Content-Type: application/json\r\n\r\n");
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
                    ("GET", "/health") => {
                        write_response(&mut stream, 200, br#"{"ok":true}"#);
                    }
                    ("GET", "/login") => {
                        write_response(
                            &mut stream,
                            200,
                            format!(r#"{{"accessToken":"{SECRET_TOKEN}"}}"#).as_bytes(),
                        );
                    }
                    ("GET", "/me") => {
                        let auth = header_value(&req.headers, "authorization").unwrap_or_default();
                        if auth == format!("Bearer {SECRET_TOKEN}") {
                            write_response(&mut stream, 200, br#"{"me":{"ok":true}}"#);
                        } else {
                            write_response(&mut stream, 401, br#"{"error":"unauthorized"}"#);
                        }
                    }
                    ("POST", "/graphql") => {
                        // Always succeed for this suite.
                        write_response(&mut stream, 200, br#"{"data":{"ok":true}}"#);
                    }
                    _ => write_response(&mut stream, 404, br#"{"error":"not_found"}"#),
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
fn run_e2e_suite_smoke_passes_and_does_not_leak_secrets() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".git")).expect("mkdir .git");

    let server = start_server();

    // REST requests
    write_str(
        &root.join("setup/rest/requests/health.request.json"),
        r#"{"method":"GET","path":"/health"}"#,
    );
    write_str(
        &root.join("setup/rest/requests/login.request.json"),
        r#"{"method":"GET","path":"/login"}"#,
    );
    write_str(
        &root.join("setup/rest/requests/me.request.json"),
        r#"{"method":"GET","path":"/me"}"#,
    );
    write_str(
        &root.join("setup/rest/requests/write.request.json"),
        r#"{"method":"POST","path":"/write"}"#,
    );

    // GraphQL ops
    write_str(
        &root.join("setup/graphql/operations/health.graphql"),
        "query Q { ok }\n",
    );
    write_str(
        &root.join("setup/graphql/operations/mutation.graphql"),
        "mutation M { write { ok } }\n",
    );

    // Suite file
    let smoke_suite_json = serde_json::json!({
      "version": 1,
      "name": "smoke",
      "defaults": {
        "env": "staging",
        "noHistory": true,
        "rest": { "url": server.base_url },
        "graphql": { "url": format!("{}/graphql", server.base_url) }
      },
      "cases": [
        { "id": "rest.health", "type": "rest", "tags": ["smoke"], "request": "setup/rest/requests/health.request.json" },
        { "id": "graphql.health", "type": "graphql", "tags": ["smoke"], "op": "setup/graphql/operations/health.graphql" },
        {
          "id": "rest_flow.me",
          "type": "rest-flow",
          "tags": ["smoke"],
          "loginRequest": "setup/rest/requests/login.request.json",
          "request": "setup/rest/requests/me.request.json",
          "tokenJq": ".accessToken"
        }
      ]
    });
    write_str(
        &root.join("tests/api/suites/smoke.suite.json"),
        &serde_json::to_string_pretty(&smoke_suite_json).expect("suite json"),
    );

    let out = run_api_test_with_env(
        root,
        &[
            "run",
            "--suite",
            "smoke",
            "--out",
            "out/smoke/results.json",
            "--junit",
            "out/smoke/junit.xml",
        ],
        &[("API_TEST_OUTPUT_DIR", "out/api-test-runner-smoke")],
    );

    assert_eq!(
        out.code,
        0,
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout_text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!stdout_text.contains(SECRET_TOKEN));

    let results_file = root.join("out/smoke/results.json");
    assert!(results_file.is_file());
    let file_bytes = std::fs::read(&results_file).expect("read results");
    assert_eq!(file_bytes, out.stdout);

    let junit_file = root.join("out/smoke/junit.xml");
    assert!(junit_file.is_file());
    let junit_text = std::fs::read_to_string(&junit_file).expect("read junit");
    assert!(junit_text.contains("<testsuite"));

    let results_json: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("results json");
    assert_eq!(results_json["summary"]["total"], 3);
    assert_eq!(results_json["summary"]["passed"], 3);
    assert_eq!(results_json["summary"]["failed"], 0);
    assert_eq!(results_json["summary"]["skipped"], 0);

    let output_dir_rel = results_json["outputDir"].as_str().unwrap_or("");
    assert!(!output_dir_rel.is_empty());
    let output_dir_abs = root.join(output_dir_rel);
    assert!(output_dir_abs.is_dir());

    // Ensure referenced artifacts exist and do not contain secrets.
    if let Some(cases) = results_json["cases"].as_array() {
        for c in cases {
            if let Some(stdout_rel) = c.get("stdoutFile").and_then(|v| v.as_str()) {
                let p = root.join(stdout_rel);
                assert!(p.is_file(), "missing stdout file: {}", p.display());
                let bytes = std::fs::read(&p).unwrap();
                assert!(!String::from_utf8_lossy(&bytes).contains(SECRET_TOKEN));
            }
            if let Some(stderr_rel) = c.get("stderrFile").and_then(|v| v.as_str()) {
                let p = root.join(stderr_rel);
                assert!(p.is_file(), "missing stderr file: {}", p.display());
                let bytes = std::fs::read(&p).unwrap();
                assert!(!String::from_utf8_lossy(&bytes).contains(SECRET_TOKEN));
            }
        }
    }

    let out2 = run_api_test(
        root,
        &[
            "summary",
            "--in",
            "out/smoke/results.json",
            "--out",
            "out/smoke/summary.md",
            "--slow",
            "5",
        ],
    );
    assert_eq!(out2.code, 0);
    let summary_file = root.join("out/smoke/summary.md");
    assert!(summary_file.is_file());
    let summary_text = std::fs::read_to_string(&summary_file).expect("read summary");
    assert!(summary_text.contains("API test summary"));

    // Sanity: rest-flow actually hit /me with Authorization.
    let reqs = server.requests.lock().expect("lock").clone();
    assert!(reqs.iter().any(|r| r.path == "/me"));
}

#[test]
fn run_e2e_suite_guardrails_fails_with_expected_messages() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".git")).expect("mkdir .git");

    let server = start_server();

    // REST requests
    write_str(
        &root.join("setup/rest/requests/write.request.json"),
        r#"{"method":"POST","path":"/write"}"#,
    );

    // GraphQL ops
    write_str(
        &root.join("setup/graphql/operations/mutation.graphql"),
        "mutation M { write { ok } }\n",
    );

    let guardrails_suite_json = serde_json::json!({
      "version": 1,
      "name": "guardrails",
      "defaults": {
        "env": "staging",
        "noHistory": true,
        "rest": { "url": server.base_url },
        "graphql": { "url": format!("{}/graphql", server.base_url) }
      },
      "cases": [
        { "id": "rest.write_no_allow", "type": "rest", "tags": ["guardrails"], "allowWrite": false, "request": "setup/rest/requests/write.request.json" },
        { "id": "rest.write_skip", "type": "rest", "tags": ["guardrails"], "allowWrite": true, "request": "setup/rest/requests/write.request.json" },
        { "id": "graphql.mutation_no_allow", "type": "graphql", "tags": ["guardrails"], "allowWrite": false, "op": "setup/graphql/operations/mutation.graphql" }
      ]
    });
    write_str(
        &root.join("tests/api/suites/guardrails.suite.json"),
        &serde_json::to_string_pretty(&guardrails_suite_json).expect("suite json"),
    );

    let out = run_api_test_with_env(
        root,
        &[
            "run",
            "--suite",
            "guardrails",
            "--out",
            "out/guardrails/results.json",
            "--junit",
            "out/guardrails/junit.xml",
        ],
        &[("API_TEST_OUTPUT_DIR", "out/api-test-runner-guardrails")],
    );

    assert_eq!(
        out.code,
        2,
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout_text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!stdout_text.contains(SECRET_TOKEN));

    let results_file = root.join("out/guardrails/results.json");
    assert!(results_file.is_file());
    let file_bytes = std::fs::read(&results_file).expect("read results");
    assert_eq!(file_bytes, out.stdout);

    let junit_file = root.join("out/guardrails/junit.xml");
    assert!(junit_file.is_file());

    let results_json: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("results json");
    assert_eq!(results_json["summary"]["total"], 3);
    assert_eq!(results_json["summary"]["passed"], 0);
    assert_eq!(results_json["summary"]["failed"], 2);
    assert_eq!(results_json["summary"]["skipped"], 1);

    let cases = results_json["cases"].as_array().expect("cases array");
    let mut by_id: std::collections::BTreeMap<String, serde_json::Value> =
        std::collections::BTreeMap::new();
    for c in cases {
        if let Some(id) = c.get("id").and_then(|v| v.as_str()) {
            by_id.insert(id.to_string(), c.clone());
        }
    }

    assert_eq!(by_id["rest.write_no_allow"]["status"], "failed");
    assert_eq!(
        by_id["rest.write_no_allow"]["message"],
        "write_capable_case_requires_allowWrite_true"
    );

    assert_eq!(by_id["graphql.mutation_no_allow"]["status"], "failed");
    assert_eq!(
        by_id["graphql.mutation_no_allow"]["message"],
        "mutation_case_requires_allowWrite_true"
    );

    assert_eq!(by_id["rest.write_skip"]["status"], "skipped");
    assert_eq!(by_id["rest.write_skip"]["message"], "write_cases_disabled");

    let output_dir_rel = results_json["outputDir"].as_str().unwrap_or("");
    assert!(!output_dir_rel.is_empty());
    let output_dir_abs = root.join(output_dir_rel);
    assert!(output_dir_abs.is_dir());
}
