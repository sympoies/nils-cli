//! Snapshot signing uses an isolated agent key, never the ambient missing key.
use super::{RemovalFixture, git, parse_json};
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_with};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

const CANARY: &str = "synthetic-identity-credential";

struct SigningFixture {
    // Kill only this fixture's agent before its temporary keyring is dropped.
    keyring: tempfile::TempDir,
    removal: RemovalFixture,
    fingerprint: String,
}

impl Drop for SigningFixture {
    fn drop(&mut self) {
        let _ = Command::new("gpgconf")
            .env("GNUPGHOME", self.keyring.path())
            .args(["--kill", "gpg-agent"])
            .output();
    }
}

impl SigningFixture {
    fn new() -> Self {
        let removal = RemovalFixture::new();
        let keyring = tempfile::tempdir().unwrap();
        fs::set_permissions(keyring.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let generated = Command::new("gpg")
            .env("GNUPGHOME", keyring.path())
            .args([
                "--batch",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                "",
                "--quick-generate-key",
                "Snapshot Agent <agent@example.invalid>",
                "ed25519",
                "sign",
                "0",
            ])
            .output()
            .unwrap();
        assert!(
            generated.status.success(),
            "{}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let keys = Command::new("gpg")
            .env("GNUPGHOME", keyring.path())
            .args(["--batch", "--with-colons", "--list-secret-keys"])
            .output()
            .unwrap();
        assert!(keys.status.success());
        let fingerprint = String::from_utf8(keys.stdout)
            .unwrap()
            .lines()
            .find_map(|line| {
                line.starts_with("fpr:")
                    .then(|| line.split(':').nth(9).unwrap().to_owned())
            })
            .unwrap();
        // The unavailable default belongs only to this synthetic HOME. The
        // selected key is supplied to the Git child by the identity owner.
        fs::write(removal.home.path().join(".gitconfig"),
            "[user]\n signingkey = 0000000000000000000000000000000000000000\n[commit]\n gpgsign = true\n[gpg]\n format = openpgp\n program = gpg\n").unwrap();
        git(removal.repo.path(), &["config", "commit.gpgsign", "true"]);
        git(
            removal.repo.path(),
            &["config", "user.name", "Snapshot Agent"],
        );
        git(
            removal.repo.path(),
            &["config", "user.email", "agent@example.invalid"],
        );
        git(
            removal.repo.path(),
            &[
                "remote",
                "set-url",
                "origin",
                "https://github.com/sandbox/widget.git",
            ],
        );
        let config = removal.home.path().join("config/forge-cli");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("identity.toml"),
            format!(
                r#"version = 1
require_session_binding = true
[credentials.agent]
kind = "env"
name = "SNAPSHOT_TEST_CREDENTIAL"
[profiles.agent]
expected_login = "snapshot-agent"
credential = "agent"
commit_name = "Snapshot Agent"
commit_email = "agent@example.invalid"
signing_fingerprint = "{fingerprint}"
operations = ["commit"]
[principals.agent]
profiles = ["agent"]
[[launch_rules]]
id = "fixture-launch"
initiator = "fixture"
principal = "agent"
[[rules]]
id = "fixture-repository"
principal = "agent"
repo = "github.com/sandbox/widget"
profile = "agent"
"#
            ),
        )
        .unwrap();
        removal.probes.write_exe(
            "gh",
            r#"#!/bin/sh
printf '%s\n' '{"login":"snapshot-agent"}'
"#,
        );
        let binding = json!({"schema_version":"cli.agent-session.broker-identity.v1", "ok":true,
            "data":{"schema_version":"agent-session.forge-binding.v1", "session_id":"fixture-caller",
                "session_incarnation":"fixture-runtime", "session_created_at":"2026-01-01T00:00:00Z",
                "root":{"machine":"fixture", "session_id":"fixture-root", "session_created_at":"2026-01-01T00:00:00Z"},
                "parent":null, "initiator":"fixture", "role":null}});
        removal.probes.write_exe(
            "agent-session",
            &format!(
                r#"#!/bin/sh
if [ "$1" = broker ]; then
 printf '%s\n' '{binding}'
else
 printf '%s\n' '{{"ok":true,"data":[]}}'
fi
"#
            ),
        );
        fs::write(
            Path::new(&removal.target).join("README.md"),
            b"selected signer\0preserved\xff\n",
        )
        .unwrap();
        git(Path::new(&removal.target), &["add", "README.md"]);
        Self {
            keyring,
            removal,
            fingerprint,
        }
    }

    fn options(&self) -> CmdOptions {
        self.removal
            .options(self.removal.repo.path())
            .with_env("HOME", self.removal.home.path().to_str().unwrap())
            .with_env(
                "XDG_CONFIG_HOME",
                self.removal.home.path().join("config").to_str().unwrap(),
            )
            .with_env(
                "XDG_STATE_HOME",
                self.removal.home.path().join("state").to_str().unwrap(),
            )
            .with_env("GNUPGHOME", self.keyring.path().to_str().unwrap())
            .with_env_remove_prefix("GIT_CONFIG")
            .with_env(
                "GIT_CONFIG_GLOBAL",
                self.removal
                    .home
                    .path()
                    .join(".gitconfig")
                    .to_str()
                    .unwrap(),
            )
            .with_env_remove_prefix("GIT_AUTHOR_")
            .with_env_remove_prefix("GIT_COMMITTER_")
            .with_env_remove_prefix("FORGE_IDENTITY_")
            .with_env("AGENT_SESSION_RUNTIME_ID", "fixture-runtime")
            .with_env("SNAPSHOT_TEST_CREDENTIAL", CANARY)
    }

    fn remove(&self) -> CmdOutput {
        run_with(
            &self.removal.harness.git_cli_bin(),
            &["worktree", "remove", "safe", "--format", "json"],
            &self.options(),
        )
    }
}

#[test]
fn selected_identity_signs_snapshot_and_restores_staged_bytes() {
    assert_selected_snapshot(SigningFixture::new());
}

#[test]
fn selected_identity_signs_detached_snapshot_and_restores_staged_bytes() {
    let fixture = SigningFixture::new();
    git(
        Path::new(&fixture.removal.target),
        &["checkout", "--detach"],
    );
    assert_selected_snapshot(fixture);
}

fn assert_selected_snapshot(fixture: SigningFixture) {
    let options = fixture.options();
    // Prove the ambient signer is unusable without changing its configuration.
    let ambient = run_with(
        Path::new("git"),
        &[
            "-C",
            &fixture.removal.target,
            "commit-tree",
            "HEAD^{tree}",
            "-S",
            "-m",
            "ambient probe",
        ],
        &options,
    );
    assert_ne!(ambient.code, 0);
    assert!(ambient.stderr_text().contains("signing failed"));
    let result = fixture.remove();
    assert_eq!(result.code, 0, "{}", result.stdout_text());
    assert!(!Path::new(&fixture.removal.target).exists());
    let receipt = parse_json(&result);
    let backup = receipt["data"]["backup_ref"].as_str().unwrap();
    let verified = run_with(Path::new("git"), &["verify-commit", backup], &options);
    assert_eq!(verified.code, 0, "{}", verified.stderr_text());
    let signer = run_with(
        Path::new("git"),
        &["show", "-s", "--format=%G?|%GF|%an|%ae|%cn|%ce", backup],
        &options,
    );
    assert_eq!(
        signer.stdout_text().trim(),
        format!(
            "G|{}|Snapshot Agent|agent@example.invalid|Snapshot Agent|agent@example.invalid",
            fixture.fingerprint
        )
    );
    let restored = fixture.removal.restore(backup, None);
    assert_eq!(restored.code, 0, "{}", restored.stdout_text());
    assert_eq!(
        fs::read(Path::new(&fixture.removal.target).join("README.md")).unwrap(),
        b"selected signer\0preserved\xff\n"
    );
    let audit = fs::read_to_string(
        fixture
            .removal
            .home
            .path()
            .join("state/forge-cli/identity-audit.jsonl"),
    )
    .unwrap();
    assert!(audit.lines().any(|line| {
        let event: serde_json::Value = serde_json::from_str(line).unwrap();
        event["outcome"] == "execution_succeeded"
            && event["object"] == git(fixture.removal.repo.path(), &["rev-parse", backup]).trim()
    }));
    assert!(!audit.contains(CANARY));
}

#[test]
fn selected_identity_failures_retain_target_index_and_redact_diagnostics() {
    for signing_failure in [false, true] {
        let fixture = SigningFixture::new();
        let expected_outcome = if signing_failure {
            // Preserve the real ephemeral key's public metadata for the owner's
            // availability check, then fail only the actual signing operation.
            let keys = Command::new("gpg")
                .env("GNUPGHOME", fixture.keyring.path())
                .args(["--batch", "--with-colons", "--list-secret-keys"])
                .output()
                .unwrap();
            assert!(keys.status.success());
            fixture.removal.probes.write_exe(
                "gpg",
                &format!(
                    r#"#!/bin/sh
case "$*" in
 *--list-secret-keys*) printf '%s' '{}' ;;
 *) printf '%s\n' 'signing failed: {CANARY}' >&2; exit 1 ;;
esac
"#,
                    String::from_utf8(keys.stdout).unwrap()
                ),
            );
            "execution_failed"
        } else {
            fixture.removal.probes.write_exe(
                "gh",
                r#"#!/bin/sh
printf '%s\n' '{"login":"wrong-actor"}'
"#,
            );
            "identity_actor_mismatch"
        };
        let target = Path::new(&fixture.removal.target);
        let index = std::path::PathBuf::from(
            git(
                target,
                &["rev-parse", "--path-format=absolute", "--git-path", "index"],
            )
            .trim(),
        );
        let original_index = fs::read(&index).unwrap();
        let original_bytes = fs::read(target.join("README.md")).unwrap();
        let result = fixture.remove();
        assert_eq!(result.code, 65, "{}", result.stdout_text());
        let error = parse_json(&result);
        assert_eq!(error["error"]["code"], "removal-backup-failed");
        assert!(
            error["error"]["hint"]
                .as_str()
                .unwrap()
                .contains("forge-cli identity doctor --operation commit")
        );
        assert!(target.exists());
        assert_eq!(fs::read(index).unwrap(), original_index);
        assert_eq!(fs::read(target.join("README.md")).unwrap(), original_bytes);
        assert!(git(target, &["for-each-ref", "refs/worktree-backup/"]).is_empty());
        assert!(!result.stdout_text().contains(CANARY));
        assert!(!result.stderr_text().contains(CANARY));
        let audit = fs::read_to_string(
            fixture
                .removal
                .home
                .path()
                .join("state/forge-cli/identity-audit.jsonl"),
        )
        .unwrap();
        assert!(
            audit.lines().any(|line| {
                let event: serde_json::Value = serde_json::from_str(line).unwrap();
                event["outcome"] == expected_outcome && event["object"].is_null()
            }),
            "missing authorization outcome: {audit}"
        );
        assert!(!audit.contains(CANARY));
    }
}
