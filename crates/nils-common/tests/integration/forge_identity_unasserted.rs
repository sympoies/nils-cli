use nils_common::forge_identity as identity;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use std::fs;

const FIXTURE: &str = include_str!("../fixtures/identity/policy.toml");

#[test]
fn noninteractive_detection_rules() {
    let none = |_: &str| false;
    assert!(!identity::is_noninteractive_with(true, true, none));
    assert!(identity::is_noninteractive_with(false, true, none));
    assert!(identity::is_noninteractive_with(true, false, none));
    for marker in ["INVOCATION_ID", "GITHUB_ACTIONS", "AGENT_SESSION_ID"] {
        assert!(
            identity::is_noninteractive_with(true, true, |name| name == marker),
            "{marker}"
        );
    }
    assert!(!identity::is_noninteractive_with(true, true, |name| name == "CI"));
}

struct Env {
    _lock: GlobalStateLock,
    _guards: Vec<EnvGuard>,
    root: tempfile::TempDir,
}
impl Env {
    /// Asserted-only policy, no principal, automation marker carrying a canary value.
    fn new(key: Option<&str>) -> Self {
        let lock = GlobalStateLock::new();
        let root = tempfile::tempdir().unwrap();
        let guards = vec![
            EnvGuard::set(
                &lock,
                "XDG_CONFIG_HOME",
                root.path().join("cfg").to_str().unwrap(),
            ),
            EnvGuard::set(
                &lock,
                "XDG_STATE_HOME",
                root.path().join("state").to_str().unwrap(),
            ),
            EnvGuard::remove(&lock, "FORGE_IDENTITY_PRINCIPAL"),
            EnvGuard::set(&lock, "AGENT_SESSION_ID", "SESSION_CANARY"),
        ];
        fs::create_dir_all(root.path().join("cfg/forge-cli")).unwrap();
        let key = key
            .map(|k| format!("unasserted_noninteractive='{k}'\n"))
            .unwrap_or_default();
        fs::write(
            root.path().join("cfg/forge-cli/identity.toml"),
            format!("activation='asserted-only'\n{key}{FIXTURE}"),
        )
        .unwrap();
        identity::reset_unasserted_notice_for_tests();
        Self {
            _lock: lock,
            _guards: guards,
            root,
        }
    }
    fn audit(&self) -> Option<String> {
        fs::read_to_string(
            self.root
                .path()
                .join("state/forge-cli/identity-audit.jsonl"),
        )
        .ok()
    }
}

#[test]
fn allow_passes_through_silently_without_audit() {
    let env = Env::new(Some("allow"));
    assert!(identity::load().unwrap().is_none());
    assert!(env.audit().is_none());
}

#[test]
fn warn_is_default_passes_through_and_audits_once_with_caller_only() {
    for key in [None, Some("warn")] {
        let env = Env::new(key);
        assert!(identity::load().unwrap().is_none());
        assert!(identity::load().unwrap().is_none());
        let audit = env.audit().unwrap();
        let lines: Vec<_> = audit.lines().collect();
        assert_eq!(lines.len(), 1, "one notice per process");
        let record: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(record["outcome"], "unasserted_noninteractive_warned");
        assert!(record["principal"].is_null());
        let caller = record["caller"].as_object().unwrap();
        let mut keys: Vec<_> = caller.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["argv0", "parent"]);
        for value in caller.values() {
            let text = value.as_str().unwrap();
            assert!(!text.is_empty() && !text.contains('/'), "{text}");
        }
        assert!(!audit.contains("SESSION_CANARY"));
        assert!(!audit.contains(env.root.path().to_str().unwrap()));
    }
}

#[test]
fn deny_fails_with_stable_code_fix_text_and_audit() {
    let env = Env::new(Some("deny"));
    let error = identity::load().err().unwrap();
    assert_eq!(error.code, "identity_unasserted_noninteractive");
    let text = error.to_string();
    assert!(text.contains("FORGE_IDENTITY_PRINCIPAL"), "{text}");
    assert!(text.contains("unasserted_noninteractive"), "{text}");
    assert!(text.contains(identity::UNASSERTED_FIX_TEXT));
    let audit = env.audit().unwrap();
    assert!(audit.contains("unasserted_noninteractive_denied"));
    assert!(!audit.contains("SESSION_CANARY"));
}

#[test]
fn asserted_principal_is_unaffected_in_every_mode() {
    for key in ["allow", "warn", "deny"] {
        let env = Env::new(Some(key));
        let _principal = EnvGuard::set(&env._lock, "FORGE_IDENTITY_PRINCIPAL", "contributor");
        assert!(identity::load().unwrap().is_some());
        assert!(env.audit().is_none());
    }
}

#[test]
fn diagnostics_loader_does_not_gate_or_audit() {
    let env = Env::new(Some("deny"));
    assert!(identity::load_ungated().unwrap().is_none());
    assert!(env.audit().is_none());
}

#[test]
fn unknown_mode_value_is_an_invalid_policy() {
    let env = Env::new(Some("sometimes"));
    assert_eq!(
        identity::load().err().unwrap().code,
        "identity_policy_invalid"
    );
    assert!(env.audit().is_none());
}
