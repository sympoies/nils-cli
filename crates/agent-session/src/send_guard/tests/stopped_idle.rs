//! Clock progression inside one attempt must not consume a wake without delivery.
use super::*;
use pretty_assertions::assert_eq;

struct Fixture {
    _temp: tempfile::TempDir,
    context: CliContext,
    record: SessionRecord,
    args: GuardArgs,
    proof: std::cell::RefCell<Option<activity::shadow::StoppedIdle>>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: temp.path().to_path_buf(),
            host: None,
        };
        let record: SessionRecord = serde_json::from_value(json!({
            "schema_version":"agent-session.session.v1", "id":"fixture", "agent":"claude",
            "mode":"interactive", "title":null, "cwd":"/", "tmux_session":"fixture",
            "prompt_file":null, "log_file":null, "created_at":"", "updated_at":"",
            "runtime":{"kind":"tmux", "tmux_session":"fixture", "generation":1,
                "launch_id":"runtime-one", "started_at":"2000-01-01T00:00:00Z"}
        }))
        .unwrap();
        let dir = context.state_dir.join("sessions/fixture");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("activity.json"), serde_json::to_vec(&json!({
        "schema_version":"agent-session.activity.v1", "runtime_id":"runtime-one", "runtime_generation":1,
        "state":{"schema_version":"agent-session.turn-state.v1", "phase":"working", "revision":3,
            "phase_changed_at":"2000-01-01T00:00:00Z",
            "source":{"kind":"provider_hook", "confidence":"observed", "provider":"claude"},
            "current_turn":{"provider_turn_id":"turn-one", "started_at":"2000-01-01T00:00:00Z"},
            "diagnostic":{"reason":"completion_evidence_pending"}},
        "last_provider_event_kind":"stop_observed", "last_provider_event_provider":"claude",
        "last_provider_event_turn_id":"turn-one", "last_provider_event_at":"2000-01-01T00:00:00Z",
        "last_event_at":"2000-01-01T00:00:00Z"
    })).unwrap()).unwrap();
        fs::write(dir.join("activity.shadow.json"), serde_json::to_vec(&json!({
        "schema_version":"agent-session.activity-shadow.v1", "runtime_id":"runtime-one",
        "runtime_generation":1, "activity_revision":3,
        "observation":{"observer_version":"claude-agents.v1", "rule_id":"claude_agents_idle",
            "projection":"waiting", "observed_at":"2000-01-01T00:00:01Z", "disagrees":true}
    })).unwrap()).unwrap();
        let args = GuardArgs {
            require_idle: true,
            ..GuardArgs::default()
        };
        Self {
            _temp: temp,
            context,
            record,
            args,
            proof: Default::default(),
        }
    }
    fn admission(&self) -> Admission<'_> {
        Admission {
            stopped_idle: &self.proof,
            context: &self.context,
            record: &self.record,
            args: &self.args,
            tmux_bin: Path::new("unused"),
            verify_composer: false,
            deadline: Instant::now() + Duration::from_secs(60),
        }
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.context.state_dir.join("sessions/fixture").join(name)
    }
    fn edit(&self, name: &str, change: impl FnOnce(&mut serde_json::Value)) {
        let path = self.path(name);
        let mut value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        change(&mut value);
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
}

fn at(seconds: u8) -> jiff::Timestamp {
    format!("2000-01-01T00:00:{seconds:02}Z").parse().unwrap()
}

#[test]
fn admitted_stopped_idle_survives_age_between_claim_and_submit() {
    let fixture = Fixture::new();
    let admission = fixture.admission();
    admission.idle_at("admission", at(30)).unwrap(); // age 29
    admission.idle_at("claimed", at(31)).unwrap(); // age 30
    admission.idle_at("submit", at(32)).unwrap(); // age 31
    let separate = fixture.admission();
    let unbound = Admission {
        stopped_idle: &Default::default(),
        ..separate
    };
    assert!(unbound.idle_at("claimed", at(32)).is_err());
    let retry = admission.idle_at("admission", at(32)).unwrap_err();
    assert_eq!(retry.details().unwrap()["safe_to_retry"], true);
}

#[test]
fn admitted_idle_is_revoked_by_current_conflicting_evidence() {
    use std::os::fd::AsRawFd;
    for case in [
        "runtime",
        "generation",
        "revision",
        "newer-work",
        "newer-waiting",
        "attention",
        "busy",
        "blocked",
        "ingress",
        "future",
        "backwards",
    ] {
        let fixture = Fixture::new();
        let admission = fixture.admission();
        admission.idle_at("admission", at(30)).unwrap();
        let mut record = fixture.record.clone();
        let mut ingress = None;
        match case {
            "runtime" => {
                record.runtime.as_mut().unwrap().launch_id = "replaced".into();
                for name in ["activity.json", "activity.shadow.json"] {
                    fixture.edit(name, |d| d["runtime_id"] = json!("replaced"));
                }
            }
            "generation" => {
                record.runtime.as_mut().unwrap().generation = 2;
                for name in ["activity.json", "activity.shadow.json"] {
                    fixture.edit(name, |d| d["runtime_generation"] = json!(2));
                }
            }
            "revision" => {
                fixture.edit("activity.json", |d| d["state"]["revision"] = json!(4));
                fixture.edit("activity.shadow.json", |d| d["activity_revision"] = json!(4));
            }
            "newer-work" => fixture.edit("activity.json", |d| {
                d["last_provider_event_kind"] = json!("progress");
                d["last_event_at"] = json!("2000-01-01T00:00:02Z");
            }),
            "newer-waiting" => fixture.edit("activity.json", |d| {
                d["state"]["revision"] = json!(4);
                d["state"]["phase"] = json!("waiting");
                d["state"]["current_turn"] = serde_json::Value::Null;
            }),
            "attention" => fixture.edit("activity.json", |d| d["state"]["current_turn"]["attention"] = json!({"kind":"clarification", "requested_at":"2000-01-01T00:00:02Z", "pending_count":1})),
            "busy" | "blocked" => fixture.edit("activity.shadow.json", |d| {
                d["observation"]["rule_id"] = json!(if case == "busy" {"claude_agents_busy"} else {"claude_agents_waiting_permission_prompt"});
                d["observation"]["projection"] = json!(if case == "busy" {"working"} else {"needs_input"});
            }),
            "ingress" => {
                let file = fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(fixture.path(".activity-ingress.lock")).unwrap();
                // SAFETY: the fixture owns this descriptor until after the guard check.
                assert_eq!(unsafe {libc::flock(file.as_raw_fd(), libc::LOCK_SH)}, 0);
                ingress = Some(file);
            }
            "future" => fixture.edit("activity.shadow.json", |d| d["observation"]["observed_at"] = json!("2000-01-01T00:00:40Z")),
            "backwards" => fixture.edit("activity.shadow.json", |d| d["observation"]["observed_at"] = json!("2000-01-01T00:00:00Z")),
            _ => unreachable!(),
        }
        let changed = Admission {
            record: &record,
            ..admission
        };
        let error = changed.idle_at("claimed", at(30)).expect_err(case);
        assert_eq!(error.details().unwrap()["safe_to_retry"], false, "{case}");
        drop(ingress);
        if case == "busy" {
            fixture.edit("activity.shadow.json", |d| {
                d["observation"]["rule_id"] = json!("claude_agents_idle");
                d["observation"]["projection"] = json!("waiting");
            });
            assert!(changed.idle_at("claimed", at(32)).is_err());
        }
    }
}

#[test]
fn a_recent_observation_before_stop_cannot_admit_or_authorize_codex() {
    let mut fixture = Fixture::new();
    fixture.edit("activity.json", |d| {
        d["last_provider_event_at"] = json!("2000-01-01T00:00:02Z");
        d["last_event_at"] = json!("2000-01-01T00:00:02Z");
    });
    assert!(fixture.admission().idle_at("admission", at(30)).is_err());
    // Restore the positive control before changing only the provider.
    fixture.edit("activity.json", |d| {
        d["last_provider_event_at"] = json!("2000-01-01T00:00:00Z");
        d["last_event_at"] = json!("2000-01-01T00:00:00Z");
    });
    fixture.admission().idle_at("admission", at(30)).unwrap();
    fixture.record.agent = "codex".into();
    assert!(fixture.admission().idle_at("admission", at(30)).is_err());
}
