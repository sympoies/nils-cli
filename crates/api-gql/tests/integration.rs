use std::path::Path;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{run_with, CmdOptions, CmdOutput};
use nils_test_support::fs::write_text;
use nils_test_support::http::{HttpResponse, RecordedRequest, TestServer};

const LOGIN_TOKEN_JWT: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.e30.sig";

fn api_gql_bin() -> std::path::PathBuf {
    resolve("api-gql")
}

fn run_api_gql(cwd: &Path, args: &[&str]) -> CmdOutput {
    let options = CmdOptions::default().with_cwd(cwd);
    run_with(&api_gql_bin(), args, &options)
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

fn start_server() -> TestServer {
    TestServer::new(
        |req: &RecordedRequest| match (req.method.as_str(), req.path.as_str()) {
            ("POST", "/graphql") => {
                if body_has_login_query(&req.body) {
                    let body =
                        format!(r#"{{"data":{{"login":{{"accessToken":"{LOGIN_TOKEN_JWT}"}}}}}}"#);
                    HttpResponse::new(200, body).with_header("Content-Type", "application/json")
                } else {
                    HttpResponse::new(200, r#"{"data":{"ok":true}}"#)
                        .with_header("Content-Type", "application/json")
                }
            }
            ("POST", "/graphql500") => HttpResponse::new(500, r#"{"error":"no"}"#)
                .with_header("Content-Type", "application/json"),
            _ => HttpResponse::new(404, "not found"),
        },
    )
    .expect("start test server")
}

#[test]
fn call_vars_min_limit_bumps_nested_limit_fields() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();

    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    let server = start_server();
    write_text(
        &setup_dir.join("endpoints.env"),
        &format!("GQL_URL_LOCAL={}/graphql\n", server.url()),
    );

    let op = root.join("ops/q.graphql");
    write_text(&op, "query Q($limit: Int) { ok }\n");

    let vars = root.join("ops/q.variables.json");
    write_text(
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

    assert_eq!(out.code, 0, "stderr={}", out.stderr_text());
    let stdout_json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout json");
    assert_eq!(stdout_json["data"]["ok"], true);

    let reqs = server.take_requests();
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
    write_text(&op, "query Q { ok }\n");

    let out = run_api_gql(
        root,
        &[
            "call",
            "--config-dir",
            "setup/graphql",
            "--url",
            &format!("{}/graphql500", server.url()),
            "q.graphql",
        ],
    );
    assert_eq!(out.code, 1);
    assert!(out
        .stderr_text()
        .contains("HTTP request failed with status"));
}

#[test]
fn call_jwt_profile_auto_login_injects_bearer_token() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();

    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    let server = start_server();
    write_text(
        &setup_dir.join("endpoints.env"),
        &format!("GQL_URL_LOCAL={}/graphql\n", server.url()),
    );
    write_text(&setup_dir.join("jwts.env"), "GQL_JWT_ADMIN=\n");

    // Selected profile is missing => auto-login uses login.<profile>.graphql
    write_text(
        &setup_dir.join("login.admin.graphql"),
        "query Login { login { accessToken } }\n",
    );

    let op = root.join("q.graphql");
    write_text(&op, "query Q { ok }\n");

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

    assert_eq!(out.code, 0, "stderr={}", out.stderr_text());

    let reqs = server.take_requests();
    assert_eq!(reqs.len(), 2);

    assert!(
        reqs[0].header_value("authorization").is_none(),
        "login should not include Authorization"
    );

    let auth = reqs[1].header_value("authorization").unwrap_or_default();
    assert_eq!(auth, format!("Bearer {LOGIN_TOKEN_JWT}"));
}

#[test]
fn report_blocks_empty_by_default() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();

    let op = root.join("q.graphql");
    write_text(&op, "query Q { ok }\n");

    let resp = root.join("resp.json");
    write_text(&resp, r#"{"data":{"pageInfo":{"hasNextPage":false}}}"#);

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
    assert!(out.stderr_text().contains("no data records"));
}

#[test]
fn report_allow_empty_writes_report() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();

    let op = root.join("q.graphql");
    write_text(&op, "query Q { ok }\n");

    let resp = root.join("resp.json");
    write_text(&resp, r#"{"data":{"pageInfo":{"hasNextPage":false}}}"#);

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
    let report_path = out.stdout_text().trim().to_string();
    assert!(!report_path.is_empty());
    assert!(Path::new(&report_path).is_file());
    let contents = std::fs::read_to_string(&report_path).expect("read report");
    assert!(contents.contains("# API Test Report"));
}
