//! Synthetic publication contracts: no provider requests or real stores.
use super::auth_generation::{annotation, profile, pull};
use crate::support::*;
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use std::path::Path;

const ATTEMPT: &str = "44444444-4444-4444-8444-444444444444";
const RECEIPT: &str = ".claude-cli-writer-receipt.json";

fn entry() -> Value {
    let mut entry = profile(annotation());
    entry["publication_attempt_id"] = json!(ATTEMPT);
    entry
}

fn publish(entry: Value, root: &Path) -> nils_test_support::cmd::CmdOutput {
    pull(
        json!({"current":"alpha", "profiles":[entry]}),
        true,
        "alpha",
        root,
    )
}

fn receipt(root: &Path) -> Value {
    let path = root.join("accounts/alpha").join(RECEIPT);
    serde_json::from_slice(&std::fs::read(path).expect("durable writer receipt")).unwrap()
}

#[test]
fn writer_receipt_success_binds_exact_context_and_attempt_after_both_readbacks() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let output = publish(entry(), tmp.path());
    assert_exit(&output, 0);
    let body: Value = serde_json::from_str(&stdout(&output)).unwrap();
    let evidence = &body["result"]["profiles"][0]["writer_receipt"];
    assert_eq!(evidence["schema_version"], "nils.claude-writer-receipt.v1");
    assert_eq!(evidence["credential_generation"], annotation());
    assert_eq!(evidence["publication_attempt_id"], ATTEMPT);
    assert_eq!(evidence["state"], "completed");
    for store in ["account_credentials", "account_config"] {
        assert_eq!(evidence["stores"][store]["state"], "verified");
    }
    assert_eq!(receipt(tmp.path()), *evidence);
    let text = evidence.to_string();
    for private in [
        "fixture-access",
        "fixture-refresh",
        "fixture-account",
        "fixture-org",
        &path_str(tmp.path()),
    ] {
        assert!(!text.contains(private));
    }
    assert_eq!(
        std::fs::metadata(tmp.path().join("accounts/alpha").join(RECEIPT))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn writer_receipt_interrupted_second_store_fences_previous_generation_snapshot_and_attempt() {
    for change in ["generation", "snapshot", "attempt"] {
        let tmp = tempfile::tempdir().unwrap();
        assert_exit(&publish(entry(), tmp.path()), 0);
        let config = tmp.path().join("accounts/alpha/.claude.json");
        std::fs::remove_file(&config).unwrap();
        std::fs::create_dir(&config).unwrap();
        let mut next = entry();
        match change {
            "generation" => next["credential_generation"]["version"]["generation"] = json!("3"),
            "snapshot" => {
                next["credential_generation"]["snapshot_id"] =
                    json!("55555555-5555-4555-8555-555555555555")
            }
            _ => next["publication_attempt_id"] = json!("66666666-6666-4666-8666-666666666666"),
        }
        let output = publish(next.clone(), tmp.path());
        assert_ne!(output.code, 0);
        let evidence = receipt(tmp.path());
        assert_eq!(evidence["state"], "in_progress");
        assert_eq!(
            evidence["credential_generation"],
            next["credential_generation"]
        );
        assert_eq!(
            evidence["publication_attempt_id"],
            next["publication_attempt_id"]
        );
        assert_eq!(
            evidence["stores"]["account_credentials"]["state"],
            "verified"
        );
        assert_ne!(evidence["stores"]["account_config"]["state"], "verified");
    }
}

#[test]
fn writer_receipt_rejects_invalid_or_unbound_attempt_before_mutation() {
    for attempt in [
        json!("invalid"),
        json!(null),
        json!("44444444-4444-1444-8444-444444444444"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let mut invalid = entry();
        invalid["publication_attempt_id"] = attempt;
        let output = publish(invalid, tmp.path());
        assert_ne!(output.code, 0);
        assert!(!tmp.path().join("accounts").exists());
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut invalid = entry();
    invalid
        .as_object_mut()
        .unwrap()
        .remove("credential_generation");
    assert_ne!(publish(invalid, tmp.path()).code, 0);
    assert!(!tmp.path().join("accounts").exists());
}

#[test]
fn writer_receipt_receipt_free_replacement_and_prune_invalidate_old_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    assert_exit(&publish(entry(), tmp.path()), 0);
    assert_eq!(receipt(tmp.path())["state"], "completed");
    assert_exit(&publish(profile(annotation()), tmp.path()), 0);
    assert!(!tmp.path().join("accounts/alpha").join(RECEIPT).exists());
    assert_exit(&publish(entry(), tmp.path()), 0);
    let mut beta = entry();
    beta["profile"] = json!("beta");
    beta["credential_generation"]["account"] = json!("beta");
    let output = pull(
        json!({"current":"beta","profiles":[beta]}),
        true,
        "beta",
        tmp.path(),
    );
    assert_exit(&output, 0);
    assert!(!tmp.path().join("accounts/alpha").exists());
}

#[test]
fn writer_receipt_rechecks_each_store_after_projection_before_completion() {
    for target in [".credentials.json", ".claude.json"] {
        let tmp = tempfile::tempdir().unwrap();
        // Prepare the fake SSH transport, then exercise a competing writer in a
        // fake Keychain process. No shared Keychain or provider is touched.
        assert_exit(&publish(entry(), tmp.path()), 0);
        let bin = tmp.path().join("bin");
        nils_test_support::write_exe(
            &bin,
            "security",
            r#"#!/bin/sh
if [ "$1" = find-generic-password ]; then exit 44; fi
cat >/dev/null
printf '%s' '{}' > "$REPLACED_STORE"
"#,
        );
        let options = base_options(tmp.path())
            .with_path_prepend(&bin)
            .with_env(
                "GENERATION_FIXTURE",
                &path_str(&tmp.path().join("export.json")),
            )
            .with_env("CLAUDE_AUTH_KEYCHAIN", "on")
            .with_env("CLAUDE_AUTH_SECURITY_BIN", &path_str(&bin.join("security")))
            .with_env(
                "REPLACED_STORE",
                &path_str(&tmp.path().join("accounts/alpha").join(target)),
            );
        let output = run(
            &[
                "auth",
                "remote",
                "pull",
                "--ssh",
                "authority",
                "--all",
                "--into",
                &path_str(&tmp.path().join("accounts")),
                "--access-only",
                "--keychain",
                "required",
                "--format",
                "json",
            ],
            &options,
        );
        assert_ne!(output.code, 0, "readback must reject replaced {target}");
        assert_ne!(receipt(tmp.path())["state"], "completed");
        let store = if target == ".credentials.json" {
            "account_credentials"
        } else {
            "account_config"
        };
        assert_eq!(receipt(tmp.path())["stores"][store]["state"], "failed");
        assert!(!stdout(&output).contains("fixture-access"));
    }
}

fn local_snapshot(root: &Path, payload: Value) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let snapshot = root.join("snapshot.json");
    std::fs::write(&snapshot, payload.to_string()).unwrap();
    std::fs::set_permissions(&snapshot, std::fs::Permissions::from_mode(0o600)).unwrap();
    snapshot
}

fn local_publish(
    root: &Path,
    snapshot: &Path,
    mode: &str,
    options: &nils_test_support::cmd::CmdOptions,
) -> nils_test_support::cmd::CmdOutput {
    run(
        &[
            "auth",
            "publish",
            "--snapshot",
            &path_str(snapshot),
            "--into",
            &path_str(&root.join("accounts")),
            "--keychain",
            mode,
            "--format",
            "json",
        ],
        options,
    )
}

#[test]
fn writer_receipt_local_replay_recovers_a_killed_writer_without_default_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let snapshot = local_snapshot(tmp.path(), entry());
    let bin = tmp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    nils_test_support::write_exe(
        &bin,
        "security",
        r#"#!/bin/sh
if [ "$1" = find-generic-password ]; then exit 44; fi
cat >/dev/null
kill -KILL "$PPID"
"#,
    );
    let options = base_options(tmp.path())
        .with_env("CLAUDE_AUTH_KEYCHAIN", "on")
        .with_env("CLAUDE_AUTH_SECURITY_BIN", &path_str(&bin.join("security")));
    let crashed = local_publish(tmp.path(), &snapshot, "required", &options);
    assert_ne!(crashed.code, 0);
    assert_eq!(receipt(tmp.path())["state"], "in_progress");
    let resumed = local_publish(tmp.path(), &snapshot, "off", &options);
    assert_exit(&resumed, 0);
    assert_eq!(receipt(tmp.path())["state"], "completed");
    assert!(!tmp.path().join("accounts/.current").exists());
    assert!(!tmp.path().join("claude-config/.credentials.json").exists());
    let credentials =
        std::fs::read_to_string(tmp.path().join("accounts/alpha/.credentials.json")).unwrap();
    let credentials: Value = serde_json::from_str(&credentials).unwrap();
    assert_eq!(credentials["claudeAiOauth"]["refreshToken"], "");
    assert!(!stdout(&resumed).contains("fixture-access"));
}

#[test]
fn writer_receipt_local_input_and_receipt_unavailable_refuse_before_store_changes() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let tmp = tempfile::tempdir().unwrap();
    let snapshot = local_snapshot(tmp.path(), entry());
    let options = base_options(tmp.path());
    std::fs::set_permissions(&snapshot, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_ne!(
        local_publish(tmp.path(), &snapshot, "off", &options).code,
        0
    );
    assert!(!tmp.path().join("accounts").exists());
    std::fs::set_permissions(&snapshot, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = tmp.path().join("link.json");
    symlink(&snapshot, &link).unwrap();
    assert_ne!(local_publish(tmp.path(), &link, "off", &options).code, 0);
    assert!(!tmp.path().join("accounts").exists());
    assert_exit(&local_publish(tmp.path(), &snapshot, "off", &options), 0);
    let before = std::fs::read(tmp.path().join("accounts/alpha/.credentials.json")).unwrap();
    let path = tmp.path().join("accounts/alpha").join(RECEIPT);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let mut changed = entry();
    changed["claudeAiOauth"]["accessToken"] = json!("fixture-new");
    local_snapshot(tmp.path(), changed);
    let output = local_publish(tmp.path(), &snapshot, "off", &options);
    assert_ne!(output.code, 0);
    assert!(!stdout(&output).contains(&path_str(tmp.path())));
    assert_eq!(
        std::fs::read(tmp.path().join("accounts/alpha/.credentials.json")).unwrap(),
        before
    );
}

#[test]
fn writer_receipt_active_projection_invalidates_account_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    assert_exit(&publish(entry(), tmp.path()), 0);
    let options = base_options(tmp.path())
        .with_path_prepend(&tmp.path().join("bin"))
        .with_env(
            "GENERATION_FIXTURE",
            &path_str(&tmp.path().join("export.json")),
        )
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
        .with_env(
            "CLAUDE_CONFIG_DIR",
            &path_str(&tmp.path().join("accounts/alpha")),
        );
    // Replace the all-export fixture with a single-profile transport payload.
    std::fs::write(tmp.path().join("export.json"), entry().to_string()).unwrap();
    let output = run(
        &[
            "auth",
            "remote",
            "pull",
            "--ssh",
            "authority",
            "--name",
            "alpha",
            "--access-only",
            "--write-active",
            "--keychain",
            "off",
            "--format",
            "json",
        ],
        &options,
    );
    assert_exit(&output, 0);
    assert!(!tmp.path().join("accounts/alpha").join(RECEIPT).exists());
}

#[test]
fn writer_receipt_refresh_reuses_held_accounts_lock_and_invalidates_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    assert_exit(&publish(entry(), tmp.path()), 0);
    let secrets = tmp.path().join("secrets");
    std::fs::create_dir(&secrets).unwrap();
    let mut authority = entry();
    authority["claudeAiOauth"]["scopes"] = json!(["user:profile"]);
    std::fs::write(secrets.join("alpha.json"), authority.to_string()).unwrap();
    std::fs::write(secrets.join("current"), "alpha\n").unwrap();
    let bin = tmp.path().join("bin");
    nils_test_support::write_exe(
        &bin,
        "claude",
        r#"#!/bin/sh
[ "$*" = 'auth login' ] || exit 90
printf '%s' '{"claudeAiOauth":{"accessToken":"fixture-updated","refreshToken":"fixture-rotated","expiresAt":4102444800000}}' > "$CLAUDE_CONFIG_DIR/.credentials.json"
printf '%s' '{"oauthAccount":{"accountUuid":"fixture-account","organizationUuid":"fixture-org"}}' > "$CLAUDE_CONFIG_DIR/.claude.json"
"#,
    );
    let options = base_options(tmp.path())
        .with_env("CLAUDE_SECRET_DIR", &path_str(&secrets))
        .with_env(
            "CLAUDE_CONFIG_DIR",
            &path_str(&tmp.path().join("accounts/alpha")),
        )
        .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
        .with_env("CLAUDE_CLI_BIN", &path_str(&bin.join("claude")));
    let output = run(
        &[
            "auth",
            "refresh",
            "alpha",
            "--accounts-dir",
            &path_str(&tmp.path().join("accounts")),
            "--format",
            "json",
        ],
        &options,
    );
    assert_exit(&output, 0);
    assert!(!tmp.path().join("accounts/alpha").join(RECEIPT).exists());
    assert!(!stdout(&output).contains("fixture-updated"));
}

#[test]
fn writer_receipt_delegated_logout_invalidates_before_upstream_failure() {
    let tmp = tempfile::tempdir().unwrap();
    assert_exit(&publish(entry(), tmp.path()), 0);
    let bin = tmp.path().join("bin");
    nils_test_support::write_exe(&bin, "claude", "#!/bin/sh\nexit 7\n");
    let options = base_options(tmp.path())
        .with_env(
            "CLAUDE_CONFIG_DIR",
            &path_str(&tmp.path().join("accounts/alpha")),
        )
        .with_env("CLAUDE_CLI_BIN", &path_str(&bin.join("claude")));
    assert_exit(&run(&["auth", "logout"], &options), 7);
    assert!(!tmp.path().join("accounts/alpha").join(RECEIPT).exists());
}
