use nils_common::git;
use nils_test_support::git::{git as raw_git, init_repo_main};
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
use pretty_assertions::assert_eq;
use std::{fs, process::Command};

const POLICY: &str = include_str!("../fixtures/identity/policy.toml");
const SECRET: &str = "FIXTURE_HTTPS_HELPER_CANARY";

fn scenario(actor: &str, rewrite: bool, local_kind: &str) {
    let untrusted_local = !local_kind.is_empty();
    let lock = GlobalStateLock::new();
    let home = tempfile::tempdir().unwrap();
    let repo = init_repo_main();
    let bins = StubBinDir::new();
    let actual_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let actual_git = String::from_utf8(actual_git.stdout).unwrap();
    let actual_git = actual_git.trim();
    fs::create_dir_all(home.path().join("forge-cli")).unwrap();
    fs::write(home.path().join("forge-cli/identity.toml"), POLICY).unwrap();
    raw_git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/sandbox/widget.git",
        ],
    );
    let global = home.path().join("git/config");
    fs::create_dir_all(global.parent().unwrap()).unwrap();
    raw_git(
        repo.path(),
        &[
            "config",
            "--file",
            global.to_str().unwrap(),
            "credential.helper",
            "",
        ],
    );
    raw_git(
        repo.path(),
        &[
            "config",
            "--file",
            global.to_str().unwrap(),
            "--add",
            "credential.helper",
            "!f() { test \"$1\" = get || exit 0; cat >/dev/null; printf 'username=fixture\\npassword=%s\\n' \"$FIXTURE_HTTPS_TOKEN\"; }; f",
        ],
    );
    if untrusted_local {
        let helper_key = if local_kind == "scoped" {
            "credential.https://github.com.helper"
        } else {
            "credential.helper"
        };
        let included = home.path().join("local-include");
        let mut config_args = vec!["config"];
        if local_kind == "include" {
            config_args.extend(["--file", included.to_str().unwrap()]);
        }
        let mut reset = config_args.clone();
        reset.extend([helper_key, ""]);
        raw_git(repo.path(), &reset);
        config_args.extend(["--add", helper_key,
            "!f() { printf 'executed' > \"$FIXTURE_LOCAL_HELPER_MARKER\"; cat >/dev/null; printf 'username=fixture\\npassword=%s\\n' \"$FIXTURE_HTTPS_TOKEN\"; }; f"]);
        raw_git(repo.path(), &config_args);
        if local_kind == "include" {
            raw_git(
                repo.path(),
                &["config", "include.path", included.to_str().unwrap()],
            );
        }
    }
    if rewrite {
        raw_git(
            repo.path(),
            &[
                "config",
                "url.https://other.invalid/.insteadOf",
                "https://github.com/",
            ],
        );
    }
    bins.write_exe(
        "gh",
        r#"#!/bin/sh
if test "$1 $2" = 'api user'; then
  if test "$GH_TOKEN" = "$FIXTURE_HTTPS_TOKEN"; then
    printf '{"login":"%s"}' "$FIXTURE_HTTPS_ACTOR"
  else
    printf '{"login":"account-a"}'
  fi
else
  exit 91
fi
"#,
    );
    bins.write_exe("git", &format!(r#"#!/bin/sh
case "$1" in
fetch|push)
  credential=$(printf 'protocol=https\nhost=github.com\npath=sandbox/widget.git\n\n' | '{actual_git}' credential fill) || exit 92
  case "$credential" in *"password=$FIXTURE_HTTPS_TOKEN"*) ;; *) exit 93;; esac
  i=0
  while test "$i" -lt "${{GIT_CONFIG_COUNT:-0}}"; do
    eval "key=\$GIT_CONFIG_KEY_$i"
    case "$key" in url.*.insteadOf) echo unexpected-rewrite >&2; exit 94;; esac
    i=$((i+1))
  done
  printf '%s\n' "$1" >> "$FIXTURE_HTTPS_TRANSPORT_LOG"
  printf 'transport-ok %s\n' "$FIXTURE_HTTPS_TOKEN"
  ;;
*) exec '{actual_git}' "$@";;
esac
"#));
    let _config = EnvGuard::set(&lock, "XDG_CONFIG_HOME", home.path().to_str().unwrap());
    let _global = EnvGuard::set(&lock, "GIT_CONFIG_GLOBAL", global.to_str().unwrap());
    let marker = home.path().join("local-helper-executed");
    let _marker = EnvGuard::set(
        &lock,
        "FIXTURE_LOCAL_HELPER_MARKER",
        marker.to_str().unwrap(),
    );
    let _state = EnvGuard::set(
        &lock,
        "XDG_STATE_HOME",
        home.path().join("state").to_str().unwrap(),
    );
    let _principal = EnvGuard::set(&lock, "FORGE_IDENTITY_PRINCIPAL", "contributor");
    let _token = EnvGuard::set(&lock, "FIXTURE_HTTPS_TOKEN", SECRET);
    let _actor = EnvGuard::set(&lock, "FIXTURE_HTTPS_ACTOR", actor);
    let log = home.path().join("transport.log");
    let _log = EnvGuard::set(&lock, "FIXTURE_HTTPS_TRANSPORT_LOG", log.to_str().unwrap());
    let _profile = if actor == "account-a" {
        EnvGuard::remove(&lock, "FIXTURE_ACCOUNT_A_CREDENTIAL")
    } else {
        EnvGuard::set(
            &lock,
            "FIXTURE_ACCOUNT_A_CREDENTIAL",
            "FIXTURE_PROFILE_TOKEN",
        )
    };
    let _path = prepend_path(&lock, bins.path());
    for args in [
        vec!["fetch", "origin"],
        vec!["push", "origin", "HEAD:refs/heads/fixture"],
    ] {
        let result = git::run_output_in(repo.path(), &args);
        if rewrite || untrusted_local || actor != "account-a" {
            assert!(
                !marker.exists(),
                "repository-local helper executed before principal binding"
            );
            let err = result.expect_err("refuse before transport");
            assert!(
                err.to_string().contains(if untrusted_local {
                    "identity_credential_helper_untrusted"
                } else if rewrite {
                    "identity_transport_override"
                } else {
                    "identity_actor_mismatch"
                }),
                "{err}"
            );
            assert!(!err.to_string().contains(SECRET));
            if !rewrite {
                assert!(
                    err.to_string().contains("credential helper"),
                    "missing fix hint: {err}"
                );
            }
        } else {
            let output = result.unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                "transport-ok [REDACTED]\n"
            );
        }
    }
    if rewrite || untrusted_local || actor != "account-a" {
        assert!(!log.exists(), "refusal must precede fetch/push");
    } else {
        assert_eq!(fs::read_to_string(log).unwrap(), "fetch\npush\n");
    }
    let audit =
        fs::read_to_string(home.path().join("state/forge-cli/identity-audit.jsonl")).unwrap();
    assert!(!audit.contains(SECRET));
}

#[test]
fn https_helper_matching_actor_fetches_and_pushes_without_rewrite() {
    scenario("account-a", false, "");
}
#[test]
fn https_helper_mismatched_actor_refuses_before_fetch_or_push() {
    scenario("account-b", false, "");
}
#[test]
fn https_helper_instead_of_still_refuses() {
    scenario("account-a", true, "");
}

#[test]
fn https_repo_local_helper_is_never_executed() {
    for local_kind in ["plain", "scoped", "include"] {
        scenario("account-a", false, local_kind);
    }
}
