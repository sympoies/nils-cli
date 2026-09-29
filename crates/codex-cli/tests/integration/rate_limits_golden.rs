//! Byte-for-byte transcript of `codex-cli diag rate-limits` across its modes.
//!
//! The orchestration behind these modes is shared with other providers, so
//! this transcript pins the exact stdout and exit code of every mode that the
//! shared driver renders: single, one-line, `--all`, `--async`, JSON
//! collections, and `--cached`. Reset epochs sit in the past and `TZ=UTC`, so
//! every rendered time is deterministic.

use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions};
use nils_test_support::http::{HttpResponse, TestServer};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;

const ALPHA_USAGE: &str = r#"{
  "plan_type": "pro",
  "rate_limit": {
    "allowed": true,
    "primary_window": { "limit_window_seconds": 18000, "used_percent": 6, "reset_at": 1700000000 },
    "secondary_window": { "limit_window_seconds": 604800, "used_percent": 12, "reset_at": 1700500000 }
  },
  "rate_limit_reset_credits": { "available_count": 2 }
}"#;

const GAMMA_USAGE: &str = r#"{ "plan_type": "plus", "rate_limit": null }"#;

fn usage_response(request: &nils_test_support::http::RecordedRequest) -> HttpResponse {
    let authorization = request.header_value("authorization").unwrap_or_default();
    if authorization.ends_with("tok-alpha") {
        HttpResponse::new(200, ALPHA_USAGE)
    } else if authorization.ends_with("tok-gamma") {
        HttpResponse::new(200, GAMMA_USAGE)
    } else {
        HttpResponse::new(
            402,
            r#"{"error":{"message":"Your subscription payment is past due."}}"#,
        )
    }
}

fn write_secret(dir: &Path, name: &str, token: &str, account: &str) {
    fs::write(
        dir.join(format!("{name}.json")),
        format!(r#"{{"tokens":{{"access_token":"{token}","account_id":"{account}"}}}}"#),
    )
    .expect("write secret");
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: String,
    options: CmdOptions,
}

impl Fixture {
    fn new(server_url: &str) -> Self {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let root = dir.path();
        let secrets = root.join("secrets");
        fs::create_dir_all(&secrets).expect("secrets");
        write_secret(&secrets, "alpha", "tok-alpha", "acct_001");
        write_secret(&secrets, "beta", "tok-beta", "acct_002");
        write_secret(&secrets, "gamma", "tok-gamma", "acct_003");

        let path = |relative: &str| root.join(relative).to_string_lossy().to_string();
        let options = CmdOptions::default()
            .with_env_remove_prefix("CODEX_")
            .with_env_remove_many(&["ZSH_DEBUG", "NO_COLOR"])
            .with_env("HOME", &path("home"))
            .with_env("CODEX_HOME", &path("codex-home"))
            .with_env("CODEX_AUTH_FILE", &path("missing-auth.json"))
            .with_env("CODEX_SECRET_DIR", &path("secrets"))
            .with_env("CODEX_SECRET_CACHE_DIR", &path("secret-cache"))
            .with_env("ZSH_CACHE_DIR", &path("cache"))
            .with_env("CODEX_CHATGPT_BASE_URL", server_url)
            .with_env("CODEX_AUTO_REFRESH_ENABLED", "false")
            .with_env("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED", "false")
            .with_env("CODEX_RATE_LIMITS_WATCH_MAX_ROUNDS", "1")
            .with_env("TZ", "UTC");
        let root = root.to_string_lossy().to_string();
        Self {
            _dir: dir,
            root,
            options,
        }
    }

    fn transcript_entry(&self, args: &[&str]) -> String {
        let output = cmd::run_with(&bin::resolve("codex-cli"), args, &self.options);
        let stdout = output
            .stdout_text()
            .replace(&self.root, "<root>")
            .lines()
            .map(|line| {
                if line.starts_with("Last update: ") {
                    "Last update: <now>".to_string()
                } else {
                    sorted_raw_usage_window_keys(line)
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!("$ {}\nexit={}\n{stdout}\n", args.join(" "), output.code)
    }
}

/// `raw_usage` is a `serde_json` map, so its key order follows serde_json's
/// `preserve_order` feature, which workspace feature unification turns on or
/// off depending on which packages share the build. Put its window keys back
/// in sorted order so the transcript pins everything else byte for byte.
fn sorted_raw_usage_window_keys(line: &str) -> String {
    const USED: &str = "\"used_percent\":";
    const RESET: &str = ",\"reset_at\":";
    let mut out = String::new();
    let mut rest = line;
    while let Some(start) = rest.find(USED) {
        let after_used = &rest[start + USED.len()..];
        let used_len = after_used
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(after_used.len());
        let after_value = &after_used[used_len..];
        let Some(after_reset) = after_value.strip_prefix(RESET) else {
            out.push_str(&rest[..start + USED.len()]);
            rest = after_used;
            continue;
        };
        let reset_len = after_reset
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(after_reset.len());
        out.push_str(&rest[..start]);
        out.push_str(&format!(
            "\"reset_at\":{},{USED}{}",
            &after_reset[..reset_len],
            &after_used[..used_len]
        ));
        rest = &after_reset[reset_len..];
    }
    out.push_str(rest);
    out
}

#[test]
fn rate_limits_golden_transcript_is_byte_stable() {
    let server = TestServer::new(usage_response).expect("server");
    let fixture = Fixture::new(&server.url());

    let commands: &[&[&str]] = &[
        &["diag", "rate-limits", "--format", "json", "alpha.json"],
        &["diag", "rate-limits", "alpha.json"],
        &["diag", "rate-limits", "--one-line", "alpha.json"],
        &["diag", "rate-limits", "--format", "json", "gamma.json"],
        &["diag", "rate-limits", "--all", "--format", "json"],
        &["diag", "rate-limits", "--async", "--json", "--jobs", "2"],
        &["diag", "rate-limits", "--all"],
        &["diag", "rate-limits", "--async"],
        &["diag", "rate-limits", "--async", "--watch"],
        &["diag", "rate-limits", "--cached", "alpha.json"],
        &["diag", "rate-limits", "--cached", "--all"],
        &["diag", "rate-limits", "--async", "--cached"],
        &["diag", "rate-limits", "--async", "--json", "--cached"],
    ];
    let transcript: String = commands
        .iter()
        .map(|args| fixture.transcript_entry(args))
        .collect();

    assert_eq!(transcript, EXPECTED_TRANSCRIPT);
}

const EXPECTED_TRANSCRIPT: &str = r##"$ diag rate-limits --format json alpha.json
exit=0
{"schema_version":"codex-cli.diag.rate-limits.v1","command":"diag rate-limits","mode":"single","ok":true,"result":{"provider":"codex","name":"alpha","target_file":"alpha.json","status":"ok","ok":true,"source":"network","summary":{"non_weekly_label":"5h","non_weekly_remaining":94,"non_weekly_reset_epoch":1700000000,"weekly_remaining":88,"weekly_reset_epoch":1700500000,"weekly_reset_local":"11-20 17:06 +00:00"},"windows":[{"label":"5h","used_percent":6,"remaining_percent":94,"reset_at_epoch":1700000000},{"label":"Weekly","used_percent":12,"remaining_percent":88,"reset_at_epoch":1700500000}],"reset_credits":{"available_count":2},"raw_usage":{"plan_type":"pro","rate_limit":{"allowed":true,"primary_window":{"limit_window_seconds":18000,"reset_at":1700000000,"used_percent":6},"secondary_window":{"limit_window_seconds":604800,"reset_at":1700500000,"used_percent":12}}}}}
$ diag rate-limits alpha.json
exit=0
Rate limits remaining
5h 94% • 11-14 22:13
Weekly 88% • 11-20 17:06
Earned resets available: 2
$ diag rate-limits --one-line alpha.json
exit=0
5h:94% W:88% 11-20 17:06
$ diag rate-limits --format json gamma.json
exit=0
{"schema_version":"codex-cli.diag.rate-limits.v1","command":"diag rate-limits","mode":"single","ok":true,"result":{"provider":"codex","name":"gamma","target_file":"gamma.json","status":"ok","ok":true,"source":"network","windows":[]}}
$ diag rate-limits --all --format json
exit=1
{"schema_version":"codex-cli.diag.rate-limits.v1","command":"diag rate-limits","mode":"all","ok":false,"results":[{"provider":"codex","name":"alpha","target_file":"alpha.json","status":"ok","ok":true,"source":"network","summary":{"non_weekly_label":"5h","non_weekly_remaining":94,"non_weekly_reset_epoch":1700000000,"weekly_remaining":88,"weekly_reset_epoch":1700500000,"weekly_reset_local":"11-20 17:06 +00:00"},"windows":[{"label":"5h","used_percent":6,"remaining_percent":94,"reset_at_epoch":1700000000},{"label":"Weekly","used_percent":12,"remaining_percent":88,"reset_at_epoch":1700500000}],"reset_credits":{"available_count":2},"raw_usage":{"plan_type":"pro","rate_limit":{"allowed":true,"primary_window":{"limit_window_seconds":18000,"reset_at":1700000000,"used_percent":6},"secondary_window":{"limit_window_seconds":604800,"reset_at":1700500000,"used_percent":12}}}},{"provider":"codex","name":"beta","target_file":"beta.json","status":"error","ok":false,"source":"network","reason_code":"billing_past_due","error":{"code":"request-failed","message":"codex-rate-limits: usage request failed (billing_past_due)"}},{"provider":"codex","name":"gamma","target_file":"gamma.json","status":"ok","ok":true,"source":"network","windows":[]}]}
$ diag rate-limits --async --json --jobs 2
exit=1
{"schema_version":"codex-cli.diag.rate-limits.v1","command":"diag rate-limits","mode":"async","ok":false,"results":[{"provider":"codex","name":"alpha","target_file":"alpha.json","status":"ok","ok":true,"source":"network","summary":{"non_weekly_label":"5h","non_weekly_remaining":94,"non_weekly_reset_epoch":1700000000,"weekly_remaining":88,"weekly_reset_epoch":1700500000,"weekly_reset_local":"11-20 17:06 +00:00"},"windows":[{"label":"5h","used_percent":6,"remaining_percent":94,"reset_at_epoch":1700000000},{"label":"Weekly","used_percent":12,"remaining_percent":88,"reset_at_epoch":1700500000}],"reset_credits":{"available_count":2},"raw_usage":{"plan_type":"pro","rate_limit":{"allowed":true,"primary_window":{"limit_window_seconds":18000,"reset_at":1700000000,"used_percent":6},"secondary_window":{"limit_window_seconds":604800,"reset_at":1700500000,"used_percent":12}}}},{"provider":"codex","name":"beta","target_file":"beta.json","status":"error","ok":false,"source":"network","reason_code":"billing_past_due","error":{"code":"request-failed","message":"codex-rate-limits: usage request failed (billing_past_due)"}},{"provider":"codex","name":"gamma","target_file":"gamma.json","status":"ok","ok":true,"source":"network","windows":[]}]}
$ diag rate-limits --all
exit=1

🚦 Codex rate limits for all accounts

Name                   5h     Left    Weekly     Left  Reset                 Resets
-----------------------------------------------------------------------------------
alpha                 94%   0h  0m       88%   0h  0m  11-20 17:06 +00:00         2
beta                    -        -         -        -  -                          -
gamma                 n/a        -       n/a        -  n/a                        -
$ diag rate-limits --async
exit=1

🚦 Codex rate limits for all accounts

Name                   5h     Left    Weekly     Left  Reset                 Resets
-----------------------------------------------------------------------------------
alpha                 94%   0h  0m       88%   0h  0m  11-20 17:06 +00:00         2
beta                    -        -         -        -  -                          -
gamma                 n/a        -       n/a        -  n/a                        -
$ diag rate-limits --async --watch
exit=1

🚦 Codex rate limits for all accounts

Name                   5h     Left    Weekly     Left  Reset                 Resets
-----------------------------------------------------------------------------------
alpha                 94%   0h  0m       88%   0h  0m  11-20 17:06 +00:00         2
beta                    -        -         -        -  -                          -
gamma                 n/a        -       n/a        -  n/a                        -

Last update: <now>
$ diag rate-limits --cached alpha.json
exit=0
5h:94% W:88% 11-20 17:06
$ diag rate-limits --cached --all
exit=0

🚦 Codex rate limits for all accounts

Name                   5h     Left    Weekly     Left  Reset                 Resets
-----------------------------------------------------------------------------------
alpha                 94%   0h  0m       88%   0h  0m  11-20 17:06 +00:00         -
beta                    -        -         -        -  -                          -
gamma                   -        -         -        -  -                          -
$ diag rate-limits --async --cached
exit=0

🚦 Codex rate limits for all accounts

Name                   5h     Left    Weekly     Left  Reset                 Resets
-----------------------------------------------------------------------------------
alpha                 94%   0h  0m       88%   0h  0m  11-20 17:06 +00:00         -
beta                    -        -         -        -  -                          -
gamma                   -        -         -        -  -                          -
$ diag rate-limits --async --json --cached
exit=0
{"schema_version":"codex-cli.diag.rate-limits.v1","command":"diag rate-limits","mode":"async","ok":true,"results":[{"provider":"codex","name":"alpha","target_file":"alpha.json","status":"ok","ok":true,"source":"cache","summary":{"non_weekly_label":"5h","non_weekly_remaining":94,"non_weekly_reset_epoch":1700000000,"weekly_remaining":88,"weekly_reset_epoch":1700500000,"weekly_reset_local":"11-20 17:06 +00:00"},"windows":[{"label":"5h","used_percent":6,"remaining_percent":94,"reset_at_epoch":1700000000},{"label":"Weekly","used_percent":12,"remaining_percent":88,"reset_at_epoch":1700500000}]},{"provider":"codex","name":"beta","target_file":"beta.json","status":"error","ok":false,"source":"cache","error":{"code":"cache-read-failed","message":"codex-rate-limits: cache not found (run codex-rate-limits without --cached, or codex-cli prompt-segment, to populate): <root>/cache/codex/prompt-segment-rate-limits/beta.kv"}},{"provider":"codex","name":"gamma","target_file":"gamma.json","status":"error","ok":false,"source":"cache","error":{"code":"cache-read-failed","message":"codex-rate-limits: cache not found (run codex-rate-limits without --cached, or codex-cli prompt-segment, to populate): <root>/cache/codex/prompt-segment-rate-limits/gamma.kv"}}]}
"##;
