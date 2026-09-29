use crate::support::*;
use nils_test_support::cmd::CmdOptions;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const FUTURE_MS: i64 = 4_102_444_800_000; // 2100-01-01
const SOON_MS: i64 = 1_000; // long expired

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
        Self {
            _tmp: tmp,
            root,
            bin,
        }
    }

    fn config_dir(&self) -> PathBuf {
        self.root.join("claude-config")
    }

    fn secret_dir(&self) -> PathBuf {
        self.root.join("claude-secrets")
    }

    fn options(&self) -> CmdOptions {
        base_options(&self.root)
            .with_path_prepend(&self.bin)
            .with_env("CLAUDE_SECRET_DIR", &path_str(&self.secret_dir()))
            .with_env("CLAUDE_AUTH_KEYCHAIN", "off")
            .with_env("CLAUDE_CLI_BIN", &path_str(&self.bin.join("claude")))
    }

    fn script(&self, name: &str, body: &str) {
        let path = self.bin.join(name);
        std::fs::write(&path, body).expect("script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    fn write_active_login(&self, refresh: &str, account: &str) {
        std::fs::create_dir_all(self.config_dir()).expect("config dir");
        write_json(
            &self.config_dir().join(".credentials.json"),
            &json!({
                "claudeAiOauth": oauth("access-1", refresh, FUTURE_MS),
                "mcpOAuth": { "server": { "accessToken": "mcp-token" } }
            }),
        );
        write_json(
            &self.config_dir().join(".claude.json"),
            &json!({ "theme": "dark", "oauthAccount": account_json(account) }),
        );
    }

    fn profile(&self, name: &str) -> Value {
        read_json(&self.secret_dir().join(format!("{name}.json")))
    }

    fn write_profile(&self, name: &str, access: &str, refresh: &str, expires: i64, account: &str) {
        std::fs::create_dir_all(self.secret_dir()).expect("secret dir");
        write_json(
            &self.secret_dir().join(format!("{name}.json")),
            &json!({
                "claudeAiOauth": oauth(access, refresh, expires),
                "oauthAccount": account_json(account)
            }),
        );
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

#[test]
fn auth_save_stores_a_refresh_capable_login_as_a_named_profile() {
    let fx = Fixture::new();
    fx.write_active_login("refresh-1", "alpha");

    let output = run(&["auth", "save", "team", "--format", "json"], &fx.options());

    assert_exit(&output, 0);
    assert!(!stdout(&output).contains("refresh-1"));
    assert!(!stdout(&output).contains("access-1"));
    let profile = fx.profile("team");
    assert_eq!(profile["claudeAiOauth"]["refreshToken"], "refresh-1");
    assert_eq!(profile["oauthAccount"]["accountUuid"], "alpha-uuid");
    assert!(profile.get("mcpOAuth").is_none());
    assert_eq!(mode(&fx.secret_dir().join("team.json")), 0o600);
    // The saved profile is now the only refresher of this login.
    let active = read_json(&fx.config_dir().join(".credentials.json"));
    assert_eq!(active["claudeAiOauth"]["refreshToken"], "");
    assert_eq!(active["claudeAiOauth"]["accessToken"], "access-1");
    assert_eq!(active["mcpOAuth"]["server"]["accessToken"], "mcp-token");
}

#[test]
fn auth_save_refuses_access_only_logins_and_identity_changes() {
    let fx = Fixture::new();
    fx.write_active_login("", "alpha");
    let output = run(&["auth", "save", "team", "--format", "json"], &fx.options());
    assert_exit(&output, 65);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "active-login-access-only"
    );

    fx.write_profile("team", "access-0", "refresh-0", FUTURE_MS, "beta");
    fx.write_active_login("refresh-1", "alpha");
    let output = run(&["auth", "save", "team", "--format", "json"], &fx.options());
    assert_exit(&output, 65);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "profile-identity-mismatch"
    );
    assert_eq!(
        fx.profile("team")["claudeAiOauth"]["refreshToken"],
        "refresh-0"
    );
}

#[test]
fn auth_use_projects_an_access_only_login_and_records_the_current_profile() {
    let fx = Fixture::new();
    fx.write_active_login("refresh-local", "alpha");
    fx.write_profile("max", "access-max", "refresh-max", FUTURE_MS, "beta");

    let output = run(&["auth", "use", "max", "--format", "json"], &fx.options());

    assert_exit(&output, 0);
    let credentials = read_json(&fx.config_dir().join(".credentials.json"));
    assert_eq!(credentials["claudeAiOauth"]["accessToken"], "access-max");
    assert_eq!(credentials["claudeAiOauth"]["refreshToken"], "");
    assert_eq!(credentials["claudeAiOauth"]["expiresAt"], FUTURE_MS);
    assert_eq!(
        credentials["mcpOAuth"]["server"]["accessToken"],
        "mcp-token"
    );
    let config = read_json(&fx.config_dir().join(".claude.json"));
    assert_eq!(config["oauthAccount"]["accountUuid"], "beta-uuid");
    assert_eq!(config["theme"], "dark");
    assert_eq!(
        std::fs::read_to_string(fx.secret_dir().join("current")).expect("current"),
        "max\n"
    );

    let current = run(&["auth", "current", "--format", "json"], &fx.options());
    assert_exit(&current, 0);
    assert_eq!(current.stdout_json()["result"]["profile"], "max");
    assert_eq!(current.stdout_json()["result"]["account_uuid"], "beta-uuid");
}

#[test]
fn auth_remote_export_prints_only_access_fields_for_the_current_profile() {
    let fx = Fixture::new();
    fx.write_profile("max", "access-max", "refresh-max", FUTURE_MS, "beta");
    std::fs::write(fx.secret_dir().join("current"), "max\n").expect("current");

    let output = run(
        &["auth", "remote", "export", "--current", "--access-only"],
        &fx.options(),
    );

    assert_exit(&output, 0);
    assert!(!stdout(&output).contains("refresh-max"));
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["profile"], "max");
    assert_eq!(payload["claudeAiOauth"]["accessToken"], "access-max");
    assert!(payload["claudeAiOauth"].get("refreshToken").is_none());
    assert_eq!(payload["oauthAccount"]["accountUuid"], "beta-uuid");
}

#[test]
fn auth_remote_pull_writes_the_current_default_as_an_access_only_login() {
    let fx = Fixture::new();
    fx.write_active_login("refresh-local", "alpha");
    // `ssh <host> <command...>` runs the export against a separate authority store.
    let authority = fx.root.join("authority");
    std::fs::create_dir_all(&authority).expect("authority");
    fx.script(
        "ssh",
        &format!(
            "#!/bin/sh\nshift\nCLAUDE_SECRET_DIR='{}' exec \"$@\"\n",
            authority.display()
        ),
    );
    fx.script(
        "claude-cli",
        &format!("#!/bin/sh\nexec '{}' \"$@\"\n", claude_cli_bin().display()),
    );
    write_json(
        &authority.join("max.json"),
        &json!({
            "claudeAiOauth": oauth("access-max", "refresh-max", FUTURE_MS),
            "oauthAccount": account_json("beta")
        }),
    );
    std::fs::write(authority.join("current"), "max\n").expect("current");

    let output = run(
        &[
            "auth",
            "remote",
            "pull",
            "--ssh",
            "authority",
            "--current",
            "--access-only",
            "--write-active",
            "--format",
            "json",
        ],
        &fx.options(),
    );

    assert_exit(&output, 0);
    assert!(!stdout(&output).contains("access-max"));
    let result = &output.stdout_json()["result"];
    assert_eq!(result["profile"], "max");
    assert_eq!(result["has_refresh_token"], false);
    assert_eq!(result["keychain"], "off");
    let credentials = read_json(&fx.config_dir().join(".credentials.json"));
    assert_eq!(credentials["claudeAiOauth"]["accessToken"], "access-max");
    assert_eq!(credentials["claudeAiOauth"]["refreshToken"], "");
    assert_eq!(
        credentials["mcpOAuth"]["server"]["accessToken"],
        "mcp-token"
    );
    assert_eq!(mode(&fx.config_dir().join(".credentials.json")), 0o600);
    let config = read_json(&fx.config_dir().join(".claude.json"));
    assert_eq!(config["oauthAccount"]["accountUuid"], "beta-uuid");
    assert_eq!(config["theme"], "dark");
}

#[test]
fn auth_remote_pull_writes_the_keychain_item_through_stdin() {
    let fx = Fixture::new();
    fx.write_active_login("refresh-local", "alpha");
    let log = fx.root.join("security.log");
    // Fake `security`: no existing item; `-i` reads commands from stdin.
    fx.script(
        "security",
        &format!(
            "#!/bin/sh\necho \"argv: $*\" >> '{log}'\nif [ \"$1\" = -i ]; then cat >> '{log}'; exit 0; fi\nexit 44\n",
            log = log.display()
        ),
    );
    fx.script(
        "ssh",
        &format!(
            "#!/bin/sh\nprintf '%s' '{}'\n",
            json!({
                "profile": "max",
                "claudeAiOauth": oauth("access-max", "refresh-leak", FUTURE_MS),
                "oauthAccount": account_json("beta")
            })
        ),
    );

    let output = run(
        &[
            "auth",
            "remote",
            "pull",
            "--ssh",
            "authority",
            "--current",
            "--access-only",
            "--write-active",
            "--keychain",
            "required",
            "--format",
            "json",
        ],
        &fx.options()
            .with_env("CLAUDE_AUTH_KEYCHAIN", "on")
            .with_env(
                "CLAUDE_AUTH_SECURITY_BIN",
                &path_str(&fx.bin.join("security")),
            )
            .with_env("USER", "tester"),
    );

    assert_exit(&output, 0);
    assert_eq!(output.stdout_json()["result"]["keychain"], "written");
    let log = std::fs::read_to_string(&log).expect("security log");
    for line in log.lines().filter(|line| line.starts_with("argv:")) {
        assert!(!line.contains("access-max"), "secret on argv: {line}");
        assert!(!line.contains("-X"), "secret on argv: {line}");
    }
    let command = log
        .lines()
        .find(|line| line.starts_with("add-generic-password"))
        .expect("add command on stdin");
    assert!(command.contains("-a \"tester\""));
    assert!(command.contains("-s \"Claude Code-credentials-"));
    let hex = command.rsplit(' ').next().expect("hex").trim();
    let decoded: Value = serde_json::from_slice(&decode_hex(hex)).expect("keychain json");
    assert_eq!(decoded["claudeAiOauth"]["accessToken"], "access-max");
    assert_eq!(decoded["claudeAiOauth"]["refreshToken"], "");
    assert!(!log.contains("refresh-leak"));
}

#[test]
fn auth_remote_pull_refuses_a_payload_without_an_access_token() {
    let fx = Fixture::new();
    fx.write_active_login("refresh-local", "alpha");
    fx.script(
        "ssh",
        "#!/bin/sh\nprintf '%s' '{\"profile\":\"max\",\"claudeAiOauth\":{\"refreshToken\":\"r\"}}'\n",
    );

    let output = run(
        &[
            "auth",
            "remote",
            "pull",
            "--ssh",
            "authority",
            "--current",
            "--access-only",
            "--write-active",
            "--format",
            "json",
        ],
        &fx.options(),
    );

    assert_exit(&output, 1);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "remote-export-missing-access-token"
    );
    let credentials = read_json(&fx.config_dir().join(".credentials.json"));
    assert_eq!(
        credentials["claudeAiOauth"]["refreshToken"],
        "refresh-local"
    );
}

#[test]
fn auth_refresh_exchanges_the_refresh_token_through_claude_code() {
    let fx = Fixture::new();
    fx.write_active_login("", "beta");
    fx.write_profile("max", "access-old", "refresh-old", SOON_MS, "beta");
    std::fs::write(fx.secret_dir().join("current"), "max\n").expect("current");
    let env_log = fx.root.join("claude-env.log");
    // Fake `claude auth login` using the documented refresh-token variables.
    fx.script(
        "claude",
        &format!(
            r#"#!/bin/sh
[ "$*" = "auth login" ] || exit 90
[ "$CLAUDE_CODE_OAUTH_REFRESH_TOKEN" = refresh-old ] || exit 91
[ "$CLAUDE_CODE_OAUTH_SCOPES" = "user:inference user:profile" ] || exit 92
echo "config=$CLAUDE_CONFIG_DIR" > '{log}'
cat > "$CLAUDE_CONFIG_DIR/.credentials.json" <<'JSON'
{{"claudeAiOauth":{{"accessToken":"access-new","refreshToken":"refresh-new","expiresAt":{FUTURE_MS},"scopes":["user:inference","user:profile"],"subscriptionType":"team"}}}}
JSON
cat > "$CLAUDE_CONFIG_DIR/.claude.json" <<'JSON'
{{"oauthAccount":{{"accountUuid":"beta-uuid","organizationUuid":"org-uuid","displayName":"Beta"}}}}
JSON
echo "Login successful."
"#,
            log = env_log.display()
        ),
    );

    let output = run(&["auth", "auto-refresh", "--format", "json"], &fx.options());

    assert_exit(&output, 0);
    assert!(!stdout(&output).contains("access-new"));
    assert!(!stdout(&output).contains("refresh-new"));
    let result = &output.stdout_json()["result"];
    assert_eq!(result["refreshed"], json!(["max"]));
    let profile = fx.profile("max");
    assert_eq!(profile["claudeAiOauth"]["accessToken"], "access-new");
    assert_eq!(profile["claudeAiOauth"]["refreshToken"], "refresh-new");
    assert_eq!(profile["oauthAccount"]["displayName"], "Beta");
    assert_eq!(profile["oauthAccount"]["emailAddress"], "beta@example.com");
    // The current profile is re-projected into the local access-only login.
    let credentials = read_json(&fx.config_dir().join(".credentials.json"));
    assert_eq!(credentials["claudeAiOauth"]["accessToken"], "access-new");
    assert_eq!(credentials["claudeAiOauth"]["refreshToken"], "");
    // The exchange ran in an isolated config dir, removed afterwards.
    let used = std::fs::read_to_string(&env_log).expect("env log");
    let used_dir = used.trim().strip_prefix("config=").expect("config line");
    assert_ne!(Path::new(used_dir), fx.config_dir());
    assert!(!Path::new(used_dir).exists());
}

#[test]
fn auth_auto_refresh_skips_fresh_profiles_and_refuses_identity_changes() {
    let fx = Fixture::new();
    fx.write_profile("fresh", "access-f", "refresh-f", FUTURE_MS, "alpha");
    fx.write_profile("stale", "access-s", "refresh-s", SOON_MS, "alpha");
    fx.script(
        "claude",
        &format!(
            r#"#!/bin/sh
cat > "$CLAUDE_CONFIG_DIR/.credentials.json" <<'JSON'
{{"claudeAiOauth":{{"accessToken":"access-x","refreshToken":"refresh-x","expiresAt":{FUTURE_MS}}}}}
JSON
cat > "$CLAUDE_CONFIG_DIR/.claude.json" <<'JSON'
{{"oauthAccount":{{"accountUuid":"other-uuid","organizationUuid":"org-uuid"}}}}
JSON
"#
        ),
    );

    let output = run(&["auth", "auto-refresh", "--format", "json"], &fx.options());

    assert_exit(&output, 1);
    let result = &output.stdout_json()["result"];
    assert_eq!(result["skipped"], json!(["fresh"]));
    assert_eq!(result["failed"][0]["profile"], "stale");
    assert_eq!(result["failed"][0]["code"], "refresh-identity-mismatch");
    assert_eq!(
        fx.profile("stale")["claudeAiOauth"]["refreshToken"],
        "refresh-s"
    );
    assert_eq!(
        fx.profile("fresh")["claudeAiOauth"]["accessToken"],
        "access-f"
    );
    // The rotated login the server already issued is kept for manual recovery.
    let quarantine = fx.secret_dir().join("stale.refresh-quarantine");
    assert_eq!(
        read_json(&quarantine)["claudeAiOauth"]["refreshToken"],
        "refresh-x"
    );
    assert_eq!(mode(&quarantine), 0o600);
    let listed = run(&["auth", "current", "--format", "json"], &fx.options());
    assert_eq!(
        listed.stdout_json()["result"]["profiles"],
        json!(["fresh", "stale"])
    );
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
}

/// A fake `claude auth login` that writes `credentials` and the beta account.
fn fake_refresh_login(fx: &Fixture, credentials: &str) {
    fx.script(
        "claude",
        &format!(
            "#!/bin/sh\ncat > \"$CLAUDE_CONFIG_DIR/.credentials.json\" <<'JSON'\n{credentials}\nJSON\ncat > \"$CLAUDE_CONFIG_DIR/.claude.json\" <<'JSON'\n{{\"oauthAccount\":{{\"accountUuid\":\"beta-uuid\",\"organizationUuid\":\"org-uuid\"}}}}\nJSON\n"
        ),
    );
}

#[test]
fn auth_refresh_keeps_the_refresh_token_when_the_exchange_returns_none() {
    let fx = Fixture::new();
    fx.write_profile("max", "access-old", "refresh-old", SOON_MS, "beta");
    fake_refresh_login(
        &fx,
        &format!(r#"{{"claudeAiOauth":{{"accessToken":"access-new","expiresAt":{FUTURE_MS}}}}}"#),
    );

    let output = run(
        &["auth", "refresh", "max", "--format", "json"],
        &fx.options(),
    );

    assert_exit(&output, 0);
    let oauth = &fx.profile("max")["claudeAiOauth"];
    assert_eq!(oauth["accessToken"], "access-new");
    assert_eq!(oauth["refreshToken"], "refresh-old");
    assert_eq!(oauth["scopes"], json!(["user:inference", "user:profile"]));
}

#[test]
fn auth_auto_refresh_honors_the_refresh_margin() {
    let fx = Fixture::new();
    let hour = 3_600_000;
    fx.write_profile("due", "access-d", "refresh-d", now_ms() + 2 * hour, "beta");
    fx.write_profile(
        "later",
        "access-l",
        "refresh-l",
        now_ms() + 6 * hour,
        "beta",
    );
    fake_refresh_login(
        &fx,
        &format!(
            r#"{{"claudeAiOauth":{{"accessToken":"access-new","refreshToken":"refresh-new","expiresAt":{FUTURE_MS}}}}}"#
        ),
    );

    let output = run(&["auth", "auto-refresh", "--format", "json"], &fx.options());
    assert_exit(&output, 0);
    let result = &output.stdout_json()["result"];
    assert_eq!(result["refreshed"], json!(["due"]));
    assert_eq!(result["skipped"], json!(["later"]));

    fx.write_profile("due", "access-d", "refresh-d", now_ms() + 2 * hour, "beta");
    let output = run(
        &["auth", "auto-refresh", "--format", "json"],
        &fx.options()
            .with_env("CLAUDE_AUTH_REFRESH_MARGIN_SECONDS", "3600"),
    );
    assert_exit(&output, 0);
    assert_eq!(
        output.stdout_json()["result"]["skipped"],
        json!(["due", "later"])
    );
}

#[test]
fn auth_refresh_failures_leave_the_profile_and_active_login_untouched() {
    for (case, body, code) in [
        ("nonzero", "#!/bin/sh\nexit 3\n".to_string(), "refresh-rejected"),
        (
            "no-access-token",
            "#!/bin/sh\nprintf '%s' '{\"claudeAiOauth\":{\"refreshToken\":\"r\"}}' > \"$CLAUDE_CONFIG_DIR/.credentials.json\"\n".to_string(),
            "refresh-output-invalid",
        ),
        (
            "not-json",
            "#!/bin/sh\nprintf 'nope' > \"$CLAUDE_CONFIG_DIR/.credentials.json\"\n".to_string(),
            "refresh-output-invalid",
        ),
    ] {
        let fx = Fixture::new();
        fx.write_active_login("", "beta");
        fx.write_profile("max", "access-old", "refresh-old", SOON_MS, "beta");
        std::fs::write(fx.secret_dir().join("current"), "max\n").expect("current");
        fx.script("claude", &body);
        let profile_before = std::fs::read(fx.secret_dir().join("max.json")).expect("profile");
        let active_before = std::fs::read(fx.config_dir().join(".credentials.json")).expect("active");

        let output = run(&["auth", "refresh", "max", "--format", "json"], &fx.options());

        assert_exit(&output, 1);
        assert_eq!(output.stdout_json()["result"]["failed"][0]["code"], code, "{case}");
        assert_eq!(
            std::fs::read(fx.secret_dir().join("max.json")).expect("profile"),
            profile_before,
            "{case}"
        );
        assert_eq!(
            std::fs::read(fx.config_dir().join(".credentials.json")).expect("active"),
            active_before,
            "{case}"
        );
    }
}

#[test]
fn auth_refresh_refuses_keychain_hosts_before_running_claude() {
    let fx = Fixture::new();
    fx.write_profile("max", "access-old", "refresh-old", SOON_MS, "beta");
    let marker = fx.root.join("claude-ran");
    fx.script(
        "claude",
        &format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    );

    let output = run(
        &["auth", "refresh", "max", "--format", "json"],
        &fx.options().with_env("CLAUDE_AUTH_KEYCHAIN", "on"),
    );

    assert_exit(&output, 1);
    assert_eq!(
        output.stdout_json()["result"]["failed"][0]["code"],
        "refresh-unsupported-on-keychain-host"
    );
    assert!(!marker.exists());
}

#[test]
fn auth_use_leaves_the_claude_config_alone_when_the_account_is_unchanged() {
    let fx = Fixture::new();
    fx.write_active_login("", "beta");
    fx.write_profile("max", "access-max", "refresh-max", FUTURE_MS, "beta");
    // The active config already carries exactly the profile's account.
    write_json(
        &fx.config_dir().join(".claude.json"),
        &json!({ "theme": "dark", "oauthAccount": account_json("beta") }),
    );
    let before = std::fs::read(fx.config_dir().join(".claude.json")).expect("config");

    let output = run(&["auth", "use", "max", "--format", "json"], &fx.options());

    assert_exit(&output, 0);
    assert_eq!(output.stdout_json()["result"]["config_updated"], false);
    assert_eq!(
        std::fs::read(fx.config_dir().join(".claude.json")).expect("config"),
        before
    );
}

#[test]
fn auth_remote_pull_reports_the_shared_failure_details() {
    let fx = Fixture::new();
    fx.write_active_login("refresh-local", "alpha");
    fx.script("ssh", "#!/bin/sh\nexit 5\n");
    let args = [
        "auth",
        "remote",
        "pull",
        "--ssh",
        "authority",
        "--current",
        "--access-only",
        "--write-active",
    ];

    let mut json_args = args.to_vec();
    json_args.extend(["--format", "json"]);
    let output = run(&json_args, &fx.options());
    assert_exit(&output, 1);
    let error = &output.stdout_json()["error"];
    assert_eq!(error["code"], "remote-export-failed");
    assert_eq!(
        error["details"],
        json!({ "ssh": "authority", "name": "current", "exit_code": 5 })
    );

    let output = run(&args, &fx.options());
    assert_exit(&output, 1);
    assert_eq!(
        stderr(&output).trim(),
        "claude-remote-pull: remote export failed (exit 5)"
    );
}

#[test]
fn auth_refresh_accepts_an_organization_the_stored_account_did_not_record() {
    let fx = Fixture::new();
    std::fs::create_dir_all(fx.secret_dir()).expect("secret dir");
    write_json(
        &fx.secret_dir().join("max.json"),
        &json!({
            "claudeAiOauth": oauth("access-old", "refresh-old", SOON_MS),
            "oauthAccount": { "accountUuid": "beta-uuid" }
        }),
    );
    fake_refresh_login(
        &fx,
        &format!(
            r#"{{"claudeAiOauth":{{"accessToken":"access-new","refreshToken":"refresh-new","expiresAt":{FUTURE_MS}}}}}"#
        ),
    );

    let output = run(
        &["auth", "refresh", "max", "--format", "json"],
        &fx.options(),
    );

    assert_exit(&output, 0);
    let profile = fx.profile("max");
    assert_eq!(profile["claudeAiOauth"]["refreshToken"], "refresh-new");
    assert_eq!(profile["oauthAccount"]["organizationUuid"], "org-uuid");
}

#[test]
fn auth_use_refuses_a_keychain_item_too_large_for_security_stdin() {
    let fx = Fixture::new();
    fx.write_active_login("", "alpha");
    fx.write_profile("max", "access-max", "refresh-max", FUTURE_MS, "beta");
    let log = fx.root.join("security.log");
    let big = "m".repeat(5000);
    fx.script(
        "security",
        &format!(
            "#!/bin/sh\necho \"argv: $1\" >> '{log}'\nif [ \"$1\" = find-generic-password ]; then printf '%s' '{{\"mcpOAuth\":{{\"server\":{{\"accessToken\":\"{big}\"}}}}}}'; exit 0; fi\nexit 0\n",
            log = log.display()
        ),
    );

    let output = run(
        &["auth", "use", "max", "--format", "json"],
        &fx.options()
            .with_env("CLAUDE_AUTH_KEYCHAIN", "on")
            .with_env(
                "CLAUDE_AUTH_SECURITY_BIN",
                &path_str(&fx.bin.join("security")),
            ),
    );

    // `auth use` projects with auto Keychain handling: the file is written and
    // the oversized Keychain write is reported instead of being split.
    assert_exit(&output, 0);
    assert_eq!(output.stdout_json()["result"]["keychain"], "unavailable");
    let calls = std::fs::read_to_string(&log).expect("security log");
    assert!(
        !calls.contains("argv: -i"),
        "security -i must not run: {calls}"
    );
    let active = read_json(&fx.config_dir().join(".credentials.json"));
    assert_eq!(active["claudeAiOauth"]["accessToken"], "access-max");
}

fn decode_hex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex"))
        .collect()
}

#[test]
fn auth_use_resolves_json_suffix_full_email_and_local_part() {
    for target in ["max.json", "beta@example.com", "BETA@example.com", "beta"] {
        let fx = Fixture::new();
        fx.write_active_login("", "alpha");
        fx.write_profile("max", "access-max", "refresh-max", FUTURE_MS, "beta");
        fx.write_profile("team", "access-team", "refresh-team", FUTURE_MS, "gamma");

        let output = run(&["auth", "use", target, "--format", "json"], &fx.options());

        assert_exit(&output, 0);
        let result = &output.stdout_json()["result"];
        assert_eq!(result["profile"], "max", "{target}");
        assert_eq!(result["target"], target, "{target}");
        assert_eq!(
            std::fs::read_to_string(fx.secret_dir().join("current")).expect("current"),
            "max\n",
            "{target}"
        );
        let credentials = read_json(&fx.config_dir().join(".credentials.json"));
        assert_eq!(credentials["claudeAiOauth"]["accessToken"], "access-max");
    }
}

#[test]
fn auth_use_reports_ambiguous_missing_and_invalid_targets() {
    let fx = Fixture::new();
    fx.write_active_login("", "alpha");
    fx.write_profile("beta-1", "access-1", "refresh-1", FUTURE_MS, "beta");
    fx.write_profile("beta-2", "access-2", "refresh-2", FUTURE_MS, "beta");

    let output = run(&["auth", "use", "beta", "--format", "json"], &fx.options());
    assert_exit(&output, 2);
    let error = &output.stdout_json()["error"];
    assert_eq!(error["code"], "ambiguous-profile");
    assert_eq!(
        error["details"],
        json!({ "target": "beta", "candidates": ["beta-1", "beta-2"] })
    );

    let output = run(&["auth", "use", "beta"], &fx.options());
    assert_exit(&output, 2);
    assert!(
        stderr(&output).contains("beta-1, beta-2"),
        "{}",
        stderr(&output)
    );

    let output = run(
        &["auth", "use", "missing@example.com", "--format", "json"],
        &fx.options(),
    );
    assert_exit(&output, 1);
    assert_eq!(output.stdout_json()["error"]["code"], "profile-not-found");

    for target in ["../escape", "bad name", "a/b"] {
        let output = run(&["auth", "use", target, "--format", "json"], &fx.options());
        assert_exit(&output, 64);
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "invalid-profile-name",
            "{target}"
        );
    }
    assert!(!fx.secret_dir().join("current").exists());
}

#[test]
fn auth_save_requires_confirmation_to_overwrite_the_same_account() {
    let fx = Fixture::new();
    fx.write_profile("team", "access-0", "refresh-0", FUTURE_MS, "alpha");
    fx.write_active_login("refresh-1", "alpha");

    let output = run(&["auth", "save", "team", "--format", "json"], &fx.options());
    assert_exit(&output, 1);
    let error = &output.stdout_json()["error"];
    assert_eq!(error["code"], "overwrite-confirmation-required");
    assert_eq!(error["details"]["profile"], "team");

    let output = run(&["auth", "save", "team"], &fx.options());
    assert_exit(&output, 1);
    assert!(stderr(&output).contains("--yes"), "{}", stderr(&output));

    // Neither refusal touched the profile or the refresh-capable source login.
    assert_eq!(
        fx.profile("team")["claudeAiOauth"]["refreshToken"],
        "refresh-0"
    );
    let active = read_json(&fx.config_dir().join(".credentials.json"));
    assert_eq!(active["claudeAiOauth"]["refreshToken"], "refresh-1");

    let output = run(
        &["auth", "save", "-y", "team.json", "--format", "json"],
        &fx.options(),
    );
    assert_exit(&output, 0);
    assert_eq!(output.stdout_json()["result"]["replaced"], true);
    assert_eq!(
        fx.profile("team")["claudeAiOauth"]["refreshToken"],
        "refresh-1"
    );
    let active = read_json(&fx.config_dir().join(".credentials.json"));
    assert_eq!(active["claudeAiOauth"]["refreshToken"], "");
}

#[test]
fn auth_save_yes_still_refuses_a_different_account() {
    let fx = Fixture::new();
    fx.write_profile("team", "access-0", "refresh-0", FUTURE_MS, "beta");
    fx.write_active_login("refresh-1", "alpha");

    let output = run(
        &["auth", "save", "--yes", "team", "--format", "json"],
        &fx.options(),
    );

    assert_exit(&output, 65);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "profile-identity-mismatch"
    );
    assert_eq!(
        fx.profile("team")["claudeAiOauth"]["refreshToken"],
        "refresh-0"
    );
}

#[test]
fn auth_remove_deletes_only_the_named_profile() {
    let fx = Fixture::new();
    fx.write_profile("max", "access-max", "refresh-max", FUTURE_MS, "beta");
    fx.write_profile("team", "access-team", "refresh-team", FUTURE_MS, "alpha");
    std::fs::write(fx.secret_dir().join("current"), "team\n").expect("current");
    std::fs::write(fx.secret_dir().join("max.refresh-quarantine"), "{}").expect("quarantine");

    let output = run(
        &["auth", "remove", "-y", "max.json", "--format", "json"],
        &fx.options(),
    );

    assert_exit(&output, 0);
    assert_eq!(
        output.stdout_json()["result"],
        json!({ "profile": "max", "removed": true })
    );
    assert!(!fx.secret_dir().join("max.json").exists());
    assert!(fx.secret_dir().join("team.json").exists());
    assert!(fx.secret_dir().join("max.refresh-quarantine").exists());
    assert_eq!(
        std::fs::read_to_string(fx.secret_dir().join("current")).expect("current"),
        "team\n"
    );

    let output = run(&["auth", "remove", "--yes", "team"], &fx.options());
    assert_exit(&output, 1);
    assert!(
        stderr(&output).contains("current default"),
        "{}",
        stderr(&output)
    );
    assert!(fx.secret_dir().join("team.json").exists());
}

#[test]
fn auth_remove_reports_each_refusal_with_its_exit_code() {
    let fx = Fixture::new();
    fx.write_profile("max", "access-max", "refresh-max", FUTURE_MS, "beta");
    std::fs::write(fx.secret_dir().join("current"), "max\n").expect("current");

    let output = run(
        &["auth", "remove", "--yes", "max", "--format", "json"],
        &fx.options(),
    );
    assert_exit(&output, 1);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "profile-is-current-default"
    );

    std::fs::remove_file(fx.secret_dir().join("current")).expect("clear current");
    let output = run(
        &["auth", "remove", "max", "--format", "json"],
        &fx.options(),
    );
    assert_exit(&output, 64);
    assert_eq!(output.stdout_json()["error"]["code"], "usage-error");
    let output = run(&["auth", "remove", "max"], &fx.options());
    assert_exit(&output, 64);
    assert!(stderr(&output).contains("--yes"), "{}", stderr(&output));
    assert!(fx.secret_dir().join("max.json").exists());

    let output = run(
        &["auth", "remove", "--yes", "missing", "--format", "json"],
        &fx.options(),
    );
    assert_exit(&output, 1);
    assert_eq!(output.stdout_json()["error"]["code"], "profile-not-found");

    let output = run(
        &["auth", "remove", "--yes", "../max", "--format", "json"],
        &fx.options(),
    );
    assert_exit(&output, 64);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "invalid-profile-name"
    );
    assert!(fx.secret_dir().join("max.json").exists());
}

#[test]
fn auth_current_exits_two_without_a_current_default() {
    let fx = Fixture::new();
    fx.write_profile("max", "access-max", "refresh-max", FUTURE_MS, "beta");

    let output = run(&["auth", "current", "--format", "json"], &fx.options());
    assert_exit(&output, 2);
    let result = &output.stdout_json()["result"];
    assert_eq!(result["matched"], false);
    assert_eq!(result["profile"], Value::Null);
    assert_eq!(result["profiles"], json!(["max"]));

    let output = run(&["auth", "current"], &fx.options());
    assert_exit(&output, 2);

    std::fs::write(fx.secret_dir().join("current"), "max\n").expect("current");
    let output = run(&["auth", "current", "--format", "json"], &fx.options());
    assert_exit(&output, 0);
    assert_eq!(output.stdout_json()["result"]["matched"], true);
    assert_eq!(output.stdout_json()["result"]["profile"], "max");
}
