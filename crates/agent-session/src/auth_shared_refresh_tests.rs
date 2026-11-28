//! Synthetic shared-account rotation through the broker and Claude hook ingress.
use super::*;
use nils_test_support::cmd::{CmdOptions, run_with};
use pretty_assertions::{assert_eq, assert_ne};
use sha2::{Digest, Sha256};

const HOOK_POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "fixture-policy"
version = "2026.11.24.1"

[[rules]]
id = "fixture.claude-activity"
products = ["claude"]
events = ["StopFailure"]
priority = 10
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }
"#;

struct Resident {
    root: PathBuf,
    context: CliContext,
    tmux: crate::restart::tests::fixture::FakeTmux,
    initial: SessionRecord,
}

impl Resident {
    fn start(parent: &Path, id: &str) -> Self {
        Self::start_for_account(parent, id, "alpha")
    }

    fn start_for_account(parent: &Path, id: &str, account: &str) -> Self {
        let root = parent.join(id);
        fs::create_dir(&root).unwrap();
        let context = context(&root);
        let tmux = fake_tmux(&root);
        let mut seed = seed_exited_session_for(&context, &root, id);
        crate::claude_account::bind_initial(&mut seed, account, "explicit").unwrap();
        crate::write_session_record(&context, &seed).unwrap();
        crate::resume_session_by_id(&context, id, &tmux).unwrap();
        let initial = load_session_record(&context, id).unwrap();
        session_ready_for(&context, id, &launch_id(&initial).unwrap());
        Self {
            root,
            context,
            tmux,
            initial,
        }
    }

    fn config_dir(&self) -> PathBuf {
        let env = fs::read_to_string(self.root.join("env")).unwrap();
        PathBuf::from(
            env.lines()
                .find_map(|line| line.strip_prefix("CLAUDE_CONFIG_DIR="))
                .unwrap(),
        )
    }

    fn observe_revocation(&self) {
        let config_home = self.root.join("hook-config");
        fs::create_dir_all(config_home.join("agent-hook")).unwrap();
        let policy = self.root.canonicalize().unwrap().join("hook-policy.toml");
        fs::write(&policy, HOOK_POLICY).unwrap();
        let config = config_home.join("agent-hook/config.toml");
        let digest = Sha256::digest(HOOK_POLICY.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        fs::write(
            &config,
            format!(
                "schema_version = \"agent-hook.config.v1\"\n[policy]\npath = {}\ndigest = \"sha256:{digest}\"\n",
                serde_json::to_string(&policy).unwrap(),
            ),
        )
        .unwrap();
        for path in [&policy, &config] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let dispatcher = nils_test_support::bin::required_sibling("agent-hook", "nils-agent-hook");
        let helper = nils_test_support::bin::resolve("agent-session");
        let payload = json!({
            "hook_event_name": "StopFailure",
            "error": "authentication_failed",
            "last_assistant_message": "OAuth token revoked · Please run /login",
            "session_id": CONVERSATION,
        });
        let dispatch = |event: &Value| {
            let result = run_with(
                &dispatcher,
                &["dispatch", "--product", "claude", "--format", "json"],
                &CmdOptions::new()
                    .without_ambient_managed_session_env()
                    .with_env_remove_prefix("AGENT_HOOK_")
                    .with_env_remove_prefix("CLAUDE_")
                    .with_env_remove_prefix("ANTHROPIC_")
                    .with_cwd(&self.root)
                    .with_env("HOME", self.root.to_str().unwrap())
                    .with_env("XDG_CONFIG_HOME", config_home.to_str().unwrap())
                    .with_env(
                        "XDG_STATE_HOME",
                        self.root.join("hook-state").to_str().unwrap(),
                    )
                    .with_env(
                        "XDG_DATA_HOME",
                        self.root.join("hook-data").to_str().unwrap(),
                    )
                    .with_env("AGENT_SESSION_BIN", helper.to_str().unwrap())
                    .with_env("AGENT_SESSION_ID", &self.initial.id)
                    .with_env(
                        "AGENT_SESSION_RUNTIME_ID",
                        &launch_id(&self.initial).unwrap(),
                    )
                    .with_env(
                        "AGENT_SESSION_STATE_DIR",
                        self.context.state_dir.to_str().unwrap(),
                    )
                    .with_stdin_str(&event.to_string()),
            );
            assert_eq!(result.code, 0, "{}", result.stderr_text());
        };
        let mut subagent = payload.clone();
        subagent["agent_id"] = json!("fixture-subagent");
        dispatch(&subagent);
        crate::auth_incident::recovery::ensure_input_allowed(&self.context, &self.initial)
            .expect("subagent failure must leave the main turn unfenced");
        dispatch(&payload);
        self.assert_quarantined();
    }

    fn assert_quarantined(&self) {
        let error =
            crate::auth_incident::recovery::ensure_input_allowed(&self.context, &self.initial)
                .unwrap_err();
        assert_eq!(error.code(), "auth-quarantined");
    }

    fn poll(&self, now: i64) {
        tick(&self.context, &self.initial, &self.tmux, now);
    }

    fn recover_at(&self, now: i64) {
        let old_launch = launch_id(&self.initial).unwrap();
        fs::write(self.root.join("exit-on-probe"), "").unwrap();
        let id = self.initial.id.clone();
        let watcher = on_relaunch(&self.context, &old_launch, move |context, launch| {
            session_ready_for(context, &id, launch);
        });
        let before = log(&self.root);
        self.poll(now);
        let current = load_session_record(&self.context, &self.initial.id).unwrap();
        assert_ne!(
            launch_id(&current),
            Some(old_launch),
            "shared publication left the sibling quarantined"
        );
        watcher.join().unwrap();
        let effects = log(&self.root);
        assert!(!effects[before.len()..].contains("paste-buffer"));
        assert_eq!(effects.matches("new-session").count(), 2);
        assert_eq!(
            current.provider_resume.as_ref().unwrap().session_id,
            CONVERSATION
        );
        let shared_dir = self
            .root
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .join("alpha");
        assert_eq!(self.config_dir(), shared_dir);
        let credential: Value =
            serde_json::from_slice(&fs::read(shared_dir.join(".credentials.json")).unwrap())
                .unwrap();
        assert_eq!(credential, json!({"synthetic":true,"generation":2}));
        assert_eq!(
            crate::claude_account::generation::baseline(&current)
                .unwrap()
                .version
                .generation,
            "2"
        );
        super::super::guarded_send::assert_submitted(
            &self.context,
            &self.root,
            &self.tmux,
            &current,
        );
    }
}

#[test]
fn shared_refresh_recovers_idle_and_busy_siblings_on_one_late_publication() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = synthetic_broker(tmp.path(), &lock);
    let first = Resident::start(tmp.path(), "refreshing-session");
    for verb in ["stage", "publish"] {
        assert!(
            publisher_command(tmp.path(), verb, 1)
                .args(["--account", "beta"])
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let other = Resident::start_for_account(tmp.path(), "unrelated-account", "beta");
    other.observe_revocation();
    let siblings = [
        Resident::start(tmp.path(), "revoked-sibling-a"),
        Resident::start(tmp.path(), "revoked-sibling-b"),
    ];
    let first_log = log(&first.root);
    let first_credential = first.config_dir().join(".credentials.json");
    assert_eq!(
        first_credential,
        tmp.path()
            .canonicalize()
            .unwrap()
            .join("alpha/.credentials.json")
    );
    for sibling in &siblings {
        assert_eq!(sibling.config_dir(), first.config_dir());
        assert_eq!(
            crate::claude_account::generation::baseline(&sibling.initial)
                .unwrap()
                .version
                .generation,
            "1"
        );
    }

    // Stage the shared rotation. Both old residents fail before completion
    // is visible, and both remain alive and fenced while the publisher verifies it.
    publisher(tmp.path(), "stage", 2);
    thread::scope(|scope| {
        for sibling in &siblings {
            scope.spawn(move || sibling.observe_revocation());
        }
    });
    for sibling in &siblings {
        sibling.poll(100);
        sibling.poll(130);
        sibling.assert_quarantined();
    }
    let [idle, busy] = &siblings;
    let busy_launch = launch_id(&busy.initial).unwrap();
    claude_hook_for(
        &busy.context,
        &busy.initial.id,
        &busy_launch,
        json!({"hook_event_name":"UserPromptSubmit","prompt":"synthetic turn",
            "session_id":CONVERSATION}),
    );
    publisher(tmp.path(), "publish", 2);
    busy.poll(219);
    busy.assert_quarantined();
    assert_eq!(log(&busy.root).matches("new-session").count(), 1);
    let board = crate::board::snapshot(&busy.context, &busy.tmux, "fixture", false).unwrap();
    assert_eq!(
        board["records"][0]["auth_recovery"]["state"],
        "deferred_until_idle"
    );
    assert_eq!(board["auth_recovery_summary"][0]["re_verified"], 1);
    assert_eq!(board["auth_recovery_summary"][0]["deferred_until_idle"], 1);
    idle.recover_at(219);
    let other_before = log(&other.root);
    other.poll(219);
    other.assert_quarantined();
    assert_eq!(log(&other.root), other_before);
    assert_eq!(
        fs::read_to_string(tmp.path().join("provider.log")).unwrap(),
        "probe\nprobe\n"
    );
    claude_hook_for(
        &busy.context,
        &busy.initial.id,
        &busy_launch,
        json!({"hook_event_name":"Stop","session_id":CONVERSATION}),
    );
    busy.recover_at(235);
    assert_eq!(
        fs::read_to_string(tmp.path().join("provider.log")).unwrap(),
        "probe\nprobe\n"
    );
    assert_eq!(log(&first.root), first_log);
    assert_eq!(
        launch_id(&load_session_record(&first.context, &first.initial.id).unwrap()),
        launch_id(&first.initial)
    );
    assert!(crate::auth_incident::view(&first.context, &first.initial).is_none());
}

#[test]
fn shared_refresh_rejects_wrong_account_and_expires_before_late_adoption() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::tempdir().unwrap();
    let _broker = synthetic_broker(tmp.path(), &lock);
    let sibling = Resident::start(tmp.path(), "revoked-sibling");
    sibling.observe_revocation();
    sibling.poll(100);
    sibling.poll(130);
    publisher(tmp.path(), "stage", 2);
    publisher(tmp.path(), "publish", 2);
    let receipt_path = tmp.path().join("publication-alpha.json");
    let correct = fs::read(&receipt_path).unwrap();
    let mut wrong: Value = serde_json::from_slice(&correct).unwrap();
    wrong["account"] = json!("other-account");
    fs::write(&receipt_path, wrong.to_string()).unwrap();
    sibling.poll(219);
    sibling.assert_quarantined();
    assert_eq!(log(&sibling.root).matches("new-session").count(), 1);
    fs::write(&receipt_path, correct).unwrap();
    sibling.poll(220);
    sibling.assert_quarantined();
    assert_eq!(log(&sibling.root).matches("new-session").count(), 1);
}
