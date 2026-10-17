use std::fs;
use std::path::{Path, PathBuf};

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use super::cli::{fake_agent, fake_tmux};

const MACHINE: &str = "worker-host";

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    state: String,
    cwd: String,
    tmux: String,
    codex: String,
    tmux_log: String,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let cwd = root.join("repo");
        fs::create_dir_all(&cwd).expect("cwd");
        let (tmux, tmux_log) = fake_tmux(&root);
        let codex = fake_agent(&root, "codex");
        Self {
            state: root.join("state").to_string_lossy().to_string(),
            cwd: cwd.to_string_lossy().to_string(),
            tmux: tmux.to_string_lossy().to_string(),
            codex: codex.to_string_lossy().to_string(),
            tmux_log: tmux_log.to_string_lossy().to_string(),
            root,
            _tmp: tmp,
        }
    }

    /// `agent-session start` with `args`, as the managed session `caller`
    /// (`(AGENT_SESSION_ID, AGENT_SESSION_RUNTIME_ID)`) or from a plain shell.
    fn start(&self, id: &str, args: &[&str], caller: Option<(&str, &str)>) -> CmdOutput {
        let mut start_args = vec!["--paste-delay-ms", "0"];
        start_args.extend_from_slice(args);
        self.launch("start", id, &start_args, caller)
    }

    /// `agent-session <command>` (`start` or `run`) for session `id`.
    fn launch(
        &self,
        command: &str,
        id: &str,
        args: &[&str],
        caller: Option<(&str, &str)>,
    ) -> CmdOutput {
        let mut argv = vec![
            "--state-dir",
            &self.state,
            command,
            "--agent",
            "codex",
            "--id",
            id,
            "--cwd",
            &self.cwd,
            "--tmux-bin",
            &self.tmux,
            "--agent-bin",
            &self.codex,
            "--format",
            "json",
        ];
        argv.extend_from_slice(args);
        let mut envs = vec![
            ("AGENT_SESSION_FAKE_TMUX_LOG", self.tmux_log.as_str()),
            ("AGENT_SESSION_MACHINE", MACHINE),
        ];
        if let Some((session, runtime)) = caller {
            envs.push(("AGENT_SESSION_ID", session));
            envs.push(("AGENT_SESSION_RUNTIME_ID", runtime));
        }
        let options = CmdOptions::new()
            .with_cwd(&self.root)
            .without_ambient_managed_session_env()
            .with_env_remove_many(&["AGENT_SESSION_HOST"])
            .with_envs(&envs);
        run_resolved("agent-session", &argv, &options)
    }

    fn record(&self, id: &str) -> Value {
        let path = Path::new(&self.state)
            .join("sessions")
            .join(id)
            .join("session.json");
        serde_json::from_slice(&fs::read(path).expect("session record")).expect("record json")
    }

    fn started(&self, id: &str, args: &[&str], caller: Option<(&str, &str)>) -> Value {
        let output = self.start(id, args, caller);
        assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
        assert_eq!(output.stderr_text(), "");
        output.stdout_json()["data"].clone()
    }
}

#[test]
fn start_records_origin_and_inherits_or_overrides_workspace() {
    let fixture = Fixture::new();
    let root = fixture.started("origin-root", &[], None);
    assert_eq!(root.get("origin"), Some(&json!({"parent":null})));
    assert_eq!(root.get("workspace"), Some(&Value::Null));
    let parent = fixture.record("origin-root");
    let output = fixture.start(
        "origin-child",
        &["--workspace", "example-workspace"],
        Some((
            "origin-root",
            parent["runtime"]["launch_id"].as_str().unwrap(),
        )),
    );
    assert_eq!(
        output.code,
        0,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    let child = fixture.record("origin-child");
    assert_eq!(
        child["origin"],
        json!({"parent":{"session_id":"origin-root","machine":MACHINE}})
    );
    assert_eq!(child["workspace"], "example-workspace");
    let runtime = child["runtime"]["launch_id"].as_str().unwrap();
    let inherited = fixture.started("origin-grandchild", &[], Some(("origin-child", runtime)));
    assert_eq!(inherited["workspace"], "example-workspace");
    let overridden = fixture.started(
        "origin-override",
        &["--workspace", "another-workspace"],
        Some(("origin-child", runtime)),
    );
    assert_eq!(overridden["workspace"], "another-workspace");
}

#[test]
fn invalid_workspace_is_rejected_before_record_creation() {
    let fixture = Fixture::new();
    for invalid in [
        "Uppercase",
        "path/value",
        "has_space",
        "double--separator",
        "trailing-",
        "",
    ] {
        let output = fixture.start("invalid-workspace", &["--workspace", invalid], None);
        assert_eq!(output.code, 64);
        assert_eq!(output.stdout_json()["error"]["code"], "workspace-invalid");
        assert!(output.stdout_json()["error"]["details"]["recovery"]["command"].is_string());
        assert!(
            !Path::new(&fixture.state)
                .join("sessions/invalid-workspace/session.json")
                .exists()
        );
    }
}

#[test]
fn workspace_set_updates_existing_session_and_new_children_only() {
    let fixture = Fixture::new();
    fixture.started("workspace-root", &[], None);
    let root = fixture.record("workspace-root");
    let runtime = root["runtime"]["launch_id"].as_str().unwrap();
    fixture.started("before-tag", &[], Some(("workspace-root", runtime)));
    let options = CmdOptions::new()
        .with_cwd(&fixture.root)
        .with_env("AGENT_SESSION_TMUX_BIN", &fixture.tmux)
        .without_ambient_managed_session_env();
    let output = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            &fixture.state,
            "workspace",
            "set",
            "workspace-root",
            "example-workspace",
            "--format",
            "json",
        ],
        &options,
    );
    assert_eq!(
        output.code,
        0,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    assert_eq!(
        output.stdout_json()["data"]["workspace"],
        "example-workspace"
    );
    let mut expected = root.clone();
    expected["workspace"] = json!("example-workspace");
    let mut tagged = fixture.record("workspace-root");
    expected.as_object_mut().unwrap().remove("updated_at");
    tagged.as_object_mut().unwrap().remove("updated_at");
    assert_eq!(tagged, expected);
    let after = fixture.started("after-tag", &[], Some(("workspace-root", runtime)));
    assert_eq!(after["workspace"], "example-workspace");
    assert_eq!(fixture.record("before-tag")["workspace"], Value::Null);
    let invalid = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            &fixture.state,
            "workspace",
            "set",
            "workspace-root",
            "private/path",
            "--format",
            "json",
        ],
        &options,
    );
    assert_eq!(invalid.code, 64);
    assert_eq!(
        fixture.record("workspace-root")["workspace"],
        "example-workspace"
    );
    let list = run_resolved(
        "agent-session",
        &["--state-dir", &fixture.state, "list", "--format", "json"],
        &options,
    );
    assert_eq!(list.code, 0, "{}", list.stderr_text());
    let listed = list.stdout_json();
    let child = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["id"] == "after-tag")
        .unwrap();
    assert_eq!(child["workspace"], "example-workspace");
    assert_eq!(child["origin"], after["origin"]);
    // An old record without the fields projects explicit nullable defaults.
    let path = Path::new(&fixture.state).join("sessions/workspace-root/session.json");
    let mut older = fixture.record("workspace-root");
    older.as_object_mut().unwrap().remove("origin");
    older.as_object_mut().unwrap().remove("workspace");
    fs::write(path, serde_json::to_vec(&older).unwrap()).unwrap();
    let shown = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            &fixture.state,
            "command",
            "workspace-root",
            "--format",
            "json",
        ],
        &options,
    );
    assert_eq!(shown.code, 0, "{}", shown.stderr_text());
    assert_eq!(
        shown.stdout_json()["data"].get("origin"),
        Some(&json!({"parent":null}))
    );
    assert_eq!(
        shown.stdout_json()["data"].get("workspace"),
        Some(&Value::Null)
    );
}

#[test]
fn remote_parent_identity_is_recorded_without_a_local_parent_record() {
    let fixture = Fixture::new();
    let child = fixture.start(
        "remote-child",
        &[],
        Some(("remote-parent", "remote-runtime")),
    );
    assert_eq!(child.code, 0, "{}", child.stderr_text());
    let record = fixture.record("remote-child");
    assert_eq!(
        record["origin"],
        json!({"parent":{"session_id":"remote-parent","machine":MACHINE}})
    );
    assert_eq!(record["workspace"], Value::Null);
}
