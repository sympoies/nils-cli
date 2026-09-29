//! Session board v1 (`docs/specs/session-board-v1.md`): the daemon's local
//! projection and the `machine` label on `agent-session list`.

use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
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
        for key in nils_test_support::cmd::MANAGED_SESSION_ENV
            .iter()
            .chain(BOARD_ENV.iter())
        {
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
