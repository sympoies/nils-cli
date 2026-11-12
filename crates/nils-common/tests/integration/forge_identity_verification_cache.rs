//! Actor verification is reused across managed calls in one process.
use nils_common::forge_identity::{self as identity, LoadedPolicy, Operation, Target};
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
use pretty_assertions::assert_eq;
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Output};
use std::{fs, path::Path};

const FIXTURE: &str = include_str!("../fixtures/identity/policy.toml");
const TOKEN_ONE: &str = "FIXTURE_CACHE_TOKEN_ONE_71";
const TOKEN_TWO: &str = "FIXTURE_CACHE_TOKEN_TWO_72";

// Guards drop in declaration order: restore the environment before releasing the lock.
struct Harness {
    home: tempfile::TempDir,
    bins: StubBinDir,
    policy: LoadedPolicy,
    _env: Vec<EnvGuard>,
    _lock: GlobalStateLock,
}

impl Harness {
    fn new() -> Self {
        let lock = GlobalStateLock::new();
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join("forge-cli")).unwrap();
        fs::write(home.path().join("forge-cli/identity.toml"), FIXTURE).unwrap();
        let bins = StubBinDir::new();
        bins.write_exe(
            "gh",
            "#!/bin/sh\nprintf 'probe\\n' >> \"$FIXTURE_PROBE_LOG\"\nsleep \"${FIXTURE_PROBE_DELAY:-0}\"\nprintf '{\"login\":\"%s\"}' \"${FIXTURE_ACTOR:-account-a}\"\n",
        );
        let env = vec![
            EnvGuard::set(&lock, "XDG_CONFIG_HOME", home.path().to_str().unwrap()),
            EnvGuard::set(
                &lock,
                "XDG_STATE_HOME",
                home.path().join("state").to_str().unwrap(),
            ),
            EnvGuard::set(&lock, "FORGE_IDENTITY_PRINCIPAL", "contributor"),
            EnvGuard::set(&lock, "FIXTURE_ACCOUNT_A_CREDENTIAL", TOKEN_ONE),
            EnvGuard::set(
                &lock,
                "FIXTURE_PROBE_LOG",
                home.path().join("probes").to_str().unwrap(),
            ),
            EnvGuard::remove(&lock, "FIXTURE_ACTOR"),
            EnvGuard::remove(&lock, "FIXTURE_PROBE_DELAY"),
            prepend_path(&lock, bins.path()),
        ];
        let policy = identity::load().unwrap().unwrap();
        Self {
            home,
            bins,
            policy,
            _env: env,
            _lock: lock,
        }
    }

    fn authorize(&self) -> identity::Result<identity::Authorization> {
        let target = Target::new("github.com", "sandbox/widget").unwrap();
        self.policy.authorize(
            &target,
            None,
            Operation::ApiRead,
            self.bins.path().join("gh").as_os_str(),
        )
    }

    fn probes(&self) -> usize {
        fs::read_to_string(self.home.path().join("probes"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }

    fn audit(&self) -> Vec<serde_json::Value> {
        read_audit(
            &self
                .home
                .path()
                .join("state/forge-cli/identity-audit.jsonl"),
        )
    }
}

fn read_audit(path: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn repeated_managed_calls_probe_the_actor_once() {
    let h = Harness::new();
    for _ in 0..5 {
        assert_eq!(h.authorize().unwrap().actor, "account-a");
    }
    assert_eq!(h.probes(), 1);
}

#[test]
fn a_different_token_is_verified_again() {
    let h = Harness::new();
    h.authorize().unwrap();
    h.authorize().unwrap();
    assert_eq!(h.probes(), 1);
    let _rotated = EnvGuard::set(&h._lock, "FIXTURE_ACCOUNT_A_CREDENTIAL", TOKEN_TWO);
    h.authorize().unwrap();
    assert_eq!(h.probes(), 2);
}

fn failed(stderr: &str) -> Output {
    Output {
        status: ExitStatus::from_raw(1 << 8),
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

#[test]
fn a_rejected_credential_is_verified_again_but_other_failures_are_not() {
    let h = Harness::new();
    let auth = h.authorize().unwrap();
    auth.observe_output(&failed("gh: Not Found (HTTP 404)"));
    auth.observe_output(&Output {
        status: ExitStatus::from_raw(0),
        stdout: b"HTTP 401 quoted in a successful body".to_vec(),
        stderr: Vec::new(),
    });
    h.authorize().unwrap();
    assert_eq!(h.probes(), 1);
    for rejection in [
        "gh: Bad credentials (HTTP 401)",
        "HTTP 403: Forbidden (url)",
    ] {
        auth.observe_output(&failed(rejection));
        h.authorize().unwrap();
    }
    assert_eq!(h.probes(), 3);
}

#[test]
fn a_mismatch_is_never_cached_as_success() {
    let h = Harness::new();
    {
        let _wrong = EnvGuard::set(&h._lock, "FIXTURE_ACTOR", "someone-else");
        for _ in 0..2 {
            assert_eq!(h.authorize().err().unwrap().code, "identity_actor_mismatch");
        }
    }
    assert_eq!(h.probes(), 2);
    assert_eq!(h.authorize().unwrap().actor, "account-a");
    assert_eq!(h.probes(), 3);
    h.authorize().unwrap();
    assert_eq!(h.probes(), 3);
}

#[test]
fn the_audit_record_notes_a_cached_verification_and_when_it_happened() {
    let h = Harness::new();
    h.authorize().unwrap();
    h.authorize().unwrap();
    let authorized: Vec<_> = h
        .audit()
        .into_iter()
        .filter(|record| record["outcome"] == "authorized")
        .collect();
    assert_eq!(authorized.len(), 2);
    assert_eq!(authorized[0]["actor_verification"]["cached"], false);
    assert_eq!(authorized[1]["actor_verification"]["cached"], true);
    assert_eq!(
        authorized[1]["actor_verification"]["verified_at"],
        authorized[0]["actor_verification"]["verified_at"]
    );
    assert!(
        authorized[1]["actor_verification"]["verified_at"]
            .as_str()
            .is_some_and(|at| !at.is_empty())
    );
    let audit =
        fs::read_to_string(h.home.path().join("state/forge-cli/identity-audit.jsonl")).unwrap();
    assert!(!audit.contains(TOKEN_ONE));
}

#[test]
fn concurrent_managed_calls_share_one_probe() {
    let h = Harness::new();
    let _delay = EnvGuard::set(&h._lock, "FIXTURE_PROBE_DELAY", "0.3");
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| assert_eq!(h.authorize().unwrap().actor, "account-a"));
        }
    });
    assert_eq!(h.probes(), 1);
}

#[test]
fn a_verification_is_not_reused_under_a_different_audit_state_root() {
    let h = Harness::new();
    h.authorize().unwrap();
    h.authorize().unwrap();
    assert_eq!(h.probes(), 1);
    let other = h.home.path().join("other-state");
    let _state = EnvGuard::set(&h._lock, "XDG_STATE_HOME", other.to_str().unwrap());
    h.authorize().unwrap();
    assert_eq!(h.probes(), 2);
    let record = &read_audit(&other.join("forge-cli/identity-audit.jsonl"))[0];
    assert_eq!(record["actor_verification"]["cached"], false);
}
