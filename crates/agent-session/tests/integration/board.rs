//! Session board v1 (`docs/specs/session-board-v1.md`): the daemon's local
//! projection, the closed-session ledger, and the `machine` label on
//! `agent-session list`.

use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

const TOKEN: &str = "board-operator-token";
const MACHINE: &str = "board-host";

/// Variables that change the board or machine label and must never leak in
/// from the environment the suite runs in.
const BOARD_ENV: [&str; 3] = [
    "AGENT_SESSION_BOARD",
    "AGENT_SESSION_MACHINE",
    "AGENT_SESSION_HOST",
];

fn run(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let options = CmdOptions::new()
        .with_cwd(dir)
        .without_ambient_managed_session_env()
        .with_env_remove_many(&BOARD_ENV)
        .with_envs(envs);
    run_resolved("agent-session", args, &options)
}

fn write_record(state_dir: &Path, id: &str, cwd: &Path, updated_at: &str) {
    let dir = state_dir.join("sessions").join(id);
    fs::create_dir_all(&dir).expect("session dir");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).expect("session dir mode");
    let record = json!({
        "schema_version": "agent-session.session.v1",
        "id": id,
        "agent": "claude",
        "mode": "interactive",
        "title": format!("Title {id}"),
        "cwd": cwd.to_string_lossy(),
        "tmux_session": format!("agent-{id}"),
        "prompt_file": "/secret/prompt.txt",
        "log_file": "/secret/session.log",
        "created_at": "2030-01-01T00:00:00Z",
        "updated_at": updated_at,
    });
    fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(&record).expect("record json"),
    )
    .expect("write record");
}

fn fake_tmux(root: &Path) -> PathBuf {
    let bin = root.join("tmux");
    fs::write(
        &bin,
        "#!/bin/sh\nprintf '%s\\n' 'no server running on /tmp/tmux-test/default' >&2\nexit 1\n",
    )
    .expect("fake tmux");
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).expect("fake tmux mode");
    bin
}

/// Kills its serve on drop, so a failed assertion cannot orphan a daemon.
struct Serve {
    child: Child,
    addr: SocketAddr,
}

impl Serve {
    fn spawn(
        root: &Path,
        state_dir: &Path,
        home: &Path,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Self {
        // Bind port 0 and read the published endpoint, so no reserved port is
        // released for another concurrently running test to take.
        let endpoint = state_dir.join("coordination/daemon-endpoint.json");
        let _ = fs::remove_file(&endpoint);
        let stderr_path = root.join("serve.stderr");
        let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
        command
            .arg("serve")
            .arg("--bind")
            .arg("127.0.0.1:0")
            .arg("--state-dir")
            .arg(state_dir)
            .arg("--machine")
            .arg(MACHINE)
            .args(args)
            .env("HOME", home)
            .env("AGENT_SESSION_TOKEN", TOKEN)
            .env("AGENT_SESSION_TMUX_BIN", fake_tmux(root))
            .env_remove("XDG_STATE_HOME")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(&stderr_path).expect("serve stderr"),
            ));
        nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
        for key in BOARD_ENV {
            command.env_remove(key);
        }
        for key in [
            "AGENT_SESSION_RELAY_URL",
            "AGENT_SESSION_RELAY_TOKEN",
            "AGENT_SESSION_RELAY_INGRESS_TOKEN",
        ] {
            command.env_remove(key);
        }
        command.envs(env.iter().copied());
        let mut child = command.spawn().expect("spawn serve");
        let deadline = Instant::now() + Duration::from_secs(15);
        let addr = loop {
            let published = fs::read(&endpoint).ok().and_then(|raw| {
                let url = serde_json::from_slice::<Value>(&raw).ok()?["url"]
                    .as_str()?
                    .to_string();
                url.strip_prefix("http://")?.parse::<SocketAddr>().ok()
            });
            if let Some(addr) = published {
                break addr;
            }
            if let Some(status) = child.try_wait().expect("poll serve") {
                panic!(
                    "serve exited before listening: {status}; {}",
                    fs::read_to_string(&stderr_path).unwrap_or_default()
                );
            }
            assert!(
                Instant::now() < deadline,
                "serve did not publish its endpoint"
            );
            thread::sleep(Duration::from_millis(20));
        };
        Self { child, addr }
    }

    fn get(&self, path: &str, headers: &[(&str, &str)]) -> (u16, Value) {
        let client = reqwest::blocking::Client::new();
        let mut request = client.get(format!("http://{}{path}", self.addr));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = request.send().expect("serve request");
        let status = response.status().as_u16();
        let body = response.json::<Value>().expect("json body");
        (status, body)
    }

    fn get_operator(&self, path: &str) -> (u16, Value) {
        self.get(path, &[("Authorization", &format!("Bearer {TOKEN}"))])
    }

    fn post_operator(&self, path: &str, body: &Value) -> (u16, Value) {
        let response = reqwest::blocking::Client::new()
            .post(format!("http://{}{path}", self.addr))
            .header("Authorization", format!("Bearer {TOKEN}"))
            .json(body)
            .send()
            .expect("serve request");
        let status = response.status().as_u16();
        (status, response.json::<Value>().expect("json body"))
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        if self.child.try_wait().expect("poll serve").is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    state_dir: PathBuf,
    home: PathBuf,
}

impl Fixture {
    /// A record the board must skip and count; it also fails `GET /sessions`.
    fn corrupt_record(&self, present: bool) {
        let dir = self.state_dir.join("sessions").join(CORRUPT_ID);
        if present {
            fs::create_dir_all(&dir).expect("corrupt dir");
            fs::write(dir.join("session.json"), b"{").expect("corrupt record");
        } else {
            fs::remove_dir_all(&dir).expect("drop corrupt record");
        }
    }
}

const CORRUPT_ID: &str = "20300101-000000-c";

/// Two stopped records, one inside HOME and one outside it.
fn fixture() -> Fixture {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path().to_path_buf();
    let state_dir = root.join("state");
    let home = root.join("home");
    let inside = home.join("Project/board-repo");
    let outside = root.join("elsewhere/outside-repo");
    fs::create_dir_all(&inside).expect("inside cwd");
    fs::create_dir_all(&outside).expect("outside cwd");
    fs::create_dir_all(state_dir.join("sessions")).expect("sessions root");
    for dir in [&state_dir, &state_dir.join("sessions")] {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).expect("state mode");
    }
    write_record(
        &state_dir,
        "20300101-000000-b",
        &inside,
        "2030-01-01T00:04:00Z",
    );
    write_record(
        &state_dir,
        "20300101-000000-a",
        &outside,
        "2030-01-01T00:02:00Z",
    );
    Fixture {
        _tmp: tmp,
        root,
        state_dir,
        home,
    }
}

#[test]
fn list_json_records_carry_the_machine_label() {
    let fixture = fixture();
    let state = fixture.state_dir.to_string_lossy().to_string();
    let tmux = fake_tmux(&fixture.root).to_string_lossy().to_string();
    let explicit = run(
        &fixture.root,
        &["--state-dir", &state, "list", "--format", "json"],
        &[
            ("AGENT_SESSION_TMUX_BIN", tmux.as_str()),
            ("AGENT_SESSION_MACHINE", MACHINE),
            ("AGENT_SESSION_HOST", "ssh-alias"),
        ],
    );
    assert_eq!(explicit.code, 0, "stderr={}", explicit.stderr_text());
    let records = explicit.stdout_json()["data"].clone();
    let records = records.as_array().expect("list data");
    assert_eq!(records.len(), 2);
    for record in records {
        assert_eq!(record["machine"], MACHINE, "{record}");
    }

    // Without AGENT_SESSION_MACHINE the label follows serve's next fallback,
    // the --host / AGENT_SESSION_HOST identity.
    let host = run(
        &fixture.root,
        &["--state-dir", &state, "list", "--format", "json"],
        &[
            ("AGENT_SESSION_TMUX_BIN", tmux.as_str()),
            ("AGENT_SESSION_HOST", "ssh-alias"),
        ],
    );
    assert_eq!(host.code, 0, "stderr={}", host.stderr_text());
    assert_eq!(host.stdout_json()["data"][0]["machine"], "ssh-alias");
}

#[test]
fn board_snapshot_is_opt_in_and_operator_authenticated() {
    let fixture = fixture();

    // Disabled (default): the route answers board-disabled before any read,
    // regardless of credentials, and GET /sessions keeps its shape.
    let disabled = Serve::spawn(&fixture.root, &fixture.state_dir, &fixture.home, &[], &[]);
    for headers in [vec![], vec![("Authorization", format!("Bearer {TOKEN}"))]] {
        let headers: Vec<(&str, &str)> = headers
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        let (status, body) = disabled.get("/board/v1", &headers);
        assert_eq!(status, 404, "{body}");
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], "board-disabled");
    }
    let (status, sessions_disabled) = disabled.get("/sessions", &[]);
    assert_eq!(status, 200, "{sessions_disabled}");
    drop(disabled);

    for (args, env) in [
        (vec!["--board"], vec![]),
        (vec![], vec![("AGENT_SESSION_BOARD", "1")]),
    ] {
        let serve = Serve::spawn(
            &fixture.root,
            &fixture.state_dir,
            &fixture.home,
            &args,
            &env,
        );

        // Enabling the board leaves the open session list unchanged.
        let (_, sessions_enabled) = serve.get("/sessions", &[]);
        assert_eq!(
            sessions_enabled["data"]["sessions"],
            sessions_disabled["data"]["sessions"]
        );

        fixture.corrupt_record(true);
        // The session list still fails closed on the corrupt record that the
        // board skips and counts below.
        let (status, body) = serve.get("/sessions", &[]);
        assert_ne!(status, 200, "{body}");
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], "session-json-invalid");

        let (status, body) = serve.get("/board/v1", &[]);
        assert_eq!(status, 401, "{body}");
        assert_eq!(body["error"]["code"], "unauthorized");
        // A session capability is not operator authority.
        let (status, body) = serve.get(
            "/board/v1",
            &[
                ("Authorization", "Bearer session-capability"),
                ("X-Agent-Session-Capability", "session-capability"),
            ],
        );
        assert_eq!(status, 401, "{body}");

        let (status, body) = serve.get_operator("/board/v1");
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["ok"], true);
        assert_eq!(body["data"]["machine"], MACHINE);
        let board = &body["data"]["board"];
        assert_eq!(board["schema_version"], "agent-session.board.v1");
        assert_eq!(board["record_schema"], "agent-session.board-record.v1");
        assert_eq!(board["machine"], MACHINE);
        assert_eq!(
            board["extensions"],
            json!(["lineage.v1", "work.v1", "programs.v1"])
        );
        assert!(board["generated_at"].as_str().is_some(), "{board}");
        assert_eq!(board["skipped_count"], 1, "{board}");
        let records = board["records"].as_array().expect("records");
        let ids: Vec<&str> = records
            .iter()
            .map(|record| record["session_id"].as_str().expect("id"))
            .collect();
        assert_eq!(ids, vec!["20300101-000000-a", "20300101-000000-b"]);

        let outside = &records[0];
        assert_eq!(outside["cwd"], Value::Null);
        assert_eq!(outside["repo_name"], "outside-repo");
        let inside = &records[1];
        assert_eq!(
            inside,
            &json!({
                "machine": MACHINE,
                "session_id": "20300101-000000-b",
                "session_incarnation": null,
                "messaging_supported": false,
                "repo_name": "board-repo",
                "cwd": "~/Project/board-repo",
                "provider": "claude",
                "agent_profile": null,
                "model": null,
                "reasoning_effort": null,
                "title": "Title 20300101-000000-b",
                "title_state": null,
                "turn_state": null,
                "state": "stopped",
                "runtime_status": inside["runtime_status"],
                "created_at": "2030-01-01T00:00:00Z",
                "updated_at": "2030-01-01T00:04:00Z",
                "closed_at": null,
                "close_reason": null,
                "summary": null,
                "role": null,
                "lineage": null,
                "work": null,
            })
        );
        assert!(
            ["stopped", "missing", "unknown"]
                .contains(&inside["runtime_status"].as_str().expect("runtime status")),
            "{inside}"
        );
        fixture.corrupt_record(false);
    }
}

/// A record whose runtime is proven never launched, so ordinary deletion
/// needs no tmux runtime to terminate.
fn write_never_launched_record(state_dir: &Path, id: &str, cwd: &Path) {
    write_record(state_dir, id, cwd, "2030-01-01T00:04:00Z");
    let path = state_dir.join("sessions").join(id).join("session.json");
    let mut record: Value =
        serde_json::from_slice(&fs::read(&path).expect("record")).expect("json");
    record["runtime"] = json!({
        "kind": "tmux",
        "tmux_session": format!("agent-{id}"),
        "generation": 1,
        "started_at": "2030-01-01T00:00:00Z",
        "launch_id": "never-launched-fixture",
    });
    record["tmux_runtime_never_launched"] = json!("never-launched-fixture");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&record).expect("record json"),
    )
    .expect("record");
}

fn ledger_entries(state_dir: &Path) -> Vec<Value> {
    let ledger: Value = serde_json::from_slice(
        &fs::read(state_dir.join("board/closed-ledger.json")).expect("closed ledger"),
    )
    .expect("ledger json");
    ledger["entries"].as_array().expect("entries").clone()
}

fn cli_delete(fixture: &Fixture, id: &str) -> CmdOutput {
    let state = fixture.state_dir.to_string_lossy().to_string();
    let tmux = fake_tmux(&fixture.root).to_string_lossy().to_string();
    let home = fixture.home.to_string_lossy().to_string();
    run(
        &fixture.root,
        &["--state-dir", &state, "delete", id, "--format", "json"],
        &[
            ("AGENT_SESSION_TMUX_BIN", tmux.as_str()),
            ("HOME", home.as_str()),
        ],
    )
}

#[test]
fn a_cli_delete_with_the_board_disabled_is_served_as_one_closed_record() {
    let fixture = fixture();
    let id = "20300101-000000-d";
    write_never_launched_record(
        &fixture.state_dir,
        id,
        &fixture.home.join("Project/closed-repo"),
    );

    let deleted = cli_delete(&fixture, id);
    assert_eq!(deleted.code, 0, "stderr={}", deleted.stderr_text());
    let entries = ledger_entries(&fixture.state_dir);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["seq"], 1);
    // The CLI process cannot know the serve identity, so nothing is stored.
    assert_eq!(entries[0]["record"].get("machine"), None);

    let disabled = Serve::spawn(&fixture.root, &fixture.state_dir, &fixture.home, &[], &[]);
    let (status, body) = disabled.get_operator("/board/closed/v1");
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "board-disabled");
    drop(disabled);

    let serve = Serve::spawn(
        &fixture.root,
        &fixture.state_dir,
        &fixture.home,
        &["--board"],
        &[],
    );
    let (status, body) = serve.get("/board/closed/v1", &[]);
    assert_eq!(status, 401, "{body}");
    let (status, body) = serve.get(
        "/board/closed/v1",
        &[
            ("Authorization", "Bearer session-capability"),
            ("X-Agent-Session-Capability", "session-capability"),
        ],
    );
    assert_eq!(status, 401, "{body}");

    let (status, body) = serve.get_operator("/board/closed/v1");
    assert_eq!(status, 200, "{body}");
    let closed = &body["data"]["board_closed"];
    assert_eq!(body["data"]["machine"], MACHINE);
    assert_eq!(closed["schema_version"], "agent-session.board-closed.v1");
    assert_eq!(closed["record_schema"], "agent-session.board-record.v1");
    assert_eq!(closed["machine"], MACHINE);
    let entries = closed["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "{closed}");
    let record = &entries[0]["record"];
    assert_eq!(closed["next_cursor"], entries[0]["cursor"]);
    assert!(record["closed_at"].as_str().is_some(), "{record}");
    assert_eq!(
        record,
        &json!({
            "machine": MACHINE,
            "session_id": id,
            "session_incarnation": "never-launched-fixture",
            "messaging_supported": false,
            "repo_name": "closed-repo",
            "cwd": "~/Project/closed-repo",
            "provider": "claude",
            "agent_profile": null,
            "model": null,
            "reasoning_effort": null,
            "title": format!("Title {id}"),
            "title_state": null,
            "turn_state": record["turn_state"],
            "state": "closed",
            "runtime_status": null,
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": record["updated_at"],
            "closed_at": record["closed_at"],
            "close_reason": "deleted",
            "summary": null,
            "role": null,
            "lineage": null,
            "work": null,
        })
    );

    // The snapshot names the ledger head; nothing is newer than it.
    let (status, snapshot) = serve.get_operator("/board/v1");
    assert_eq!(status, 200, "{snapshot}");
    let head = snapshot["data"]["board"]["ledger_cursor"]
        .as_str()
        .expect("ledger cursor")
        .to_string();
    assert_eq!(head, entries[0]["cursor"].as_str().expect("cursor"));
    let (status, tail) = serve.get_operator(&format!("/board/closed/v1?since={head}"));
    assert_eq!(status, 200, "{tail}");
    assert_eq!(tail["data"]["board_closed"]["entries"], json!([]));
    assert_eq!(tail["data"]["board_closed"]["next_cursor"], head);

    let (status, body) = serve.get_operator("/board/closed/v1?since=not-a-cursor");
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"]["code"], "board-cursor-invalid");
    let expired = "v1:00000000-0000-4000-8000-000000000000:0";
    let (status, body) = serve.get_operator(&format!("/board/closed/v1?since={expired}"));
    assert_eq!(status, 410, "{body}");
    assert_eq!(body["error"]["code"], "board-cursor-expired");
}

#[test]
fn record_only_removal_appends_a_deleted_entry() {
    let fixture = fixture();
    let id = "20300101-000000-b";
    let serve = Serve::spawn(
        &fixture.root,
        &fixture.state_dir,
        &fixture.home,
        &["--board"],
        &[],
    );
    let preview_path = format!(
        "/sessions/{id}/maintenance?operation=delete&schema_version=agent-session.session-maintenance.v2"
    );
    let (status, preview) = serve.get_operator(&preview_path);
    assert_eq!(status, 200, "{preview}");
    let preview = &preview["data"]["maintenance"];
    assert!(
        preview["actions"]
            .as_array()
            .expect("actions")
            .iter()
            .any(|action| action["id"] == "remove_console_record"),
        "{preview}"
    );
    let (status, body) = serve.post_operator(
        &format!("/sessions/{id}/maintenance/actions"),
        &json!({
            "schema_version": "agent-session.session-maintenance.v2",
            "operation": "delete",
            "action": "remove_console_record",
            "expected_session_incarnation": preview["session_incarnation"],
            "expected_session_generation": preview["session_generation"],
            "expected_preview_digest": preview["preview_digest"],
            "confirmed": true,
        }),
    );
    assert_eq!(status, 200, "{body}");
    let entries = ledger_entries(&fixture.state_dir);
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["record"]["session_id"], id);
    assert_eq!(entries[0]["record"]["close_reason"], "deleted");
}

#[test]
fn a_ledger_failure_never_fails_the_delete_and_a_runtime_exit_writes_nothing() {
    let fixture = fixture();
    // A stopped record that stays behind never becomes a closed entry.
    let serve = Serve::spawn(
        &fixture.root,
        &fixture.state_dir,
        &fixture.home,
        &["--board"],
        &[],
    );
    let (status, snapshot) = serve.get_operator("/board/v1");
    assert_eq!(status, 200, "{snapshot}");
    assert_eq!(
        snapshot["data"]["board"]["records"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let (status, closed) = serve.get_operator("/board/closed/v1");
    assert_eq!(status, 200, "{closed}");
    assert_eq!(closed["data"]["board_closed"]["entries"], json!([]));
    drop(serve);

    // An untrusted ledger store fails the append, not the deletion.
    let elsewhere = fixture.root.join("elsewhere-board");
    fs::create_dir_all(&elsewhere).expect("elsewhere");
    fs::remove_dir_all(fixture.state_dir.join("board")).expect("drop ledger store");
    std::os::unix::fs::symlink(&elsewhere, fixture.state_dir.join("board")).expect("symlink");
    let id = "20300101-000000-d";
    write_never_launched_record(&fixture.state_dir, id, &fixture.home.join("Project/x"));
    let deleted = cli_delete(&fixture, id);
    assert_eq!(deleted.code, 0, "stderr={}", deleted.stderr_text());
    assert!(!fixture.state_dir.join("sessions").join(id).exists());
    assert!(!elsewhere.join("closed-ledger.json").exists());

    let serve = Serve::spawn(
        &fixture.root,
        &fixture.state_dir,
        &fixture.home,
        &["--board"],
        &[],
    );
    for path in ["/board/v1", "/board/closed/v1"] {
        let (status, body) = serve.get_operator(path);
        assert_eq!(status, 503, "{path}: {body}");
        assert_eq!(body["error"]["code"], "board-ledger-unavailable");
    }
}

fn cli_board(fixture: &Fixture, args: &[&str]) -> CmdOutput {
    let state = fixture.state_dir.to_string_lossy().to_string();
    let tmux = fake_tmux(&fixture.root).to_string_lossy().to_string();
    let home = fixture.home.to_string_lossy().to_string();
    let mut argv = vec!["--state-dir", state.as_str(), "board"];
    argv.extend_from_slice(args);
    run(
        &fixture.root,
        &argv,
        &[
            ("AGENT_SESSION_TMUX_BIN", tmux.as_str()),
            ("HOME", home.as_str()),
            ("AGENT_SESSION_MACHINE", MACHINE),
        ],
    )
}

fn board_ids(output: &CmdOutput) -> Vec<String> {
    output.stdout_json()["data"]["board"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["session_id"].as_str().expect("id").to_string())
        .collect()
}

fn ago(seconds: i64) -> String {
    jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() - seconds)
        .expect("timestamp")
        .to_string()
}

const HOUR: i64 = 60 * 60;
const DAY: i64 = 24 * HOUR;

/// Stopped records updated 1 hour, 2 hours, 2 days, and 8 days ago (all
/// relative to now, so the window filter is exercised and the test does not
/// age), plus one closed record from a real CLI delete.
fn board_fixture() -> Fixture {
    let fixture = fixture();
    write_record(
        &fixture.state_dir,
        "20300101-000000-b",
        &fixture.home.join("Project/board-repo"),
        &ago(HOUR),
    );
    write_record(
        &fixture.state_dir,
        "20300101-000000-a",
        &fixture.root.join("elsewhere/outside-repo"),
        &ago(2 * HOUR),
    );
    write_record(
        &fixture.state_dir,
        "20300101-000000-e",
        &fixture.home.join("Project/two-days"),
        &ago(2 * DAY),
    );
    write_record(
        &fixture.state_dir,
        "20300101-000000-f",
        &fixture.home.join("Project/eight-days"),
        &ago(8 * DAY),
    );
    write_never_launched_record(
        &fixture.state_dir,
        "20300101-000000-d",
        &fixture.home.join("Project/closed-repo"),
    );
    let deleted = cli_delete(&fixture, "20300101-000000-d");
    assert_eq!(deleted.code, 0, "stderr={}", deleted.stderr_text());
    fixture
}

#[test]
fn board_local_mode_emits_a_board_view_of_this_machine() {
    let fixture = board_fixture();
    let output = cli_board(&fixture, &["--format", "json"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let envelope = output.stdout_json();
    assert_eq!(envelope["schema_version"], "cli.agent-session.board.v1");
    assert_eq!(envelope["ok"], true);
    assert_eq!(envelope["data"]["mode"], "local");
    let board = &envelope["data"]["board"];
    assert_eq!(board["schema_version"], "agent-session.board-view.v1");
    assert_eq!(board["record_schema"], "agent-session.board-record.v1");
    assert_eq!(board["retention"], "7d");
    assert_eq!(board["since_capped"], false);
    assert_eq!(board["truncated"], false);
    let seconds = |value: &Value| {
        value
            .as_str()
            .expect("timestamp")
            .parse::<jiff::Timestamp>()
            .expect("rfc3339")
            .as_second()
    };
    // The default window is the 7-day retention.
    assert_eq!(
        seconds(&board["generated_at"]) - seconds(&board["effective_since"]),
        7 * DAY
    );
    assert_eq!(
        board["machines"],
        json!([{"machine": MACHINE, "available": true, "last_seen_at": board["generated_at"]}])
    );
    // Stopped rows newest first by updated_at, then closed rows; the row
    // older than the 7-day retention is outside the default window.
    assert_eq!(
        board_ids(&output),
        vec![
            "20300101-000000-b",
            "20300101-000000-a",
            "20300101-000000-e",
            "20300101-000000-d"
        ]
    );
    for record in board["records"].as_array().expect("records") {
        assert_eq!(record["machine"], MACHINE, "{record}");
        assert_eq!(record["messaging_supported"], false, "{record}");
        assert_eq!(record.get("console_owner"), None, "{record}");
    }
    assert_eq!(board["records"][3]["state"], "closed");
    assert_eq!(board["records"][3]["close_reason"], "deleted");
    assert_eq!(board["records"][0]["cwd"], "~/Project/board-repo");
}

#[test]
fn board_local_mode_applies_the_query_filters() {
    let fixture = board_fixture();
    let ids = |args: &[&str]| {
        let mut argv = args.to_vec();
        argv.extend_from_slice(&["--format", "json"]);
        let output = cli_board(&fixture, &argv);
        assert_eq!(output.code, 0, "{args:?}: stderr={}", output.stderr_text());
        board_ids(&output)
    };
    assert_eq!(ids(&["--state", "closed"]), vec!["20300101-000000-d"]);
    assert_eq!(
        ids(&["--state", "stopped"]),
        vec![
            "20300101-000000-b",
            "20300101-000000-a",
            "20300101-000000-e"
        ]
    );
    // Stopped rows are windowed by updated_at, closed rows by closed_at.
    assert_eq!(
        ids(&["--since", "1d"]),
        vec![
            "20300101-000000-b",
            "20300101-000000-a",
            "20300101-000000-d"
        ]
    );
    assert_eq!(
        ids(&["--since", "90m"]),
        vec!["20300101-000000-b", "20300101-000000-d"]
    );
    assert!(ids(&["--since", "3d"]).contains(&"20300101-000000-e".to_string()));
    assert_eq!(ids(&["--state", "live"]), Vec::<String>::new());
    assert_eq!(ids(&["--repo", "board-repo"]), vec!["20300101-000000-b"]);
    assert_eq!(ids(&["--repo", "Board-repo"]), Vec::<String>::new());
    assert_eq!(ids(&["--machine", "elsewhere"]), Vec::<String>::new());
    assert_eq!(ids(&["--machine", MACHINE]).len(), 4);

    // A window longer than the 7-day ledger bound is clamped, not rejected.
    let capped = cli_board(&fixture, &["--since", "2w", "--format", "json"]);
    assert_eq!(capped.code, 0, "stderr={}", capped.stderr_text());
    let board = capped.stdout_json()["data"]["board"].clone();
    assert_eq!(board["since_capped"], true);
    assert_eq!(board["retention"], "7d");
    // Clamped to 7 days: the 8-day-old row stays outside the window.
    assert!(
        !board["records"]
            .as_array()
            .expect("records")
            .iter()
            .any(|record| record["session_id"] == "20300101-000000-f"),
        "{board}"
    );
    // The closed row was closed just now, so a short window still keeps it.
    assert_eq!(ids(&["--since", "30m", "--state", "closed"]).len(), 1);
    let within = cli_board(&fixture, &["--since", "3d", "--format", "json"]);
    assert_eq!(within.stdout_json()["data"]["board"]["since_capped"], false);

    for args in [
        vec!["--since", "3x"],
        vec!["--since", "0d"],
        vec!["--since", "d"],
        vec!["--state", "running"],
    ] {
        let mut argv = args.clone();
        argv.extend_from_slice(&["--format", "json"]);
        let output = cli_board(&fixture, &argv);
        assert_eq!(output.code, 64, "{args:?}: {}", output.stdout_text());
        let envelope = output.stdout_json();
        assert_eq!(envelope["schema_version"], "cli.agent-session.board.v1");
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["error"]["code"], "board-query-invalid", "{args:?}");
    }
}

#[test]
fn board_local_mode_text_output_is_one_line_per_record() {
    let fixture = board_fixture();
    let output = cli_board(&fixture, &[]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let text = output.stdout_text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        vec![
            "mode: local",
            "stopped  board-host  20300101-000000-b  board-repo  -  -  Title 20300101-000000-b",
            "stopped  board-host  20300101-000000-a  outside-repo  -  -  Title 20300101-000000-a",
            "stopped  board-host  20300101-000000-e  two-days  -  -  Title 20300101-000000-e",
            "closed  board-host  20300101-000000-d  closed-repo  -  -  Title 20300101-000000-d",
        ],
        "{text}"
    );
}

fn session_ref(id: &str) -> Value {
    json!({
        "machine": MACHINE,
        "session_id": id,
        "session_created_at": "2030-01-01T00:00:00Z",
    })
}

/// Give the stored record `id` a lineage under `parent` in the tree of `root`.
fn set_lineage(fixture: &Fixture, id: &str, parent: Option<&str>, root: &str, role: Option<&str>) {
    let path = fixture
        .state_dir
        .join("sessions")
        .join(id)
        .join("session.json");
    let mut record: Value =
        serde_json::from_slice(&fs::read(&path).expect("record")).expect("json");
    record["lineage"] = json!({
        "schema_version": "agent-session.session-lineage.v1",
        "machine": MACHINE,
        "parent": parent.map(session_ref),
        "root": session_ref(root),
        "depth": u32::from(parent.is_some()),
        "starter": {"kind": if parent.is_some() { "session" } else { "operator" }, "via": "cli"},
        "budget": null,
    });
    if let Some(role) = role {
        record["role"] = json!(role);
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(&record).expect("record json"),
    )
    .expect("record");
}

#[test]
fn the_board_carries_role_lineage_work_and_the_tree_filter() {
    let fixture = fixture();
    let cwd = fixture.home.join("Project/board-repo");
    for id in ["tree-root", "tree-child", "tree-orphan", "tree-gone"] {
        write_never_launched_record(&fixture.state_dir, id, &cwd);
    }
    set_lineage(
        &fixture,
        "tree-root",
        None,
        "tree-root",
        Some("coordinator"),
    );
    set_lineage(&fixture, "tree-child", Some("tree-root"), "tree-root", None);
    set_lineage(&fixture, "tree-gone", Some("tree-root"), "tree-root", None);
    set_lineage(
        &fixture,
        "tree-orphan",
        Some("tree-gone"),
        "tree-root",
        None,
    );
    // The orphan's parent closes; the orphan keeps its root.
    let state = fixture.state_dir.to_string_lossy().to_string();
    let tmux = fake_tmux(&fixture.root).to_string_lossy().to_string();
    let home = fixture.home.to_string_lossy().to_string();
    let deleted = run(
        &fixture.root,
        &[
            "--state-dir",
            &state,
            "delete",
            "tree-gone",
            "--orphan-children",
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_TMUX_BIN", tmux.as_str()),
            ("HOME", home.as_str()),
        ],
    );
    assert_eq!(deleted.code, 0, "stderr={}", deleted.stderr_text());

    let output = cli_board(&fixture, &["--format", "json", "--root", "tree-root"]);
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let board = output.stdout_json()["data"]["board"].clone();
    assert_eq!(board["extensions"], json!(["lineage.v1", "work.v1"]));
    let records = board["records"].as_array().expect("records");
    let record = |id: &str| {
        records
            .iter()
            .find(|record| record["session_id"] == id)
            .unwrap_or_else(|| panic!("{id} on the board: {board}"))
    };
    assert_eq!(records.len(), 4, "{board}");
    assert_eq!(record("tree-root")["role"], "coordinator");
    assert_eq!(record("tree-root")["lineage"]["parent"], Value::Null);
    assert_eq!(
        record("tree-root")["subtree"],
        json!({"live": 0, "stopped": 2})
    );
    assert_eq!(
        record("tree-child")["lineage"]["parent"],
        session_ref("tree-root")
    );
    assert_eq!(record("tree-child")["role"], Value::Null);
    assert_eq!(record("tree-child")["work"], Value::Null);
    assert_eq!(
        record("tree-orphan")["lineage"]["root"],
        session_ref("tree-root")
    );
    assert_eq!(record("tree-orphan")["orphaned"], true);
    assert_eq!(record("tree-child")["orphaned"], false);
    // The closed parent keeps its lineage in the ledger entry.
    assert_eq!(record("tree-gone")["state"], "closed");
    assert_eq!(record("tree-gone")["lineage"]["depth"], 1);
    assert_eq!(record("tree-gone")["orphaned"], false);

    // Records outside the tree are not selected.
    let output = cli_board(
        &fixture,
        &["--format", "json", "--root", "20300101-000000-b"],
    );
    assert_eq!(board_ids(&output), vec!["20300101-000000-b"]);
    let output = cli_board(&fixture, &["--format", "json", "--root", "nobody"]);
    assert_eq!(board_ids(&output), Vec::<String>::new());

    // The daemon snapshot carries the same members and its envelope names
    // the extensions.
    let serve = Serve::spawn(
        &fixture.root,
        &fixture.state_dir,
        &fixture.home,
        &["--board"],
        &[],
    );
    let auth = format!("Bearer {TOKEN}");
    let (status, body) = serve.get("/board/v1", &[("Authorization", auth.as_str())]);
    assert_eq!(status, 200, "{body}");
    let snapshot = &body["data"]["board"];
    assert_eq!(
        snapshot["extensions"],
        json!(["lineage.v1", "work.v1", "programs.v1"])
    );
    let root = snapshot["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| record["session_id"] == "tree-root")
        .expect("root record");
    assert_eq!(root["role"], "coordinator");
    assert_eq!(root["lineage"]["root"], session_ref("tree-root"));
    assert!(
        root.get("orphaned").is_none(),
        "daemons never annotate: {root}"
    );
    let (status, body) = serve.get("/board/closed/v1", &[("Authorization", auth.as_str())]);
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["data"]["board_closed"]["extensions"],
        json!(["lineage.v1", "work.v1"])
    );
}

fn fake_forge_cli(root: &Path) -> PathBuf {
    let bin = root.join("forge-cli");
    fs::write(
        &bin,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *\"show o/program#44 \"*) ;;\n\
         *) exit 1 ;;\n\
         esac\n\
         printf '%s' '{\"schema_version\":\"cli.forge-cli.issue.tracker.show.v1\",\"ok\":true,\"data\":{\"title\":\"Program tracker\",\"state\":\"open\",\"url\":\"https://example.test/o/program/issues/44\",\"rows\":[{\"id\":\"A1\",\"title\":\"Lane\",\"reference\":\"o/repo#1\",\"done\":false,\"phase\":null,\"after\":[],\"notes\":null,\"line\":3}]}}'\n",
    )
    .expect("fake forge-cli");
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).expect("fake forge-cli mode");
    bin
}

#[test]
fn the_programs_route_serves_the_lanes_of_the_programs_sessions_name() {
    let fixture = fixture();
    let cwd = fixture.home.join("Project/board-repo");
    for id in ["with-program", "with-unreadable-program"] {
        write_never_launched_record(&fixture.state_dir, id, &cwd);
        let path = fixture
            .state_dir
            .join("sessions")
            .join(id)
            .join("session.json");
        let mut record: Value =
            serde_json::from_slice(&fs::read(&path).expect("record")).expect("json");
        let repository = if id == "with-program" {
            "o/program"
        } else {
            "o/unreadable"
        };
        record["work"] = json!({
            "program": {"provider": "github", "repository": repository, "number": 44},
            "issues": [],
            "inherited": false,
            "revision": 1
        });
        fs::write(&path, serde_json::to_vec_pretty(&record).expect("json")).expect("record");
    }
    let forge = fake_forge_cli(&fixture.root);

    // Disabled: board-disabled before authentication.
    let disabled = Serve::spawn(&fixture.root, &fixture.state_dir, &fixture.home, &[], &[]);
    let (status, body) = disabled.get("/board/programs/v1", &[]);
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "board-disabled");
    drop(disabled);

    let serve = Serve::spawn(
        &fixture.root,
        &fixture.state_dir,
        &fixture.home,
        &["--board"],
        &[("AGENT_SESSION_FORGE_CLI_BIN", &forge.to_string_lossy())],
    );
    let (status, body) = serve.get("/board/programs/v1", &[]);
    assert_eq!(status, 401, "{body}");
    let (status, body) = serve.get_operator("/board/programs/v1");
    assert_eq!(status, 200, "{body}");
    let programs = &body["data"]["board_programs"];
    assert_eq!(
        programs["schema_version"],
        "agent-session.board-programs.v1"
    );
    assert_eq!(body["data"]["machine"], MACHINE);
    assert_eq!(programs["machine"], MACHINE);
    let list = programs["programs"].as_array().expect("programs");
    // The program the tracker cannot read is omitted, not guessed.
    assert_eq!(list.len(), 1, "{programs}");
    assert_eq!(list[0]["ref"], "o/program#44");
    assert_eq!(list[0]["title"], "Program tracker");
    assert_eq!(list[0]["state"], "open");
    assert_eq!(list[0]["stale"], false);
    assert_eq!(
        list[0]["rows"],
        json!([{
            "id": "A1", "title": "Lane", "reference": "o/repo#1", "done": false,
            "phase": null, "after": [], "notes": null
        }])
    );
}
