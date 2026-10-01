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
        let mut argv = vec![
            "--state-dir",
            &self.state,
            "start",
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
            "--paste-delay-ms",
            "0",
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
