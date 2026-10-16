use super::*;
use nils_test_support::cmd::{CmdOptions, run_resolved};
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;

const POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "fixture-policy"
version = "2026.10.11.1"

[[rules]]
id = "fixture.codex-attention"
products = ["codex"]
events = ["PermissionRequest"]
priority = 10
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }
"#;

struct Fixture {
    temp: tempfile::TempDir,
    home: std::path::PathBuf,
    _env: Vec<EnvGuard>,
}

impl Fixture {
    fn new(lock: &GlobalStateLock) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let home = root.join("home");
        let config_home = root.join("config");
        let state_home = root.join("state");
        fs::create_dir_all(config_home.join("agent-hook")).unwrap();
        fs::create_dir_all(&home).unwrap();
        let policy = root.join("policy.toml");
        fs::write(&policy, POLICY).unwrap();
        let config = config_home.join("agent-hook/config.toml");
        fs::write(
            &config,
            format!(
                "schema_version = \"agent-hook.config.v1\"\n[policy]\npath = {}\ndigest = \"sha256:{}\"\n",
                serde_json::to_string(&policy).unwrap(),
                hex_digest(Sha256::digest(POLICY.as_bytes()))
            ),
        )
        .unwrap();
        for path in [&policy, &config] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let helper = nils_test_support::bin::resolve("agent-session");
        // Fail here, naming the build command, on a missing or earlier-release
        // dispatcher; the run_resolved calls below reuse this artifact.
        let dispatcher = nils_test_support::bin::required_sibling("agent-hook", "nils-agent-hook");
        let env = vec![
            EnvGuard::set(lock, "HOME", home.to_str().unwrap()),
            EnvGuard::set(lock, "XDG_CONFIG_HOME", config_home.to_str().unwrap()),
            EnvGuard::set(lock, "XDG_STATE_HOME", state_home.to_str().unwrap()),
            EnvGuard::remove(lock, "CODEX_HOME"),
            EnvGuard::set(lock, "AGENT_SESSION_BIN", helper.to_str().unwrap()),
            EnvGuard::set(lock, "AGENT_HOOK_BIN", dispatcher.to_str().unwrap()),
        ];
        assert_eq!(
            agent_hook::run_with_args([
                "agent-hook",
                "setup",
                "--product",
                "codex",
                "--apply",
                "--format",
                "json",
            ]),
            0,
            "existing owned setup must seed the fixture"
        );
        let doctor = run_resolved(
            "agent-hook",
            &["doctor", "--product", "codex", "--format", "json"],
            &CmdOptions::new()
                .without_ambient_managed_session_env()
                .with_env("AGENT_SESSION_BIN", helper.to_str().unwrap()),
        );
        assert_eq!(doctor.code, 0, "{}", doctor.stdout_text());
        assert_eq!(doctor.stdout_json()["data"][0]["status"], "converged");
        Self {
            temp,
            home,
            _env: env,
        }
    }

    fn session(&self, authority: &str) -> (CliContext, SessionRecord) {
        // Existing passing activity setup owns the runtime/replay fixture.
        // Selecting synthetic protocol authority here does not audit a version.
        let (context, mut created) = test_session(&self.temp);
        let runtime = created.record.runtime.as_mut().unwrap();
        runtime.kind = "codex_app_server".into();
        runtime
            .extra
            .insert("codex_attention_authority".into(), json!(authority));
        write_session_record(&context, &created.record).unwrap();
        activate_runtime(&context, &created.record).unwrap();
        let mut started = event(TurnEventKind::TurnStarted, "fixture-start");
        started.runtime_id = runtime_id(&created.record).into();
        ingest_event(&context, &created.record.id, started).unwrap();
        (context, created.record)
    }

    fn permission(
        &self,
        context: &CliContext,
        record: &SessionRecord,
        authority: &str,
    ) -> nils_test_support::cmd::CmdOutput {
        let helper = nils_test_support::bin::resolve("agent-session");
        run_resolved(
            "agent-hook",
            &["dispatch", "--product", "codex", "--format", "json"],
            &CmdOptions::new()
                .without_ambient_managed_session_env()
                .with_cwd(self.temp.path())
                .with_env("AGENT_SESSION_ID", &record.id)
                .with_env("AGENT_SESSION_RUNTIME_ID", runtime_id(record))
                .with_env("AGENT_SESSION_STATE_DIR", context.state_dir.to_str().unwrap())
                .with_env("AGENT_SESSION_BIN", helper.to_str().unwrap())
                .with_env("AGENT_SESSION_ATTENTION_AUTHORITY", authority)
                .with_stdin_str(r#"{"hook_event_name":"PermissionRequest","session_id":"session-1","turn_id":"turn-1","tool_name":"Bash","tool_input":{"command":"synthetic-private-command"}}"#),
        )
    }
}

fn runtime_id(record: &SessionRecord) -> &str {
    &record.runtime.as_ref().unwrap().launch_id
}

#[test]
fn source_guard_accepts_owned_dispatch_without_retired_direct_reporter() {
    let lock = GlobalStateLock::new();
    let fixture = Fixture::new(&lock);
    let path = fixture.home.join(".codex/config.toml");
    let before = fs::read(&path).unwrap();
    assert!(codex_protocol_attention_source_guard_configured());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn source_guard_rejects_an_unguarded_direct_reporter_added_to_dispatch() {
    let lock = GlobalStateLock::new();
    let fixture = Fixture::new(&lock);
    let path = fixture.home.join(".codex/config.toml");
    let mut config = fs::read_to_string(&path).unwrap();
    config.push_str("\n[[hooks.PermissionRequest]]\n[[hooks.PermissionRequest.hooks]]\ntype = \"command\"\ncommand = \"agent-session activity hook --agent codex\"\ntimeout = 5\n");
    fs::write(&path, config).unwrap();
    assert!(!codex_protocol_attention_source_guard_configured());
}

#[test]
fn dispatch_suppresses_generic_permission_for_protocol_authority() {
    let lock = GlobalStateLock::new();
    let fixture = Fixture::new(&lock);
    let (context, record) = fixture.session("protocol");
    let path = session_dir(&context, &record.id).join(ACTIVITY_FILE);
    let before = fs::read(&path).unwrap();
    let output = fixture.permission(&context, &record, "protocol");
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(fs::read(path).unwrap(), before);
    let state = state_for_view(&context, &record).unwrap();
    assert_eq!(state.phase, TurnPhase::Working);
    assert!(state.current_turn.unwrap().attention.is_none());
}

#[test]
fn dispatch_retains_conservative_approval_for_hook_authority() {
    let lock = GlobalStateLock::new();
    let fixture = Fixture::new(&lock);
    let (context, record) = fixture.session("hook");
    let output = fixture.permission(&context, &record, "hook");
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let state = state_for_view(&context, &record).unwrap();
    assert_eq!(state.phase, TurnPhase::NeedsInput);
    assert_eq!(
        state.current_turn.unwrap().attention.unwrap().kind,
        "approval"
    );
    assert!(!output.stdout_text().contains("synthetic-private-command"));
}

#[test]
fn dispatch_does_not_bypass_mismatched_runtime_attention_authority() {
    let lock = GlobalStateLock::new();
    let fixture = Fixture::new(&lock);
    let (context, record) = fixture.session("protocol");
    let output = fixture.permission(&context, &record, "hook");
    // Advisory activity failure reports a warning rather than denying work;
    // the owning helper must still poison the mismatched runtime's state.
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(output.stdout_json()["data"]["action"], "warn");
    assert!(
        output.stdout_json()["data"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason["code"] == "session-activity-failed")
    );
    let state = state_for_view(&context, &record).unwrap();
    assert_eq!(state.phase, TurnPhase::Unknown);
    assert_eq!(
        state.diagnostic.unwrap().reason,
        "attention_authority_mismatch"
    );
}
