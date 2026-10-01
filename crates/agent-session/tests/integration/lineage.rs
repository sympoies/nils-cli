//! Session lineage and work references (`docs/specs/session-lineage-work-v1.md`)
//! through the CLI: a start inside a managed session records it as parent and
//! inherits its program and issues.

use std::fs;
use std::path::{Path, PathBuf};

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use super::cli::{fake_agent, fake_tmux};

const MACHINE: &str = "lineage-host";

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

fn session_ref(record: &Value, incarnation: bool) -> Value {
    let mut value = json!({
        "machine": MACHINE,
        "session_id": record["id"],
        "session_created_at": record["created_at"],
    });
    if incarnation {
        value["session_incarnation"] = record["runtime"]["launch_id"].clone();
    }
    value
}

fn github(repository: &str, number: u64) -> Value {
    json!({"provider": "github", "repository": repository, "number": number})
}

#[test]
fn start_records_the_calling_session_as_parent_and_inherits_its_work() {
    let fixture = Fixture::new();
    let root_view = fixture.started(
        "lineage-root",
        &[
            "--program",
            "serenvia/laoda#44",
            "--issue",
            "sympoies/nils-cli#2032",
        ],
        None,
    );
    let root = fixture.record("lineage-root");
    assert_eq!(
        root["lineage"],
        json!({
            "schema_version": "agent-session.session-lineage.v1",
            "machine": MACHINE,
            "parent": null,
            "root": session_ref(&root, false),
            "depth": 0,
            "starter": {"kind": "operator", "via": "cli"},
            "budget": null,
        })
    );
    let root_work = json!({
        "program": github("serenvia/laoda", 44),
        "issues": [github("sympoies/nils-cli", 2032)],
        "inherited": false,
        "revision": 1,
    });
    assert_eq!(root["work"], root_work);
    assert_eq!(
        (&root_view["lineage"], &root_view["work"]),
        (&root["lineage"], &root_work)
    );

    let root_launch = root["runtime"]["launch_id"].as_str().unwrap().to_string();
    fixture.started(
        "lineage-child",
        &[],
        Some(("lineage-root", root_launch.as_str())),
    );
    let child = fixture.record("lineage-child");
    assert_eq!(
        child["lineage"],
        json!({
            "schema_version": "agent-session.session-lineage.v1",
            "machine": MACHINE,
            "parent": session_ref(&root, true),
            "root": session_ref(&root, false),
            "depth": 1,
            "starter": {"kind": "session", "via": "cli"},
            "budget": null,
        })
    );
    assert_eq!(
        child["work"],
        json!({
            "program": github("serenvia/laoda", 44),
            "issues": [github("sympoies/nils-cli", 2032)],
            "inherited": true,
            "revision": 1,
        })
    );

    // A grandchild keeps the root and replaces only the dimension it names.
    let child_launch = child["runtime"]["launch_id"].as_str().unwrap().to_string();
    fixture.started(
        "lineage-grandchild",
        &["--issue", "sympoies/nils-cli#2040"],
        Some(("lineage-child", child_launch.as_str())),
    );
    let grandchild = fixture.record("lineage-grandchild");
    assert_eq!(grandchild["lineage"]["parent"], session_ref(&child, true));
    assert_eq!(grandchild["lineage"]["root"], session_ref(&root, false));
    assert_eq!(grandchild["lineage"]["depth"], 2);
    assert_eq!(
        grandchild["work"],
        json!({
            "program": github("serenvia/laoda", 44),
            "issues": [github("sympoies/nils-cli", 2040)],
            "inherited": false,
            "revision": 1,
        })
    );

    // --no-parent starts an intentional new root; --no-inherit-work drops
    // the parent's references.
    fixture.started(
        "lineage-new-root",
        &["--no-parent"],
        Some(("lineage-child", child_launch.as_str())),
    );
    let new_root = fixture.record("lineage-new-root");
    assert_eq!(new_root["lineage"]["parent"], Value::Null);
    assert_eq!(new_root["lineage"]["root"], session_ref(&new_root, false));
    assert_eq!(new_root["lineage"]["starter"]["kind"], "operator");
    assert!(new_root.get("work").is_none(), "{new_root}");
    fixture.started(
        "lineage-no-work",
        &["--no-inherit-work"],
        Some(("lineage-child", child_launch.as_str())),
    );
    let no_work = fixture.record("lineage-no-work");
    assert_eq!(no_work["lineage"]["depth"], 2);
    assert!(no_work.get("work").is_none(), "{no_work}");
}

#[test]
fn run_records_the_calling_session_as_parent_like_start() {
    let fixture = Fixture::new();
    fixture.started("run-parent", &["--issue", "sympoies/nils-cli#2032"], None);
    let parent = fixture.record("run-parent");
    let launch = parent["runtime"]["launch_id"].as_str().unwrap().to_string();
    let output = fixture.launch(
        "run",
        "run-child",
        &["--prompt", "one-shot task"],
        Some(("run-parent", launch.as_str())),
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let child = fixture.record("run-child");
    assert_eq!(child["lineage"]["parent"], session_ref(&parent, true));
    assert_eq!(child["lineage"]["root"], session_ref(&parent, false));
    assert_eq!(child["lineage"]["depth"], 1);
    assert_eq!(
        child["lineage"]["starter"],
        json!({"kind": "session", "via": "cli"})
    );
    assert_eq!(
        child["work"],
        json!({
            "program": null,
            "issues": [github("sympoies/nils-cli", 2032)],
            "inherited": true,
            "revision": 1,
        })
    );
}

#[test]
fn a_child_reuses_the_machine_label_its_parent_was_created_under() {
    // A daemon started with `serve --machine m4` on a host named otherwise
    // labels its sessions m4; a start inside one keeps that label.
    let fixture = Fixture::new();
    fixture.started("label-parent", &[], None);
    let path = Path::new(&fixture.state).join("sessions/label-parent/session.json");
    let mut parent = fixture.record("label-parent");
    parent["lineage"]["machine"] = json!("daemon-label");
    parent["lineage"]["root"]["machine"] = json!("daemon-label");
    fs::write(&path, serde_json::to_vec(&parent).expect("record")).expect("write record");
    let launch = parent["runtime"]["launch_id"].as_str().unwrap().to_string();
    fixture.started("label-child", &[], Some(("label-parent", launch.as_str())));
    let child = fixture.record("label-child");
    assert_eq!(child["lineage"]["machine"], "daemon-label");
    assert_eq!(child["lineage"]["parent"]["machine"], "daemon-label");
    assert_eq!(child["lineage"]["root"]["machine"], "daemon-label");
}

#[test]
fn start_never_guesses_a_parent_that_does_not_resolve() {
    let fixture = Fixture::new();
    fixture.started("lineage-parent", &[], None);
    for (id, caller) in [
        ("lineage-stale", ("lineage-parent", "an-older-runtime")),
        ("lineage-foreign", ("not-in-this-state-dir", "whatever")),
    ] {
        let output = fixture.start(id, &[], Some(caller));
        assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
        let stderr = output.stderr_text();
        assert!(
            stderr.starts_with(&format!("warning: AGENT_SESSION_ID {}", caller.0))
                && stderr.contains("starting a new root session without a parent"),
            "{stderr}"
        );
        let record = fixture.record(id);
        assert_eq!(record["lineage"]["parent"], Value::Null, "{id}");
        assert_eq!(record["lineage"]["depth"], 0, "{id}");
        assert_eq!(
            record["lineage"]["starter"],
            json!({"kind": "operator", "via": "cli"})
        );
    }
}

#[test]
fn start_rejects_free_text_and_too_many_work_references() {
    let fixture = Fixture::new();
    for args in [
        vec!["--issue", "fix the login bug"],
        vec!["--program", "serenvia/laoda"],
        vec![
            "--issue", "a/b#1", "--issue", "a/b#2", "--issue", "a/b#3", "--issue", "a/b#4",
            "--issue", "a/b#5",
        ],
    ] {
        let output = fixture.start("lineage-refused", &args, None);
        assert_eq!(output.code, 64, "{args:?}: stdout={}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "work-ref-invalid",
            "{args:?}"
        );
        assert!(
            !Path::new(&fixture.state)
                .join("sessions/lineage-refused")
                .exists()
        );
    }
}

impl Fixture {
    /// An operator command (no managed session in the environment).
    fn operator(&self, args: &[&str]) -> CmdOutput {
        let mut argv = vec!["--state-dir", self.state.as_str()];
        argv.extend_from_slice(args);
        argv.extend_from_slice(&["--format", "json"]);
        let options = CmdOptions::new()
            .with_cwd(&self.root)
            .without_ambient_managed_session_env()
            .with_env_remove_many(&["AGENT_SESSION_HOST"])
            .with_envs(&[("AGENT_SESSION_MACHINE", MACHINE)]);
        run_resolved("agent-session", &argv, &options)
    }
}

fn error_code(output: &CmdOutput) -> Value {
    output.stdout_json()["error"]["code"].clone()
}

#[test]
fn lineage_adopt_records_a_steward_and_keeps_the_original_parent() {
    let fixture = Fixture::new();
    fixture.started("adopt-parent", &[], None);
    let parent = fixture.record("adopt-parent");
    let launch = parent["runtime"]["launch_id"].as_str().unwrap().to_string();
    fixture.started("adopt-child", &[], Some(("adopt-parent", launch.as_str())));
    fixture.started("adopt-steward", &[], None);
    let steward = fixture.record("adopt-steward");
    let lineage_before = fixture.record("adopt-child")["lineage"].clone();

    let output = fixture.operator(&["lineage", "adopt", "adopt-child", "--by", "adopt-steward"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let body = output.stdout_json();
    assert_eq!(body["schema_version"], "cli.agent-session.lineage-adopt.v1");
    let mut steward_ref = session_ref(&steward, true);
    assert_eq!(body["data"]["lineage_adoption"]["adopted_by"], steward_ref);
    assert_eq!(body["data"]["lineage_adoption"]["revision"], 1);
    assert_eq!(body["data"]["effective_parent"], steward_ref);
    let child = fixture.record("adopt-child");
    assert_eq!(child["lineage"], lineage_before);
    assert_eq!(child["lineage_adoption"]["adopted_by"], steward_ref);

    // The adoption revision fences concurrent stewards.
    let output = fixture.operator(&[
        "lineage",
        "adopt",
        "adopt-child",
        "--by",
        "adopt-parent",
        "--if-revision",
        "0",
    ]);
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert_eq!(error_code(&output), "lineage-revision-conflict");
    assert_eq!(
        output.stdout_json()["error"]["details"]["current_revision"],
        1
    );

    // The list view carries the adoption; a never-adopted session has none.
    let output = fixture.operator(&["list"]);
    let list = output.stdout_json();
    let sessions = list["data"].as_array().expect("sessions");
    let view = |id: &str| {
        sessions
            .iter()
            .find(|session| session["id"] == id)
            .unwrap_or_else(|| panic!("{id} listed"))
            .clone()
    };
    assert_eq!(
        view("adopt-child")["lineage_adoption"],
        child["lineage_adoption"]
    );
    assert!(view("adopt-steward").get("lineage_adoption").is_none());

    // A steward on another machine is named by its full identity: machine and
    // creation time together.
    for args in [
        vec![
            "lineage",
            "adopt",
            "adopt-child",
            "--by",
            "remote-steward",
            "--by-machine",
            "far-host",
        ],
        vec![
            "lineage",
            "adopt",
            "adopt-child",
            "--by",
            "adopt-steward",
            "--by-created-at",
            "2026-10-01T00:00:00Z",
        ],
    ] {
        let output = fixture.operator(&args);
        assert_eq!(output.code, 64, "{args:?}: stdout={}", output.stdout_text());
    }
    let output = fixture.operator(&[
        "lineage",
        "adopt",
        "adopt-child",
        "--by",
        "remote-steward",
        "--by-machine",
        "far-host",
        "--by-created-at",
        "2026-10-01T00:00:00Z",
        "--if-revision",
        "1",
    ]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    steward_ref = json!({
        "machine": "far-host",
        "session_id": "remote-steward",
        "session_created_at": "2026-10-01T00:00:00Z",
    });
    assert_eq!(
        output.stdout_json()["data"]["lineage_adoption"]["adopted_by"],
        steward_ref
    );

    // Clearing the steward makes the original parent effective again.
    let output = fixture.operator(&["lineage", "adopt", "adopt-child", "--clear"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let data = &output.stdout_json()["data"];
    assert_eq!(data["lineage_adoption"]["adopted_by"], Value::Null);
    assert_eq!(data["lineage_adoption"]["revision"], 3);
    assert_eq!(data["effective_parent"], session_ref(&parent, true));

    let output = fixture.operator(&["lineage", "adopt", "adopt-child", "--by", "adopt-child"]);
    assert_eq!(output.code, 64, "stdout={}", output.stdout_text());
    assert_eq!(error_code(&output), "lineage-invalid");

    // A parent cannot be adopted by its own child: that would be a loop.
    let parent_before = fixture.record("adopt-parent");
    let output = fixture.operator(&["lineage", "adopt", "adopt-parent", "--by", "adopt-child"]);
    assert_eq!(output.code, 64, "stdout={}", output.stdout_text());
    assert_eq!(error_code(&output), "lineage-invalid");
    assert_eq!(fixture.record("adopt-parent"), parent_before);
    // Nor through a steward: the child's steward cannot be adopted by it.
    let output = fixture.operator(&["lineage", "adopt", "adopt-child", "--by", "adopt-steward"]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let output = fixture.operator(&["lineage", "adopt", "adopt-steward", "--by", "adopt-child"]);
    assert_eq!(error_code(&output), "lineage-invalid");
}

#[test]
fn work_set_replaces_named_dimensions_under_a_revision_fence() {
    let fixture = Fixture::new();
    fixture.started("work-target", &[], None);

    let output = fixture.operator(&[
        "work",
        "set",
        "work-target",
        "--program",
        "serenvia/laoda#44",
        "--if-revision",
        "0",
    ]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let body = output.stdout_json();
    assert_eq!(body["schema_version"], "cli.agent-session.work-set.v1");
    assert_eq!(
        body["data"]["work"],
        json!({
            "program": github("serenvia/laoda", 44),
            "issues": [],
            "inherited": false,
            "revision": 1,
        })
    );

    let output = fixture.operator(&[
        "work",
        "set",
        "work-target",
        "--issue",
        "sympoies/nils-cli#2032",
        "--if-revision",
        "0",
    ]);
    assert_eq!(output.code, 65, "stdout={}", output.stdout_text());
    assert_eq!(error_code(&output), "work-revision-conflict");

    let output = fixture.operator(&[
        "work",
        "set",
        "work-target",
        "--issue",
        "sympoies/nils-cli#2032",
        "--issue",
        "sympoies/nils-cli#2033",
        "--if-revision",
        "1",
    ]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    assert_eq!(
        fixture.record("work-target")["work"],
        json!({
            "program": github("serenvia/laoda", 44),
            "issues": [github("sympoies/nils-cli", 2032), github("sympoies/nils-cli", 2033)],
            "inherited": false,
            "revision": 2,
        })
    );

    let output = fixture.operator(&[
        "work",
        "set",
        "work-target",
        "--clear-program",
        "--if-revision",
        "2",
    ]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    let work = &output.stdout_json()["data"]["work"];
    assert_eq!(
        (&work["program"], work["revision"].as_u64()),
        (&Value::Null, Some(3))
    );

    // Emptied work stays, so its revision keeps fencing later updates.
    let output = fixture.operator(&[
        "work",
        "set",
        "work-target",
        "--clear-issues",
        "--if-revision",
        "3",
    ]);
    assert_eq!(output.code, 0, "stdout={}", output.stdout_text());
    assert_eq!(
        fixture.record("work-target")["work"],
        json!({"program": null, "issues": [], "inherited": false, "revision": 4})
    );
    let output = fixture.operator(&[
        "work",
        "set",
        "work-target",
        "--program",
        "serenvia/laoda#44",
        "--if-revision",
        "0",
    ]);
    assert_eq!(error_code(&output), "work-revision-conflict");
    assert_eq!(
        output.stdout_json()["error"]["details"]["current_revision"],
        4
    );

    for args in [
        vec!["work", "set", "work-target", "--if-revision", "4"],
        vec![
            "work",
            "set",
            "work-target",
            "--issue",
            "free text",
            "--if-revision",
            "4",
        ],
    ] {
        let output = fixture.operator(&args);
        assert_eq!(output.code, 64, "{args:?}: stdout={}", output.stdout_text());
        assert_eq!(error_code(&output), "work-ref-invalid", "{args:?}");
    }
}
