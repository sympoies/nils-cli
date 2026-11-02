//! Managed title writers, revision fences and canonical display projection.
use super::coordination::{capability, seed_brokers, write_private_json};
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

struct Fixture {
    _tmp: tempfile::TempDir,
    state: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        fs::create_dir(&state).unwrap();
        seed_brokers(
            &state,
            &[
                (
                    "worker",
                    "worker-runtime",
                    "synthetic-worker-capability-material",
                ),
                (
                    "parent",
                    "parent-runtime",
                    "synthetic-parent-capability-material",
                ),
                (
                    "coordinator",
                    "coordinator-runtime",
                    "synthetic-coordinator-capability-material",
                ),
                ("lead", "lead-runtime", "synthetic-lead-capability-material"),
                (
                    "unrelated",
                    "unrelated-runtime",
                    "synthetic-unrelated-capability-material",
                ),
            ],
        );
        let this = Self { _tmp: tmp, state };
        for id in ["worker", "parent", "coordinator", "lead", "unrelated"] {
            let mut record = this.record(id);
            record["lineage"] = json!({"schema_version":"agent-session.session-lineage.v1", "machine":"fixture", "parent":null, "root":this.reference(id), "depth":0, "starter":{"kind":"operator", "via":"cli"}});
            write_private_json(&this.path(id), &record);
        }
        this.parent("worker", "parent");
        this.parent("parent", "coordinator");
        this
    }
    fn path(&self, id: &str) -> PathBuf {
        self.state.join("sessions").join(id).join("session.json")
    }
    fn record(&self, id: &str) -> Value {
        serde_json::from_slice(&fs::read(self.path(id)).unwrap()).unwrap()
    }
    fn reference(&self, id: &str) -> Value {
        json!({"machine":"fixture", "session_id":id, "session_created_at":self.record(id)["created_at"]})
    }
    fn parent(&self, child: &str, parent: &str) {
        let mut record = self.record(child);
        record["lineage"] = json!({"schema_version":"agent-session.session-lineage.v1", "machine":"fixture", "parent":self.reference(parent), "root":self.reference("coordinator"), "depth":1, "starter":{"kind":"session", "via":"cli"}});
        write_private_json(&self.path(child), &record);
    }
    fn run(&self, caller: Option<&str>, args: &[&str]) -> CmdOutput {
        let mut argv = vec!["--state-dir", self.state.to_str().unwrap()];
        argv.extend_from_slice(args);
        argv.extend(["--format", "json"]);
        let mut options = CmdOptions::new()
            .with_cwd(self._tmp.path())
            .without_ambient_managed_session_env();
        if let Some(caller) = caller {
            options = options.with_env("AGENT_SESSION_ID", caller).with_env(
                "AGENT_SESSION_CAPABILITY_FILE",
                &capability(&self.state, caller),
            );
        }
        run_resolved("agent-session", &argv, &options)
    }
    fn set(&self, caller: &str, revision: &str) -> CmdOutput {
        self.run(
            Some(caller),
            &[
                "title",
                "set",
                "worker",
                "Manual topic",
                "--if-revision",
                revision,
            ],
        )
    }
}
fn success(output: CmdOutput) -> Value {
    assert_eq!(
        output.code,
        0,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    output.stdout_json()["data"].clone()
}
fn refusal(output: CmdOutput, code: &str, exit: i32) {
    assert_eq!(
        output.code,
        exit,
        "{} {}",
        output.stdout_text(),
        output.stderr_text()
    );
    assert_eq!(output.stdout_json()["error"]["code"], code);
    if exit == 65 {
        let details = &output.stdout_json()["error"]["details"];
        assert_eq!(details["retryable"], false);
        assert!(details["next_action"].is_string());
        assert!(details["recovery"].is_object());
    }
    let rendered = format!("{}{}", output.stdout_text(), output.stderr_text());
    assert!(!rendered.contains("Manual topic"));
    assert!(!rendered.contains("synthetic-"));
}
#[test]
fn title_self_set_pins_and_projects_the_manual_title() {
    let f = Fixture::new();
    let data = success(f.set("worker", "0"));
    assert_eq!(data["title"], "Manual topic");
    assert_eq!(data["title_mode"], "pinned");
    assert_eq!(data["title_revision"], 1);
    assert_eq!(data["display_revision"], 1);
    let list = success(f.run(None, &["list"]));
    let worker = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == "worker")
        .unwrap();
    assert_eq!(worker["title"], "Manual topic");
    assert_eq!(worker["title_mode"], "pinned");
    assert_eq!(f.record("worker")["title_state"], Value::Null);
}
#[test]
fn title_parent_and_coordinator_can_set_a_descendant_title() {
    for caller in ["parent", "coordinator"] {
        let f = Fixture::new();
        success(f.set(caller, "0"));
    }
}
#[test]
fn title_adopted_lead_can_set_without_being_launch_parent() {
    let f = Fixture::new();
    let mut worker = f.record("worker");
    worker["lineage_adoption"] = json!({"adopted_by":f.reference("lead"), "revision":1, "updated_at":"2030-01-01T00:00:00Z"});
    write_private_json(&f.path("worker"), &worker);
    success(f.set("lead", "0"));
    success(f.set("parent", "1"));
}
#[test]
fn title_unrecorded_lead_and_unauthenticated_shell_cannot_write() {
    let f = Fixture::new();
    let mut lead = f.record("unrelated");
    lead["role"] = json!("lead");
    write_private_json(&f.path("unrelated"), &lead);
    let before = fs::read(f.path("worker")).unwrap();
    refusal(f.set("unrelated", "0"), "title-forbidden", 65);
    let output = f.run(None, &["title", "auto", "worker", "--if-revision", "0"]);
    assert_eq!(output.code, 65);
    assert_eq!(fs::read(f.path("worker")).unwrap(), before);
}
#[test]
fn title_stale_revision_refuses_without_writing() {
    let f = Fixture::new();
    success(f.set("worker", "0"));
    let before = fs::read(f.path("worker")).unwrap();
    refusal(f.set("worker", "0"), "title-revision-conflict", 65);
    refusal(
        f.run(
            Some("worker"),
            &["title", "auto", "worker", "--if-revision", "0"],
        ),
        "title-revision-conflict",
        65,
    );
    assert_eq!(fs::read(f.path("worker")).unwrap(), before);
}
#[test]
fn title_auto_restores_generation_mode_and_invalidates_previous_inference() {
    let f = Fixture::new();
    success(f.set("worker", "0"));
    let mut record = f.record("worker");
    record["title_state"] =
        json!({"topic":"Manual topic", "topic_source":"user", "references":[], "activity":null});
    write_private_json(&f.path("worker"), &record);
    let data = success(f.run(
        Some("worker"),
        &["title", "auto", "worker", "--if-revision", "1"],
    ));
    assert_eq!(f.record("worker")["title_state"], Value::Null);
    assert_eq!(data["title_mode"], "auto");
    assert_eq!(data["title_revision"], 2);
    assert_eq!(data["display_revision"], 2);
}
#[test]
fn title_validation_bounds_and_single_line_refusals_preserve_the_record() {
    let f = Fixture::new();
    let long = "x".repeat(121);
    let before = fs::read(f.path("worker")).unwrap();
    for (title, code) in [
        (long.as_str(), "title-too-long"),
        ("first\nsecond", "title-invalid"),
        (" ", "title-invalid"),
    ] {
        refusal(
            f.run(
                Some("worker"),
                &["title", "set", "worker", title, "--if-revision", "0"],
            ),
            code,
            64,
        );
    }
    assert_eq!(fs::read(f.path("worker")).unwrap(), before);
}

#[test]
fn title_supervisor_identity_requires_matching_creation_and_machine() {
    for field in ["session_created_at", "machine"] {
        let f = Fixture::new();
        let mut record = f.record("worker");
        record["lineage"]["parent"][field] = json!("different-identity");
        write_private_json(&f.path("worker"), &record);
        refusal(f.set("parent", "0"), "title-forbidden", 65);
    }
}

#[test]
fn title_revision_is_required_and_unicode_title_limit_counts_characters() {
    let f = Fixture::new();
    let output = f.run(Some("worker"), &["title", "auto", "worker"]);
    refusal(output, "parse-error", 64);
    let title = "界".repeat(120);
    let padded = format!("  {title}  ");
    let data = success(f.run(
        Some("worker"),
        &["title", "set", "worker", &padded, "--if-revision", "0"],
    ));
    assert_eq!(data["title"], title);
}

#[test]
fn title_start_uses_the_same_plain_title_validation() {
    let f = Fixture::new();
    let (tmux, _) = super::cli::fake_tmux(f._tmp.path());
    let agent = super::cli::fake_agent(f._tmp.path(), "codex");
    for title in [
        "first\nsecond".to_string(),
        "x".repeat(121),
        " ".to_string(),
    ] {
        let output = f.run(
            None,
            &[
                "start",
                "--id",
                "invalid-title",
                "--agent",
                "codex",
                "--cwd",
                f._tmp.path().to_str().unwrap(),
                "--tmux-bin",
                tmux.to_str().unwrap(),
                "--agent-bin",
                agent.to_str().unwrap(),
                "--title",
                &title,
                "--title-mode",
                "pinned",
            ],
        );
        assert_eq!(output.code, 64, "{}", output.stdout_text());
        assert!(!f.path("invalid-title").exists());
    }
}
