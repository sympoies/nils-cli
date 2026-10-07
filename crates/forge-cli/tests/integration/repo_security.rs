//! GitHub repository inventory and security read contracts.
use super::support::{StubEnv, parse_envelope, run_forge_cli};
use pretty_assertions::assert_eq;

#[test]
fn repo_security_org_inventory_filters_before_limit_and_pages() {
    let first: Vec<_> = (0..100).map(|i| serde_json::json!({"full_name":format!("acme/archived-{i}"),"name":format!("archived-{i}"),"html_url":format!("https://github.com/acme/archived-{i}"),"archived":true,"fork":false,"private":false})).collect();
    let script = format!(
        r#"#!/bin/sh
case "$*" in
 'api -X GET orgs/acme/repos -f type=sources -f per_page=100 -f page=1') cat <<'JSON'
{first}
JSON
 ;;
 'api -X GET orgs/acme/repos -f type=sources -f per_page=100 -f page=2') printf '%s\n' '[{{"full_name":"acme/widget","name":"widget","html_url":"https://github.com/acme/widget","archived":false,"fork":false,"private":false,"default_branch":"main","updated_at":"2026-01-02T00:00:00Z"}}]';;
 *) echo "unexpected argv: $*" >&2; exit 97;;
esac
"#,
        first = serde_json::to_string(&first).unwrap()
    );
    let stub = StubEnv::new().gh_stub(&script);
    let out = run_forge_cli(
        &stub,
        &[
            "repo",
            "list",
            "--provider",
            "github",
            "--org",
            "acme",
            "--source",
            "--no-archived",
            "--limit",
            "1",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let v = parse_envelope(&out.stdout);
    assert_eq!(v["schema_version"], "cli.forge-cli.repo.list.v1");
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(v["data"]["items"][0]["full_name"], "acme/widget");
}

#[test]
fn repo_security_alerts_all_kinds_are_typed_and_omit_secret_values() {
    for kind in ["dependabot", "code-scanning", "secret-scanning"] {
        let pagination = if kind == "dependabot" {
            "--include"
        } else {
            "-f page=1"
        };
        let headers = if kind == "dependabot" {
            "printf 'HTTP/2.0 200 OK\\n\\n'\n"
        } else {
            ""
        };
        let script = format!(
            r#"#!/bin/sh
[ "$*" = 'api -X GET repos/acme/widget/{kind}/alerts -f state=open -f per_page=100 {pagination}' ] || {{ echo "unexpected argv: $*" >&2; exit 97; }}
{headers}printf '%s\n' '[{{"number":7,"state":"open","html_url":"https://github.com/acme/widget/security/alerts/7","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-02T00:00:00Z","secret":"SCANNED_FIXTURE_CANARY_DO_NOT_EMIT","secret_type_display_name":"Example credential","security_advisory":{{"summary":"Example vulnerability","severity":"high"}},"rule":{{"description":"Example rule","severity":"warning"}}}}]'
"#
        );
        let stub = StubEnv::new().gh_stub(&script);
        let out = run_forge_cli(
            &stub,
            &[
                "security",
                "alerts",
                "list",
                "--provider",
                "github",
                "--repo",
                "acme/widget",
                "--kind",
                kind,
                "--state",
                "open",
                "--format",
                "json",
            ],
        );
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        assert!(!out.stdout.contains("SCANNED_FIXTURE_CANARY_DO_NOT_EMIT"));
        let v = parse_envelope(&out.stdout);
        assert_eq!(v["schema_version"], "cli.forge-cli.security.alerts.list.v1");
        assert_eq!(v["data"]["kind"], kind);
        assert_eq!(v["data"]["items"][0]["number"], 7);
        assert_eq!(v["data"]["items"][0]["updated_at"], "2026-01-02T00:00:00Z");
    }
}

#[test]
fn repo_security_settings_distinguishes_unavailable_from_disabled() {
    for response in [
        serde_json::json!({"security_and_analysis": {"secret_scanning": {"status": "enabled"}}}),
        serde_json::json!({"security_and_analysis": null}),
        serde_json::json!({}),
    ] {
        let block = response
            .get("security_and_analysis")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let script = format!(
            "#!/bin/sh\n[ \"$*\" = 'api repos/acme/widget' ] || exit 97\nprintf '%s\n' '{response}'\n"
        );
        let stub = StubEnv::new().gh_stub(&script);
        let out = run_forge_cli(
            &stub,
            &[
                "security",
                "settings",
                "view",
                "--provider",
                "github",
                "--repo",
                "acme/widget",
                "--format",
                "json",
            ],
        );
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        assert_eq!(
            parse_envelope(&out.stdout)["data"]["security_and_analysis"],
            block
        );
    }
}

#[test]
fn repo_security_commands_are_read_only_and_dry_run_never_calls_backend() {
    for args in [
        vec!["repo", "list", "--org", "acme"],
        vec!["security", "alerts", "list", "--kind", "dependabot"],
        vec!["security", "settings", "view"],
    ] {
        let stub = StubEnv::new().gh_stub("#!/bin/sh\necho backend-called >&2\nexit 97\n");
        let mut command = vec!["--provider", "github", "--format", "json", "--dry-run"];
        if args[0] == "security" {
            command.extend(["--repo", "acme/widget"]);
        }
        command.extend(args.clone());
        let out = run_forge_cli(&stub, &command);
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        assert!(parse_envelope(&out.stdout)["data"]["plan"].is_array());
        let mut effect = vec!["operation-effect", "--format", "json", "--"];
        effect.extend(command.iter().copied().filter(|a| *a != "--dry-run"));
        let out = run_forge_cli(&stub, &effect);
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        let v = parse_envelope(&out.stdout);
        assert_eq!(v["data"]["effect"], "read_only");
        assert_eq!(v["data"]["provider_effect"], "network_read");
    }
}

#[test]
fn repo_security_alert_pagination_honors_large_limits() {
    let row = serde_json::json!({"number":7,"state":"open","html_url":"https://github.com/acme/widget/security/alerts/7"});
    let first = serde_json::to_string(&vec![row.clone(); 100]).unwrap();
    let second = serde_json::to_string(&vec![row]).unwrap();
    let script = format!(
        r#"#!/bin/sh
case "$*" in
 *' -f page='*) echo 'HTTP 400: Pagination using the page parameter is not supported' >&2; exit 1;;
 'api -X GET repos/acme/widget/dependabot/alerts -f state=open -f per_page=100 --include')
   printf '%s\r\n' 'HTTP/2.0 200 OK' 'lInK: <https://api.github.com/repos/acme/widget/dependabot/alerts?per_page=100&after=cursor%2Btwo%2F%3D>; rel="next"' ''
   printf '%s' '{first}';;
 'api -X GET repos/acme/widget/dependabot/alerts -f state=open -f per_page=100 --include -f after=cursor+two/=')
   printf '%s\n' 'HTTP/2.0 200 OK' ''
   printf '%s' '{second}';;
 *) echo "unexpected argv: $*" >&2; exit 97;;
esac
"#
    );
    let stub = StubEnv::new().gh_stub(&script);
    let out = run_forge_cli(
        &stub,
        &[
            "security",
            "alerts",
            "list",
            "--repo",
            "acme/widget",
            "--provider",
            "github",
            "--kind",
            "dependabot",
            "--limit",
            "101",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let v = parse_envelope(&out.stdout);
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 101);
    assert_eq!(v["data"]["limited"], true);
}

#[test]
fn repo_security_rejects_invalid_targets_and_states_before_backend() {
    for (args, code) in [
        (vec!["repo", "list", "--org", "acme/path"], "org_invalid"),
        (
            vec!["repo", "list", "--org", "acme", "--repo", "acme/widget"],
            "repo_conflict",
        ),
        (
            vec![
                "security",
                "alerts",
                "list",
                "--repo",
                "acme/widget",
                "--kind",
                "secret-scanning",
                "--state",
                "fixed",
            ],
            "alert_state_invalid",
        ),
    ] {
        let stub = StubEnv::new().gh_stub("#!/bin/sh\necho backend-called >&2\nexit 97\n");
        let mut command = vec!["--provider", "github", "--format", "json"];
        command.extend(args);
        let out = run_forge_cli(&stub, &command);
        assert_eq!(out.code, 65, "{} {}", out.stdout, out.stderr);
        assert_eq!(parse_envelope(&out.stdout)["error"]["code"], code);
        assert!(!out.stderr.contains("backend-called"));
    }
}

#[test]
fn repo_security_provider_denials_are_not_empty_success() {
    for args in [
        vec!["repo", "list", "--org", "acme"],
        vec![
            "security",
            "alerts",
            "list",
            "--repo",
            "acme/widget",
            "--kind",
            "dependabot",
        ],
        vec!["security", "settings", "view", "--repo", "acme/widget"],
    ] {
        let stub =
            StubEnv::new().gh_stub("#!/bin/sh\necho 'HTTP 403: permission denied' >&2\nexit 1\n");
        let mut command = vec!["--provider", "github", "--format", "json"];
        command.extend(args);
        let out = run_forge_cli(&stub, &command);
        assert_eq!(out.code, 1, "{} {}", out.stdout, out.stderr);
        assert_eq!(parse_envelope(&out.stdout)["ok"], false);
    }
}

#[test]
fn repo_security_enterprise_authority_binds_all_reads() {
    for args in [
        vec!["repo", "list", "--org", "acme"],
        vec![
            "security",
            "alerts",
            "list",
            "--repo",
            "acme/widget",
            "--kind",
            "code-scanning",
        ],
        vec!["security", "settings", "view", "--repo", "acme/widget"],
    ] {
        let stub = StubEnv::new().gh_stub(
            r#"#!/bin/sh
[ "$GH_HOST" = enterprise.example ] || exit 96
[ "$1 $2 $3" = 'api --hostname enterprise.example' ] || exit 97
case "$*" in
 *repos/acme/widget) printf '{"security_and_analysis":null}';;
 *) printf '[]';;
esac
"#,
        );
        let mut command = vec![
            "--provider",
            "github",
            "--host",
            "enterprise.example",
            "--format",
            "json",
        ];
        command.extend(args);
        let out = run_forge_cli(&stub, &command);
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        assert_eq!(
            parse_envelope(&out.stdout)["data"]["host"],
            "enterprise.example"
        );
    }
}

#[test]
fn repo_security_alert_states_follow_each_provider_kind_contract() {
    for (kind, state, valid) in [
        ("dependabot", "auto_dismissed", true),
        ("dependabot", "resolved", false),
        ("code-scanning", "auto_dismissed", false),
        ("code-scanning", "resolved", false),
        ("secret-scanning", "resolved", true),
        ("secret-scanning", "dismissed", false),
        ("dependabot", "all", true),
        ("code-scanning", "all", true),
        ("secret-scanning", "all", true),
    ] {
        let script = if valid {
            let state_arg = if state == "all" {
                String::new()
            } else {
                format!(" -f state={state}")
            };
            let pagination = if kind == "dependabot" {
                "--include"
            } else {
                "-f page=1"
            };
            let headers = if kind == "dependabot" {
                "printf 'HTTP/2.0 200 OK\\n\\n'\n"
            } else {
                ""
            };
            format!(
                "#!/bin/sh\n[ \"$*\" = 'api -X GET repos/acme/widget/{kind}/alerts{state_arg} -f per_page=100 {pagination}' ] || exit 97\n{headers}printf '[]'\n"
            )
        } else {
            "#!/bin/sh\necho backend-called >&2\nexit 97\n".to_string()
        };
        let stub = StubEnv::new().gh_stub(&script);
        let out = run_forge_cli(
            &stub,
            &[
                "security",
                "alerts",
                "list",
                "--provider",
                "github",
                "--repo",
                "acme/widget",
                "--kind",
                kind,
                "--state",
                state,
                "--format",
                "json",
            ],
        );
        assert_eq!(
            out.code,
            if valid { 0 } else { 65 },
            "{kind}/{state}: {} {}",
            out.stdout,
            out.stderr
        );
        if !valid {
            assert_eq!(
                parse_envelope(&out.stdout)["error"]["code"],
                "alert_state_invalid"
            );
        }
    }
}
