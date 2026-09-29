//! Per-account config directories: `auth remote pull --all --into` on a
//! replica, and `--accounts-dir` re-projection on the authority.

use crate::support::*;
use nils_test_support::cmd::CmdOptions;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const FUTURE_MS: i64 = 4_102_444_800_000; // 2100-01-01
const SOON_MS: i64 = 1_000; // long expired
const MARKER: &str = ".claude-cli-account";

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    bin: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().to_path_buf();
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).expect("bin");
        let fx = Self {
            _tmp: tmp,
            root,
            bin,
        };
        std::fs::create_dir_all(fx.authority()).expect("authority");
        // `ssh <host> <command...>` logs the call and runs the export against
        // a separate authority store.
        fx.script(
            "ssh",
            &format!(
                "#!/bin/sh\necho \"$*\" >> '{log}'\nshift\nCLAUDE_SECRET_DIR='{authority}' exec \"$@\"\n",
                log = fx.ssh_log().display(),
                authority = fx.authority().display()
            ),
        );
        fx.script(
            "claude-cli",
            &format!("#!/bin/sh\nexec '{}' \"$@\"\n", claude_cli_bin().display()),
        );
        fx
    }

    fn authority(&self) -> PathBuf {
        self.root.join("authority")
    }

    fn ssh_log(&self) -> PathBuf {
        self.root.join("ssh.log")
    }

    fn accounts(&self) -> PathBuf {
        self.root.join("accounts")
    }

    fn secret_dir(&self) -> PathBuf {
        self.root.join("claude-secrets")
    }

    fn options(&self) -> CmdOptions {
        base_options(&self.root)
            .with_path_prepend(&self.bin)
            .with_env_remove("CLAUDE_ACCOUNTS_DIR")
            .with_env("CLAUDE_SECRET_DIR", &path_str(&self.secret_dir()))
            .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
            .with_env("CLAUDE_CLI_BIN", &path_str(&self.bin.join("claude")))
    }

    fn script(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        std::fs::write(&path, body).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    fn authority_profile(&self, name: &str, access: &str, account: &str) {
        write_json(
            &self.authority().join(format!("{name}.json")),
            &json!({
                "claudeAiOauth": oauth(access, &format!("refresh-{name}"), FUTURE_MS),
                "oauthAccount": account_json(account)
            }),
        );
    }

    fn pull_all(&self, extra: &[&str], options: &CmdOptions) -> nils_test_support::cmd::CmdOutput {
        let into = path_str(&self.accounts());
        let mut args = vec![
            "auth",
            "remote",
            "pull",
            "--ssh",
            "authority",
            "--all",
            "--into",
            &into,
            "--access-only",
            "--format",
            "json",
        ];
        args.extend_from_slice(extra);
        run(&args, options)
    }
}

fn oauth(access: &str, refresh: &str, expires: i64) -> Value {
    json!({
        "accessToken": access,
        "refreshToken": refresh,
        "expiresAt": expires,
        "scopes": ["user:inference", "user:profile"],
        "subscriptionType": "team",
        "rateLimitTier": "default_claude_max_5x"
    })
}

fn account_json(account: &str) -> Value {
    json!({
        "accountUuid": format!("{account}-uuid"),
        "organizationUuid": "org-uuid",
        "emailAddress": format!("{account}@example.com")
    })
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, serde_json::to_vec(value).expect("json")).expect("write json");
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("read json")).expect("parse json")
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

/// Every regular file under `dir`, read as text, must be free of `needle`.
fn assert_tree_free_of(dir: &Path, needle: &str) {
    for entry in std::fs::read_dir(dir).expect("read dir").flatten() {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).expect("metadata");
        if meta.is_dir() {
            assert_tree_free_of(&path, needle);
        } else if meta.is_file() {
            let bytes = std::fs::read(&path).expect("read");
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains(needle), "{} holds {needle}", path.display());
        }
    }
}

fn service_for(dir: &Path) -> String {
    let digest = Sha256::digest(path_str(dir).as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("Claude Code-credentials-{}", &hex[..8])
}

#[test]
fn auth_remote_export_all_prints_every_profile_access_only() {
    let fx = Fixture::new();
    fx.authority_profile("alpha", "access-alpha", "alpha");
    fx.authority_profile("max", "access-max", "beta");
    std::fs::write(fx.authority().join("current"), "max\n").expect("current");

    let output = run(
        &["auth", "remote", "export", "--all", "--access-only"],
        &fx.options()
            .with_env("CLAUDE_SECRET_DIR", &path_str(&fx.authority())),
    );

    assert_exit(&output, 0);
    assert!(!stdout(&output).contains("refresh-"));
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["current"], "max");
    let profiles = payload["profiles"].as_array().expect("profiles");
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0]["profile"], "alpha");
    assert_eq!(profiles[0]["claudeAiOauth"]["accessToken"], "access-alpha");
    assert_eq!(profiles[1]["profile"], "max");
    assert_eq!(profiles[1]["oauthAccount"]["accountUuid"], "beta-uuid");
}

#[test]
fn auth_remote_pull_all_projects_every_profile_into_its_own_config_dir() {
    let fx = Fixture::new();
    fx.authority_profile("alpha", "access-alpha", "alpha");
    fx.authority_profile("max", "access-max", "beta");
    std::fs::write(fx.authority().join("current"), "max\n").expect("current");
    // An existing account config keeps everything but its account.
    write_json(
        &fx.accounts().join("max").join(".claude.json"),
        &json!({ "theme": "dark", "oauthAccount": account_json("old") }),
    );

    let output = fx.pull_all(&[], &fx.options());

    assert_exit(&output, 0);
    assert!(!stdout(&output).contains("access-alpha"));
    assert!(!stdout(&output).contains("refresh-"));
    // One SSH round trip exports every profile.
    let calls = std::fs::read_to_string(fx.ssh_log()).expect("ssh log");
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        ["authority claude-cli auth remote export --all --access-only"]
    );
    let result = &output.stdout_json()["result"];
    assert_eq!(result["current"], "max");
    assert_eq!(result["into"], path_str(&fx.accounts()));
    assert_eq!(result["pruned"], json!([]));
    assert_eq!(
        result["profiles"],
        json!([
            {
                "name": "alpha",
                "config_dir": path_str(&fx.accounts().join("alpha")),
                "written": true,
                "keychain": "off",
                "expires_at": FUTURE_MS,
                "has_refresh_token": false
            },
            {
                "name": "max",
                "config_dir": path_str(&fx.accounts().join("max")),
                "written": true,
                "keychain": "off",
                "expires_at": FUTURE_MS,
                "has_refresh_token": false
            }
        ])
    );

    for (name, access, account) in [
        ("alpha", "access-alpha", "alpha"),
        ("max", "access-max", "beta"),
    ] {
        let dir = fx.accounts().join(name);
        let credentials_file = dir.join(".credentials.json");
        assert!(
            std::fs::symlink_metadata(&credentials_file)
                .expect("credentials")
                .is_file()
        );
        assert_eq!(mode(&credentials_file), 0o600);
        let credentials = read_json(&credentials_file);
        assert_eq!(credentials["claudeAiOauth"]["accessToken"], access);
        assert_eq!(credentials["claudeAiOauth"]["refreshToken"], "");
        assert_eq!(
            read_json(&dir.join(".claude.json"))["oauthAccount"],
            account_json(account)
        );
        assert!(dir.join(MARKER).is_file(), "{name} marker");
    }
    let fresh = read_json(&fx.accounts().join("alpha").join(".claude.json"));
    assert_eq!(fresh["hasCompletedOnboarding"], true);
    let kept = read_json(&fx.accounts().join("max").join(".claude.json"));
    assert_eq!(kept["theme"], "dark");
    assert!(kept.get("hasCompletedOnboarding").is_none());
    assert_tree_free_of(&fx.accounts(), "refresh-");
    // The default login is not touched by an accounts pull.
    assert!(
        !fx.root
            .join("claude-config")
            .join(".credentials.json")
            .exists()
    );
}

#[test]
fn auth_remote_pull_all_prunes_only_marked_directories_of_removed_profiles() {
    let fx = Fixture::new();
    fx.authority_profile("alpha", "access-alpha", "alpha");
    let accounts = fx.accounts();
    // A marked directory of a removed profile, sharing a directory by symlink.
    let shared = fx.root.join("shared-projects");
    std::fs::create_dir_all(&shared).expect("shared");
    std::fs::write(shared.join("keep.jsonl"), "{}").expect("shared file");
    let gone = accounts.join("gone");
    std::fs::create_dir_all(&gone).expect("gone");
    std::fs::write(gone.join(MARKER), "gone\n").expect("marker");
    std::os::unix::fs::symlink(&shared, gone.join("projects")).expect("symlink");
    // An unmarked directory and a symlink to a marked directory stay.
    std::fs::create_dir_all(accounts.join("manual")).expect("manual");
    let outside = fx.root.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join(MARKER), "outside\n").expect("marker");
    std::os::unix::fs::symlink(&outside, accounts.join("linked")).expect("symlink");

    let output = fx.pull_all(&[], &fx.options());

    assert_exit(&output, 0);
    assert_eq!(output.stdout_json()["result"]["pruned"], json!(["gone"]));
    assert!(!gone.exists());
    assert!(shared.join("keep.jsonl").is_file());
    assert!(accounts.join("manual").is_dir());
    assert!(accounts.join("linked").exists());
    assert!(outside.join(MARKER).is_file());
    assert!(accounts.join("alpha").join(".credentials.json").is_file());
}

#[test]
fn auth_remote_pull_all_refuses_an_empty_export_without_pruning() {
    let fx = Fixture::new();
    let kept = fx.accounts().join("alpha");
    std::fs::create_dir_all(&kept).expect("alpha");
    std::fs::write(kept.join(MARKER), "alpha\n").expect("marker");

    let output = fx.pull_all(&[], &fx.options());

    assert_exit(&output, 1);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "remote-export-missing-access-token"
    );
    assert!(kept.join(MARKER).is_file());
}

#[test]
fn auth_remote_pull_all_writes_one_keychain_item_per_config_dir() {
    let fx = Fixture::new();
    fx.authority_profile("alpha", "access-alpha", "alpha");
    fx.authority_profile("max", "access-max", "beta");
    let log = fx.root.join("security.log");
    // Fake `security`: no existing item; `-i` reads commands from stdin.
    fx.script(
        "security",
        &format!(
            "#!/bin/sh\necho \"argv: $*\" >> '{log}'\nif [ \"$1\" = -i ]; then cat >> '{log}'; exit 0; fi\nexit 44\n",
            log = log.display()
        ),
    );

    let output = fx.pull_all(
        &["--keychain", "required"],
        &fx.options()
            .with_env("CLAUDE_AUTH_KEYCHAIN", "on")
            .with_env(
                "CLAUDE_AUTH_SECURITY_BIN",
                &path_str(&fx.bin.join("security")),
            )
            .with_env("USER", "tester"),
    );

    assert_exit(&output, 0);
    let result = &output.stdout_json()["result"];
    assert_eq!(result["profiles"][0]["keychain"], "written");
    assert_eq!(result["profiles"][1]["keychain"], "written");
    let log = std::fs::read_to_string(&log).expect("security log");
    for line in log.lines().filter(|line| line.starts_with("argv:")) {
        assert!(!line.contains("access-"), "secret on argv: {line}");
        assert!(!line.contains("-X"), "secret on argv: {line}");
    }
    assert!(!log.contains("refresh-"));
    let commands: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("add-generic-password"))
        .collect();
    assert_eq!(commands.len(), 2);
    for (command, name, access) in [
        (commands[0], "alpha", "access-alpha"),
        (commands[1], "max", "access-max"),
    ] {
        let service = service_for(&fx.accounts().join(name));
        assert!(command.contains("-a \"tester\""), "{command}");
        assert!(
            command.contains(&format!("-s \"{service}\"")),
            "{name}: {command}"
        );
        let hex = command.rsplit(' ').next().expect("hex").trim();
        let decoded: Value = serde_json::from_slice(&decode_hex(hex)).expect("keychain json");
        assert_eq!(decoded["claudeAiOauth"]["accessToken"], access);
        assert_eq!(decoded["claudeAiOauth"]["refreshToken"], "");
    }
    assert_ne!(
        service_for(&fx.accounts().join("alpha")),
        service_for(&fx.accounts().join("max"))
    );
}

#[test]
fn auth_remote_pull_all_rejects_mixed_selectors() {
    let fx = Fixture::new();
    for args in [
        vec![
            "auth",
            "remote",
            "pull",
            "--ssh",
            "authority",
            "--all",
            "--access-only",
        ],
        vec![
            "auth",
            "remote",
            "pull",
            "--ssh",
            "authority",
            "--current",
            "--into",
            "x",
            "--access-only",
            "--write-active",
        ],
        vec![
            "auth",
            "remote",
            "pull",
            "--ssh",
            "authority",
            "--all",
            "--into",
            "x",
            "--access-only",
            "--write-active",
        ],
    ] {
        let output = run(&args, &fx.options());
        assert_exit(&output, 64);
    }
    assert!(!fx.ssh_log().exists());
}

/// A fake `claude auth login` returning a rotated login for the beta account.
fn fake_refresh_login(fx: &Fixture) {
    fx.script(
        "claude",
        &format!(
            "#!/bin/sh\ncat > \"$CLAUDE_CONFIG_DIR/.credentials.json\" <<'JSON'\n{{\"claudeAiOauth\":{{\"accessToken\":\"access-new\",\"refreshToken\":\"refresh-new\",\"expiresAt\":{FUTURE_MS},\"scopes\":[\"user:inference\"]}}}}\nJSON\ncat > \"$CLAUDE_CONFIG_DIR/.claude.json\" <<'JSON'\n{{\"oauthAccount\":{{\"accountUuid\":\"beta-uuid\",\"organizationUuid\":\"org-uuid\"}}}}\nJSON\n"
        ),
    );
}

fn write_profile(fx: &Fixture, name: &str, expires: i64) {
    write_json(
        &fx.secret_dir().join(format!("{name}.json")),
        &json!({
            "claudeAiOauth": oauth("access-old", "refresh-old", expires),
            "oauthAccount": account_json("beta")
        }),
    );
}

#[test]
fn auth_auto_refresh_reprojects_refreshed_profiles_into_the_accounts_dir() {
    let fx = Fixture::new();
    write_profile(&fx, "max", SOON_MS);
    write_profile(&fx, "fresh", FUTURE_MS);
    fake_refresh_login(&fx);
    let accounts = path_str(&fx.accounts());

    let output = run(
        &[
            "auth",
            "auto-refresh",
            "--accounts-dir",
            &accounts,
            "--format",
            "json",
        ],
        &fx.options(),
    );

    assert_exit(&output, 0);
    let result = &output.stdout_json()["result"];
    assert_eq!(result["refreshed"], json!(["max"]));
    assert_eq!(result["projected"], json!(["max"]));
    let dir = fx.accounts().join("max");
    let credentials = read_json(&dir.join(".credentials.json"));
    assert_eq!(credentials["claudeAiOauth"]["accessToken"], "access-new");
    assert_eq!(credentials["claudeAiOauth"]["refreshToken"], "");
    assert_eq!(mode(&dir.join(".credentials.json")), 0o600);
    assert_eq!(
        read_json(&dir.join(".claude.json"))["oauthAccount"]["accountUuid"],
        "beta-uuid"
    );
    assert!(dir.join(MARKER).is_file());
    assert!(!fx.accounts().join("fresh").exists());
    assert_tree_free_of(&fx.accounts(), "refresh-");
    // The authority profile keeps its rotated refresh token.
    assert_eq!(
        read_json(&fx.secret_dir().join("max.json"))["claudeAiOauth"]["refreshToken"],
        "refresh-new"
    );
}

#[test]
fn auth_refresh_reads_the_accounts_dir_from_the_environment() {
    let fx = Fixture::new();
    write_profile(&fx, "max", SOON_MS);
    fake_refresh_login(&fx);

    let output = run(
        &["auth", "refresh", "max", "--format", "json"],
        &fx.options()
            .with_env("CLAUDE_ACCOUNTS_DIR", &path_str(&fx.accounts())),
    );

    assert_exit(&output, 0);
    assert_eq!(output.stdout_json()["result"]["projected"], json!(["max"]));
    let credentials = read_json(&fx.accounts().join("max").join(".credentials.json"));
    assert_eq!(credentials["claudeAiOauth"]["accessToken"], "access-new");
    assert_eq!(credentials["claudeAiOauth"]["refreshToken"], "");
    assert_tree_free_of(&fx.accounts(), "refresh-");
}

fn decode_hex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex"))
        .collect()
}
