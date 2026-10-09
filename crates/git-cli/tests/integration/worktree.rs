use crate::common;
use common::{GitCliHarness, git, init_bare_remote, init_repo};
use nils_test_support::cmd::{CmdOutput, run_with};
use nils_test_support::git::git_output;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn run_with_agent_home(
    harness: &GitCliHarness,
    cwd: &Path,
    agent_home: &Path,
    args: &[&str],
) -> CmdOutput {
    let agent_home = agent_home.to_string_lossy().to_string();
    let options = harness.cmd_options(cwd).with_env("AGENT_HOME", &agent_home);
    run_with(&harness.git_cli_bin(), args, &options)
}

fn parse_json(output: &CmdOutput) -> Value {
    serde_json::from_str(output.stdout_text().trim()).expect("valid json output")
}

#[test]
fn worktree_remove_retains_dirty_content_without_force() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");
    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree", "add", "retained", "--from", "main", "--format", "json",
        ],
    );
    assert_eq!(add.code, 0);
    let target = parse_json(&add)["data"]["path"]
        .as_str()
        .unwrap()
        .to_string();
    fs::write(
        Path::new(&target).join("unfinished.txt"),
        "retain this content",
    )
    .unwrap();
    let result = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "remove", "retained", "--format", "json"],
    );
    assert_ne!(result.code, 0, "dirty content must survive removal");
    assert_eq!(
        fs::read_to_string(Path::new(&target).join("unfinished.txt")).unwrap(),
        "retain this content"
    );
}

struct RemovalFixture {
    harness: GitCliHarness,
    repo: tempfile::TempDir,
    _remote: tempfile::TempDir,
    home: tempfile::TempDir,
    probes: nils_test_support::StubBinDir,
    target: String,
}

impl RemovalFixture {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let harness = GitCliHarness::new();
        let repo = init_repo();
        let remote = init_bare_remote();
        git(
            repo.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        git(repo.path(), &["push", "-u", "origin", "main"]);
        git(remote.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
        let home = tempfile::TempDir::new().unwrap();
        for directory in [
            home.path().join("lease-state"),
            home.path().join("sessions"),
            home.path().join("sessions/coordination"),
        ] {
            fs::create_dir_all(&directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let add = run_with_agent_home(
            &harness,
            repo.path(),
            home.path(),
            &[
                "worktree", "add", "safe", "--from", "main", "--format", "json",
            ],
        );
        assert_eq!(add.code, 0, "{}", add.stderr_text());
        let target = parse_json(&add)["data"]["path"]
            .as_str()
            .unwrap()
            .to_string();
        let probes = nils_test_support::StubBinDir::new();
        probes.write_exe("lsof", "#!/bin/sh\nexit 1\n");
        probes.write_exe(
            "agent-session",
            "#!/bin/sh\nprintf '%s\\n' '{\"ok\":true,\"data\":[]}'\n",
        );
        probes.write_exe(
            "forge-cli",
            "#!/bin/sh\nprintf '%s\\n' '{\"ok\":true,\"data\":{\"items\":[]}}'\n",
        );
        Self {
            harness,
            repo,
            _remote: remote,
            home,
            probes,
            target,
        }
    }
    fn remove(&self, target: &str) -> CmdOutput {
        let options = self
            .harness
            .cmd_options(self.repo.path())
            .with_path_prepend(self.probes.path())
            .with_env("AGENT_HOME", self.home.path().to_str().unwrap())
            .with_env(
                "AGENT_RUNTIME_CHECKOUT_LEASE_STATE_HOME",
                self.home.path().join("lease-state").to_str().unwrap(),
            )
            .with_env(
                "AGENT_SESSION_STATE_DIR",
                self.home.path().join("sessions").to_str().unwrap(),
            )
            .with_env("AGENT_SESSION_COORDINATION_MODE", "advisory");
        run_with(
            &self.harness.git_cli_bin(),
            &["worktree", "remove", target, "--safe", "--format", "json"],
            &options,
        )
    }
    fn refused(&self, code: &str) {
        let result = self.remove("safe");
        assert_ne!(result.code, 0, "{}", result.stdout_text());
        assert_eq!(
            parse_json(&result)["error"]["code"],
            code,
            "{}",
            result.stdout_text()
        );
        assert!(Path::new(&self.target).exists());
    }
    fn cleanup(&self) -> CmdOutput {
        let options = self
            .harness
            .cmd_options(self.repo.path())
            .with_path_prepend(self.probes.path())
            .with_env("AGENT_HOME", self.home.path().to_str().unwrap())
            .with_env(
                "AGENT_RUNTIME_CHECKOUT_LEASE_STATE_HOME",
                self.home.path().join("lease-state").to_str().unwrap(),
            )
            .with_env(
                "AGENT_SESSION_STATE_DIR",
                self.home.path().join("sessions").to_str().unwrap(),
            )
            .with_env("AGENT_SESSION_COORDINATION_MODE", "advisory")
            .with_stdin_str("y\n");
        run_with(
            &self.harness.git_cli_bin(),
            &["branch", "cleanup", "--remove-worktrees"],
            &options,
        )
    }
    fn commit(&self) {
        fs::write(Path::new(&self.target).join("new.txt"), "new commit").unwrap();
        git(Path::new(&self.target), &["add", "."]);
        git(
            Path::new(&self.target),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "test",
            ],
        );
    }
}

#[test]
fn safe_removal_queries_the_explicit_session_state_root() {
    let fixture = RemovalFixture::new();
    fs::write(
        fixture.home.path().join("sessions/live-sessions.json"),
        serde_json::to_vec(&serde_json::json!({
            "ok": true,
            "data": [{ "status": "running", "cwd": fixture.target }]
        }))
        .unwrap(),
    )
    .unwrap();
    fixture.probes.write_exe(
        "agent-session",
        "#!/bin/sh\nif [ \"$1\" = --state-dir ] && [ \"$2\" = \"$AGENT_SESSION_STATE_DIR\" ] && [ \"$3\" = list ]; then\n cat \"$AGENT_SESSION_STATE_DIR/live-sessions.json\"\nelse\n printf '%s\\n' '{\"ok\":true,\"data\":[]}'\nfi\n",
    );
    fixture.refused("removal-session-active");
}

#[test]
fn safe_removal_retains_target_behind_startup_lifecycle_barrier() {
    use sha2::{Digest, Sha256};
    use std::os::fd::AsRawFd;
    use std::os::unix::{
        ffi::OsStrExt,
        fs::{OpenOptionsExt, PermissionsExt},
    };

    let fixture = RemovalFixture::new();
    let root = fs::canonicalize(&fixture.target).unwrap();
    let key = Sha256::digest(root.as_os_str().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let directory = fixture
        .home
        .path()
        .join("sessions/coordination/worktree-lifecycle");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(directory.join(format!("{key}.lock")))
        .unwrap();
    // A startup that has not registered yet owns this barrier. Removal must
    // retain the checkout even though the session and process snapshots are idle.
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    fixture.refused("removal-lifecycle-busy");
    drop(lock);
    let result = fixture.remove("safe");
    assert_eq!(result.code, 0, "{}", result.stdout_text());
    assert!(!root.exists());
}

#[test]
fn safe_removal_branch_cleanup_proves_managed_clean_success_and_dirty_retention() {
    for dirty in [false, true] {
        let fixture = RemovalFixture::new();
        let unfinished = Path::new(&fixture.target).join("unfinished.txt");
        if dirty {
            fs::write(&unfinished, "retain").unwrap();
        }
        let output = fixture.cleanup();
        if dirty {
            assert_ne!(output.code, 0, "{}", output.stdout_text());
            assert!(
                output.stderr_text().contains("removal-dirty"),
                "{}",
                output.stderr_text()
            );
            assert_eq!(fs::read_to_string(unfinished).unwrap(), "retain");
            assert!(
                git(fixture.repo.path(), &["branch", "--list", "feat/safe"]).contains("feat/safe")
            );
        } else {
            assert_eq!(
                output.code,
                0,
                "{} {}",
                output.stdout_text(),
                output.stderr_text()
            );
            assert!(!Path::new(&fixture.target).exists());
            assert!(
                git(fixture.repo.path(), &["branch", "--list", "feat/safe"])
                    .trim()
                    .is_empty()
            );
        }
    }
}

#[test]
fn safe_removal_holds_lifecycle_guard_through_final_process_proof() {
    use nils_common::worktree_lifecycle::{Error, Guard};
    use std::time::{Duration, Instant};
    let fixture = RemovalFixture::new();
    let marker = fixture.home.path().join("proof-entered");
    let release = fixture.home.path().join("proof-release");
    let quote = nils_common::shell::quote_posix_single;
    fixture.probes.write_exe("lsof", &format!(
        "#!/bin/sh\ntouch {}\ncount=0\nwhile [ ! -f {} ] && [ \"$count\" -lt 500 ]; do sleep 0.01; count=$((count+1)); done\nexit 1\n",
        quote(marker.to_str().unwrap()), quote(release.to_str().unwrap()),
    ));
    std::thread::scope(|scope| {
        let removing = scope.spawn(|| fixture.remove("safe"));
        let deadline = Instant::now() + Duration::from_secs(4);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let held = Guard::acquire(
            &fixture.home.path().join("sessions"),
            Path::new(&fixture.target),
        );
        fs::write(&release, "release").unwrap();
        let result = removing.join().unwrap();
        assert!(marker.exists(), "{}", result.stdout_text());
        assert!(
            matches!(held, Err(Error::Busy)),
            "removal dropped guard before deletion"
        );
        assert_eq!(result.code, 0, "{}", result.stdout_text());
        assert!(!Path::new(&fixture.target).exists());
    });
}

#[test]
fn safe_removal_idle_clean_merged_succeeds_in_advisory() {
    let fixture = RemovalFixture::new();
    let removed_head = git(Path::new(&fixture.target), &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    let result = fixture.remove("safe");
    assert_eq!(
        result.code,
        0,
        "{} {}",
        result.stdout_text(),
        result.stderr_text()
    );
    assert!(!Path::new(&fixture.target).exists());
    let receipt = parse_json(&result);
    assert_eq!(receipt["data"]["removed_branch"], "feat/safe");
    assert_eq!(receipt["data"]["removed_head"], removed_head);
    assert_eq!(
        receipt["data"]["delivery_proof"]["basis"],
        "remote-default-ancestry"
    );
    assert_eq!(receipt["data"]["delivery_proof"]["default_branch"], "main");
    assert_eq!(
        receipt["data"]["delivery_proof"]["default_head"],
        removed_head
    );
    assert_eq!(
        git(
            fixture.repo.path(),
            &["rev-parse", "--verify", "refs/heads/feat/safe"]
        )
        .trim()
        .len(),
        40
    );
}

#[test]
fn safe_removal_accepts_freshly_pushed_merged_commit() {
    let fixture = RemovalFixture::new();
    fixture.commit();
    git(Path::new(&fixture.target), &["push", "origin", "HEAD"]);
    git(fixture.repo.path(), &["merge", "--ff-only", "feat/safe"]);
    git(fixture.repo.path(), &["push", "origin", "main"]);
    let result = fixture.remove("safe");
    assert_eq!(result.code, 0, "{}", result.stdout_text());
    assert!(!Path::new(&fixture.target).exists());
}

#[test]
fn safe_removal_requires_exact_provider_head_for_squash_merge() {
    for exact in [true, false] {
        let fixture = RemovalFixture::new();
        fixture.commit();
        let head = git(Path::new(&fixture.target), &["rev-parse", "HEAD"])
            .trim()
            .to_string();
        git(Path::new(&fixture.target), &["push", "origin", "HEAD"]);
        // Distinct parentage means ancestry cannot stand in for provider proof.
        let listed = serde_json::json!({"ok":true,"data":{"items":[{"number":1}]}});
        let viewed = serde_json::json!({"ok":true,"data":{"state":"merged","head":"feat/safe","base":"main",
            "head_sha":if exact { head.clone() } else { "0".repeat(40) }, "merged_at":"2030-01-01T00:00:00Z"}});
        fixture.probes.write_exe("forge-cli", &format!("#!/bin/sh\ncase \"$2\" in\n list) printf '%s\\n' '{listed}' ;;\n view) printf '%s\\n' '{viewed}' ;;\n *) exit 2 ;;\nesac\n"));
        let result = fixture.remove("safe");
        if exact {
            assert_eq!(result.code, 0, "{}", result.stdout_text());
            assert!(!Path::new(&fixture.target).exists());
            let receipt = parse_json(&result);
            assert_eq!(receipt["data"]["removed_head"], head);
            assert_eq!(receipt["data"]["removed_branch"], "feat/safe");
            assert_eq!(
                receipt["data"]["delivery_proof"]["basis"],
                "provider-exact-head-merge"
            );
            assert_eq!(receipt["data"]["delivery_proof"]["default_branch"], "main");
            assert_eq!(
                receipt["data"]["delivery_proof"]["default_head"],
                git(fixture._remote.path(), &["rev-parse", "refs/heads/main"]).trim()
            );
            assert_eq!(receipt["data"]["delivery_proof"]["pr_number"], 1);
        } else {
            assert_eq!(
                parse_json(&result)["error"]["code"],
                "removal-head-undelivered"
            );
            assert!(Path::new(&fixture.target).exists());
        }
    }
}

#[test]
fn safe_removal_fences_active_claim_and_nonterminal_operation_bindings() {
    use std::os::unix::fs::PermissionsExt;
    for claim_state in ["active", "released"] {
        let fixture = RemovalFixture::new();
        let key = "f".repeat(64);
        let fingerprint = nils_common::coordination_projection::worktree_fingerprint(
            1,
            &key,
            Path::new(&fixture.target),
        )
        .unwrap();
        let registry = serde_json::json!({"schema_version":"agent-session.coordination-registry.v2",
            "fingerprint_epoch":1,"fingerprint_key":key,"brokers":{},
            "claims":[{"schema_version":"agent-session.work-context.v1","claim_id":"claim",
                "session_id":"holder","session_incarnation":"holder-incarnation", "state":claim_state,
                "worktrees":[fingerprint],"expires_at_epoch":4102444800i64}],
            "operations":if claim_state == "released" { vec![serde_json::json!({"schema_version":"agent-session.operation-lease.v1", "claim_id":"claim", "state":"reconcile_pending"})] } else { vec![] }});
        let path = fixture
            .home
            .path()
            .join("sessions/coordination/registry.json");
        fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fixture.refused("removal-session-active");
    }
}

#[test]
fn safe_removal_allows_known_terminal_claim_states() {
    use std::os::unix::fs::PermissionsExt;
    for state in ["released", "expired", "stale"] {
        let fixture = RemovalFixture::new();
        let key = "a".repeat(64);
        let fingerprint = nils_common::coordination_projection::worktree_fingerprint(
            1,
            &key,
            Path::new(&fixture.target),
        )
        .unwrap();
        let registry = serde_json::json!({"schema_version":"agent-session.coordination-registry.v2",
            "fingerprint_epoch":1,"fingerprint_key":key,"brokers":{},
            "claims":[{"schema_version":"agent-session.work-context.v1","claim_id":"claim",
                "session_id":"holder","session_incarnation":"holder-incarnation", "state":state,
                "worktrees":[fingerprint],"expires_at_epoch":1}],"operations":[]});
        let path = fixture
            .home
            .path()
            .join("sessions/coordination/registry.json");
        fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let result = fixture.remove("safe");
        assert_eq!(result.code, 0, "{}", result.stdout_text());
        assert!(!Path::new(&fixture.target).exists());
    }
}

#[test]
fn safe_removal_retains_semantically_unknown_ownership_state() {
    use std::os::unix::fs::PermissionsExt;
    for unknown_claim in [true, false] {
        let fixture = RemovalFixture::new();
        let key = "a".repeat(64);
        let fingerprint = nils_common::coordination_projection::worktree_fingerprint(
            1,
            &key,
            Path::new(&fixture.target),
        )
        .unwrap();
        let registry = serde_json::json!({"schema_version":"agent-session.coordination-registry.v2",
            "fingerprint_epoch":1,"fingerprint_key":key,"brokers":{},
            "claims":[{"schema_version":"agent-session.work-context.v1","claim_id":"claim",
                "session_id":"holder","session_incarnation":"holder-incarnation",
                "state":if unknown_claim { "future_state" } else { "released" },
                "worktrees":[fingerprint],"expires_at_epoch":4102444800i64}],
            "operations":if unknown_claim { vec![] } else { vec![serde_json::json!({"schema_version":"agent-session.operation-lease.v1", "claim_id":"claim", "state":"future_state"})] }});
        let path = fixture
            .home
            .path()
            .join("sessions/coordination/registry.json");
        fs::write(&path, serde_json::to_vec(&registry).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fixture.refused("removal-proof-unavailable");
    }
}

#[test]
fn safe_removal_retains_unknown_registry_state_and_locked_target() {
    let fixture = RemovalFixture::new();
    git(fixture.repo.path(), &["worktree", "lock", &fixture.target]);
    let result = fixture.remove("safe");
    assert_ne!(result.code, 0);
    assert!(Path::new(&fixture.target).exists());
    git(
        fixture.repo.path(),
        &["worktree", "unlock", &fixture.target],
    );
    use std::os::unix::fs::PermissionsExt;
    let registry = fixture
        .home
        .path()
        .join("sessions/coordination/registry.json");
    fs::write(&registry, "{}").unwrap();
    fs::set_permissions(&registry, fs::Permissions::from_mode(0o600)).unwrap();
    fixture.refused("removal-proof-unavailable");
}

#[test]
fn safe_removal_retains_unpushed_and_pushed_unmerged_commits() {
    for pushed in [false, true] {
        let fixture = RemovalFixture::new();
        fixture.commit();
        if pushed {
            git(Path::new(&fixture.target), &["push", "origin", "HEAD"]);
        }
        fixture.refused("removal-head-undelivered");
    }
}

#[test]
fn safe_removal_retains_live_session_binding_and_unknown_inventory() {
    let fixture = RemovalFixture::new();
    fixture.probes.write_exe(
        "agent-session",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' '{}'\n",
            serde_json::json!({"ok":true,"data":[{"status":"running","cwd":fixture.target}]})
        ),
    );
    fixture.refused("removal-session-active");
    fixture.probes.write_exe(
        "agent-session",
        "#!/bin/sh\nprintf '%s\\n' '{\"ok\":true,\"data\":{}}'\n",
    );
    fixture.refused("removal-proof-unavailable");
}

#[test]
fn safe_removal_retains_active_checkout_lease_including_requester() {
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;
    let fixture = RemovalFixture::new();
    // Initialize the sentinel and stable lock through one harmless failed proof.
    fixture
        .probes
        .write_exe("lsof", "#!/bin/sh\necho p123\nexit 0\n");
    fixture.refused("removal-process-active");
    let target = Path::new(&fixture.target).canonicalize().unwrap();
    let git_dir = PathBuf::from(git(&target, &["rev-parse", "--absolute-git-dir"]).trim());
    let common_dir = fixture.repo.path().join(".git").canonicalize().unwrap();
    let instance = fs::read_to_string(git_dir.join(".agent-runtime-checkout-instance")).unwrap();
    let directory = fixture
        .home
        .path()
        .join("lease-state")
        .join(
            Sha256::digest(common_dir.as_os_str().as_encoded_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        )
        .join(
            Sha256::digest(target.as_os_str().as_encoded_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        );
    let lease = serde_json::json!({"schema":"agent-runtime.checkout-lease.v1", "session_key":"a".repeat(64),
        "checkout_instance":instance.trim(), "checkout_root":target,"checkout_git_dir":git_dir,
        "acquired_at":1,"refreshed_at":1,"expires_at":4102444800u64});
    fs::write(
        directory.join("lease.json"),
        serde_json::to_vec(&lease).unwrap(),
    )
    .unwrap();
    fs::set_permissions(
        directory.join("lease.json"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    fixture.refused("removal-lease-active-or-unavailable");
}

#[test]
fn safe_removal_retains_process_cwd_and_open_file() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    for cwd in [true, false] {
        let fixture = RemovalFixture::new();
        // The child announces readiness only after it owns the cwd/file.
        let mut child = Command::new("python3").args(["-c", if cwd {
            "import os,sys,time; os.chdir(sys.argv[1]); print('ready',flush=True); time.sleep(30)"
        } else {
            "import sys,time; f=open(sys.argv[1]+'/README.md'); print('ready',flush=True); time.sleep(30)"
        }, &fixture.target]).stdout(Stdio::piped()).spawn().unwrap();
        let mut ready = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready.trim(), "ready");
        // Pin the probe to the actual live holder. Fixture-global mounts do not
        // influence the positive proof; absence is tested separately.
        fixture.probes.write_exe("lsof", &format!("#!/bin/sh\nexec /usr/bin/env PATH=/usr/bin:/usr/sbin:/bin:/sbin lsof -p {} -a \"$@\"\n", child.id()));
        let result = fixture.remove("safe");
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(
            parse_json(&result)["error"]["code"],
            "removal-process-active",
            "{}",
            result.stdout_text()
        );
        assert!(Path::new(&fixture.target).exists());
    }
}

#[test]
fn safe_removal_retains_incomplete_process_proof_and_unmanaged_target() {
    let fixture = RemovalFixture::new();
    fixture.probes.write_exe(
        "lsof",
        "#!/bin/sh\necho 'inventory incomplete' >&2\nexit 1\n",
    );
    fixture.refused("removal-proof-unavailable");
    let unmanaged = fixture.home.path().join("unmanaged");
    git(
        fixture.repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "unmanaged",
            unmanaged.to_str().unwrap(),
        ],
    );
    let result = fixture.remove(unmanaged.to_str().unwrap());
    assert_eq!(parse_json(&result)["error"]["code"], "removal-unmanaged");
    assert!(unmanaged.exists());
}

/// Read one config key, distinguishing "unset" from "set to the empty string".
fn git_config_optional(repo: &Path, key: &str) -> Option<String> {
    let output = git_output(repo, &["config", "--get", key]);
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[test]
fn worktree_add_creates_deterministic_agent_home_path_and_lists_json() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree",
            "add",
            "topic-one",
            "--from",
            "main",
            "--format",
            "json",
        ],
    );

    assert_eq!(add.code, 0, "stderr: {}", add.stderr_text());
    assert_eq!(add.stderr_text(), "");

    let add_json = parse_json(&add);
    assert_eq!(add_json["schema_version"], "cli.git-cli.worktree.add.v1");
    assert_eq!(add_json["ok"], true);
    assert_eq!(add_json["data"]["slug"], "topic-one");
    assert_eq!(
        add_json["data"]["kind"], "feature",
        "default kind is feature"
    );
    assert_eq!(add_json["data"]["branch"], "feat/topic-one");

    let repo_key = add_json["data"]["repo_key"].as_str().expect("repo key");
    let path = add_json["data"]["path"].as_str().expect("path");
    let canonical_agent_home = agent_home
        .path()
        .canonicalize()
        .expect("canonical agent home");
    let canonical_agent_home_text = canonical_agent_home.to_string_lossy().to_string();
    let expected_path = canonical_agent_home
        .join("worktrees")
        .join(repo_key)
        .join("topic-one");
    assert_eq!(
        add_json["data"]["agent_home"].as_str(),
        Some(canonical_agent_home_text.as_str())
    );
    assert_eq!(path, expected_path.to_string_lossy());
    assert!(expected_path.exists(), "worktree path should exist");

    let porcelain = git(repo.path(), &["worktree", "list", "--porcelain"]);
    assert!(porcelain.contains("branch refs/heads/feat/topic-one"));
    assert!(porcelain.contains(path));

    let list = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "list", "--format", "json"],
    );
    assert_eq!(list.code, 0, "stderr: {}", list.stderr_text());
    let list_json = parse_json(&list);
    assert_eq!(list_json["schema_version"], "cli.git-cli.worktree.list.v1");

    let entries = list_json["data"]["entries"]
        .as_array()
        .expect("entries array");
    let managed = entries
        .iter()
        .find(|entry| entry["path"].as_str() == Some(path))
        .expect("managed worktree listed");
    assert_eq!(managed["branch"], "feat/topic-one");
    assert_eq!(managed["managed"], true);
}

#[test]
fn worktree_add_kind_bug_uses_fix_branch_prefix() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree",
            "add",
            "topic-bug",
            "--from",
            "main",
            "--kind",
            "bug",
            "--format",
            "json",
        ],
    );

    assert_eq!(add.code, 0, "stderr: {}", add.stderr_text());
    assert_eq!(add.stderr_text(), "");

    let add_json = parse_json(&add);
    assert_eq!(add_json["ok"], true);
    assert_eq!(add_json["data"]["slug"], "topic-bug");
    assert_eq!(add_json["data"]["kind"], "bug");
    assert_eq!(
        add_json["data"]["branch"], "fix/topic-bug",
        "kind=bug derives the fix/ prefix forge-cli's --kind bug expects"
    );

    let path = add_json["data"]["path"].as_str().expect("path");
    let porcelain = git(repo.path(), &["worktree", "list", "--porcelain"]);
    assert!(porcelain.contains("branch refs/heads/fix/topic-bug"));
    assert!(porcelain.contains(path));
}

#[test]
fn worktree_add_kind_test_uses_test_branch_prefix() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree",
            "add",
            "topic-test",
            "--from",
            "main",
            "--kind",
            "test",
            "--format",
            "json",
        ],
    );

    assert_eq!(add.code, 0, "stderr: {}", add.stderr_text());
    assert_eq!(add.stderr_text(), "");

    let add_json = parse_json(&add);
    assert_eq!(add_json["ok"], true);
    assert_eq!(add_json["data"]["slug"], "topic-test");
    assert_eq!(add_json["data"]["kind"], "test");
    assert_eq!(
        add_json["data"]["branch"], "test/topic-test",
        "kind=test derives the test/ prefix forge-cli's --kind test expects"
    );

    let path = add_json["data"]["path"].as_str().expect("path");
    let porcelain = git(repo.path(), &["worktree", "list", "--porcelain"]);
    assert!(porcelain.contains("branch refs/heads/test/topic-test"));
    assert!(porcelain.contains(path));
}

#[test]
fn worktree_add_rejects_unknown_kind() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree", "add", "topic-x", "--kind", "nope", "--from", "main", "--format", "json",
        ],
    );

    assert_ne!(add.code, 0, "unknown --kind must fail");
    let add_json = parse_json(&add);
    assert_eq!(add_json["ok"], false);
    assert_eq!(add_json["error"]["code"], "invalid-kind");
}

#[test]
fn worktree_add_existing_slug_fails_with_json_error() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let first = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "add", "topic-one", "--from", "main"],
    );
    assert_eq!(first.code, 0, "stderr: {}", first.stderr_text());

    let second = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree",
            "add",
            "topic-one",
            "--from",
            "main",
            "--format",
            "json",
        ],
    );

    assert_ne!(second.code, 0);
    assert_eq!(second.stderr_text(), "");
    let json = parse_json(&second);
    assert_eq!(json["schema_version"], "cli.git-cli.worktree.add.v1");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "branch-exists");
    assert!(
        json["error"]["message"]
            .as_str()
            .expect("message")
            .contains("feat/topic-one")
    );
}

#[test]
fn worktree_remove_parse_error_respects_json_format() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let output = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "remove", "--format", "json"],
    );

    assert_eq!(output.code, 64);
    assert_eq!(output.stderr_text(), "");
    let json = parse_json(&output);
    assert_eq!(json["schema_version"], "cli.git-cli.worktree.remove.v1");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "invalid-target-count");
}

#[test]
fn worktree_remove_with_branch_name_hints_slug_in_text_and_json() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    // A docs-kind worktree: branch `docs/topic-docs`, slug `topic-docs`.
    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree",
            "add",
            "topic-docs",
            "--from",
            "main",
            "--kind",
            "docs",
            "--format",
            "json",
        ],
    );
    assert_eq!(add.code, 0, "stderr: {}", add.stderr_text());
    let add_json = parse_json(&add);
    assert_eq!(add_json["data"]["branch"], "docs/topic-docs");

    // Passing the branch name (text mode) fails but points at the slug + path.
    let text = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "remove", "docs/topic-docs"],
    );
    assert_ne!(text.code, 0);
    let stderr = text.stderr_text();
    assert!(stderr.contains("hint:"), "stderr: {stderr}");
    assert!(stderr.contains("branch name"), "stderr: {stderr}");
    assert!(stderr.contains("slug 'topic-docs'"), "stderr: {stderr}");

    // The same mistake in JSON mode carries the hint on the error envelope.
    let json = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "remove", "docs/topic-docs", "--format", "json"],
    );
    assert_ne!(json.code, 0);
    let json = parse_json(&json);
    assert_eq!(json["error"]["code"], "worktree-not-found");
    let hint = json["error"]["hint"].as_str().expect("hint");
    assert!(hint.contains("slug 'topic-docs'"), "hint: {hint}");
}

#[test]
fn worktree_list_from_linked_worktree_resolves_primary_repo_root() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree",
            "add",
            "topic-one",
            "--from",
            "main",
            "--format",
            "json",
        ],
    );
    assert_eq!(add.code, 0, "stderr: {}", add.stderr_text());
    let add_json = parse_json(&add);
    let linked_path = add_json["data"]["path"].as_str().expect("path").to_string();
    let expected_repo_root = repo
        .path()
        .canonicalize()
        .expect("canonical repo")
        .to_string_lossy()
        .to_string();

    // Run `worktree list` from INSIDE the linked worktree. The managed layout
    // (repo_root / repo_key / managed flag) must reflect the PRIMARY worktree,
    // not the linked one we happen to stand in.
    let list = run_with_agent_home(
        &harness,
        Path::new(&linked_path),
        agent_home.path(),
        &["worktree", "list", "--format", "json"],
    );
    assert_eq!(list.code, 0, "stderr: {}", list.stderr_text());
    let list_json = parse_json(&list);
    assert_eq!(
        list_json["data"]["repo_root"].as_str(),
        Some(expected_repo_root.as_str()),
        "repo_root should resolve to the primary worktree even from inside a linked worktree"
    );

    let entries = list_json["data"]["entries"]
        .as_array()
        .expect("entries array");
    let managed = entries
        .iter()
        .find(|entry| entry["path"].as_str() == Some(linked_path.as_str()))
        .expect("managed worktree listed");
    assert_eq!(
        managed["managed"], true,
        "managed worktree must stay classified managed from inside a linked worktree"
    );
}

#[test]
fn worktree_go_resolves_slug_and_emits_path_shell_and_json() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree", "add", "topic-go", "--from", "main", "--format", "json",
        ],
    );
    assert_eq!(add.code, 0, "stderr: {}", add.stderr_text());
    let path = parse_json(&add)["data"]["path"]
        .as_str()
        .expect("path")
        .to_string();

    // Default text mode prints the bare resolved path (composable with `cd`).
    let go = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "go", "topic-go"],
    );
    assert_eq!(go.code, 0, "stderr: {}", go.stderr_text());
    assert_eq!(go.stdout_text().trim(), path);

    // Shell mode prints an evaluable `cd -- <path>` command.
    let go_shell = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "go", "topic-go", "--shell"],
    );
    assert_eq!(go_shell.code, 0, "stderr: {}", go_shell.stderr_text());
    let shell_out = go_shell.stdout_text();
    assert!(
        shell_out.trim_start().starts_with("cd -- "),
        "stdout: {shell_out}"
    );
    assert!(shell_out.contains(&path), "stdout: {shell_out}");

    // JSON mode carries the resolved metadata under a versioned envelope.
    let go_json = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "go", "topic-go", "--format", "json"],
    );
    assert_eq!(go_json.code, 0, "stderr: {}", go_json.stderr_text());
    let json = parse_json(&go_json);
    assert_eq!(json["schema_version"], "cli.git-cli.worktree.go.v1");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["path"].as_str(), Some(path.as_str()));
    assert_eq!(json["data"]["branch"], "feat/topic-go");
    assert_eq!(json["data"]["managed"], true);
}

#[test]
fn worktree_go_resolves_branch_name_from_a_linked_worktree() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let alpha = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree", "add", "alpha", "--from", "main", "--format", "json",
        ],
    );
    assert_eq!(alpha.code, 0, "stderr: {}", alpha.stderr_text());
    let alpha_path = parse_json(&alpha)["data"]["path"]
        .as_str()
        .expect("alpha path")
        .to_string();

    let beta = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &[
            "worktree", "add", "beta", "--from", "main", "--format", "json",
        ],
    );
    assert_eq!(beta.code, 0, "stderr: {}", beta.stderr_text());
    let beta_path = parse_json(&beta)["data"]["path"]
        .as_str()
        .expect("beta path")
        .to_string();

    // From inside alpha, jump to beta by its full branch name.
    let go = run_with_agent_home(
        &harness,
        Path::new(&alpha_path),
        agent_home.path(),
        &["worktree", "go", "feat/beta", "--format", "json"],
    );
    assert_eq!(go.code, 0, "stderr: {}", go.stderr_text());
    let json = parse_json(&go);
    assert_eq!(json["data"]["path"].as_str(), Some(beta_path.as_str()));
}

#[test]
fn worktree_go_unknown_target_errors_in_json() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let go = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "go", "does-not-exist", "--format", "json"],
    );
    assert_ne!(go.code, 0);
    assert_eq!(go.stderr_text(), "");
    let json = parse_json(&go);
    assert_eq!(json["schema_version"], "cli.git-cli.worktree.go.v1");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "worktree-not-found");
}

#[test]
fn worktree_remove_refuses_primary_and_removes_managed_slug() {
    let fixture = RemovalFixture::new();
    fs::create_dir(fixture.repo.path().join("safe")).unwrap();
    let primary = fixture.remove(fixture.repo.path().to_str().unwrap());
    assert_eq!(
        parse_json(&primary)["error"]["code"],
        "refuse-primary-worktree"
    );
    let result = fixture.remove("safe");
    assert_eq!(result.code, 0, "{}", result.stdout_text());
    assert_eq!(parse_json(&result)["data"]["removed_path"], fixture.target);
    assert!(!Path::new(&fixture.target).exists());
}

#[test]
fn worktree_add_does_not_adopt_the_base_ref_as_upstream() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let remote = init_bare_remote();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let remote_path = remote.path().to_string_lossy().to_string();
    git(repo.path(), &["remote", "add", "origin", &remote_path]);
    git(repo.path(), &["push", "-u", "origin", "main"]);
    git(repo.path(), &["remote", "set-head", "origin", "main"]);

    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "add", "topic-upstream", "--format", "json"],
    );
    assert_eq!(add.code, 0, "stderr: {}", add.stderr_text());

    let add_json = parse_json(&add);
    assert_eq!(
        add_json["data"]["base_ref"], "origin/main",
        "the default base ref stays the cached remote default branch"
    );
    assert_eq!(add_json["data"]["branch"], "feat/topic-upstream");

    // Branching from `origin/main` must not make the default branch this
    // branch's upstream. A managed worktree branch is unpublished, so any
    // consumer that reads `@{upstream}` to find the branch head — `forge-cli pr
    // deliver` among them — would resolve the default branch instead and report
    // the head as unpushed.
    assert_eq!(
        git_config_optional(repo.path(), "branch.feat/topic-upstream.merge"),
        None,
        "a new managed branch must not inherit an upstream ref"
    );
    assert_eq!(
        git_config_optional(repo.path(), "branch.feat/topic-upstream.remote"),
        None,
        "a new managed branch must not inherit an upstream remote"
    );

    let worktree_path = add_json["data"]["path"].as_str().expect("path");
    let upstream = git_output(
        Path::new(worktree_path),
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    );
    assert!(
        !upstream.status.success(),
        "an unpublished managed branch has no upstream, got {}",
        String::from_utf8_lossy(&upstream.stdout).trim()
    );
}

#[test]
fn worktree_add_caches_a_missing_remote_head_before_resolving_the_base() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let remote = init_bare_remote();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    let remote_path = remote.path().to_string_lossy().to_string();
    git(repo.path(), &["remote", "add", "origin", &remote_path]);
    git(repo.path(), &["push", "-u", "origin", "main"]);
    // A bare fixture remote keeps whatever `init.defaultBranch` the host uses.
    git(remote.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    let cached = git_output(
        repo.path(),
        &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"],
    );
    assert!(
        !cached.status.success(),
        "fixture starts without origin/HEAD"
    );

    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "add", "topic-sethead", "--format", "json"],
    );
    assert_eq!(add.code, 0, "stderr: {}", add.stderr_text());
    assert_eq!(parse_json(&add)["data"]["base_ref"], "origin/main");

    let cached = git_output(
        repo.path(),
        &["symbolic-ref", "-q", "refs/remotes/origin/HEAD"],
    );
    assert!(
        cached.status.success(),
        "worktree add must cache the remote HEAD it found missing"
    );
}

#[test]
fn worktree_add_survives_an_unreachable_remote_when_head_is_missing() {
    let harness = GitCliHarness::new();
    let repo = init_repo();
    let agent_home = tempfile::TempDir::new().expect("agent home");

    git(
        repo.path(),
        &["remote", "add", "origin", "/nonexistent/remote.git"],
    );

    let add = run_with_agent_home(
        &harness,
        repo.path(),
        agent_home.path(),
        &["worktree", "add", "topic-offline", "--format", "json"],
    );
    assert_eq!(add.code, 0, "stderr: {}", add.stderr_text());
    assert_eq!(parse_json(&add)["data"]["base_ref"], "main");
}
