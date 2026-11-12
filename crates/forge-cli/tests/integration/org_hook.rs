//! `org hook` integration tests against the stub `gh` shared with the
//! `repo hook` tests (see `repo_hook.rs`).

use std::fs;

use pretty_assertions::assert_eq;

use super::repo_hook::{
    NEW, OLD, active_row, batch_file, calls, flag, never_leaks, patches, run_batch, stored, stub,
    url_row,
};
use super::support::{
    CmdOutput, StubEnv, parse_envelope, run_forge_cli, run_forge_cli_in_with_env,
};

/// Value of the secret environment variable; it may appear only in the
/// request-body file.
const ENV_SECRET: &str = "ENV-SECRET-CANARY-VALUE";

fn org_cmd(stub: &StubEnv, dry: bool, tail: &[&str]) -> CmdOutput {
    let mut args = vec!["--provider", "github", "--format", "json"];
    if dry {
        args.push("--dry-run");
    }
    args.extend_from_slice(&["org", "hook"]);
    args.extend_from_slice(tail);
    args.extend_from_slice(&["--org", "acme"]);
    run_forge_cli(stub, &args)
}

fn create(stub: &StubEnv, dry: bool, url: &str, env: &[(&str, &str)]) -> CmdOutput {
    let mut args = vec!["--provider", "github", "--format", "json"];
    if dry {
        args.push("--dry-run");
    }
    args.extend_from_slice(&[
        "org",
        "hook",
        "create",
        "--org",
        "acme",
        "--url",
        url,
        "--events",
        "push,pull_request",
        "--content-type",
        "json",
        "--secret-env",
        "ORG_HOOK_SECRET",
    ]);
    run_forge_cli_in_with_env(stub, &args, None, env)
}

fn assert_no_env_secret(out: &CmdOutput, stub: &StubEnv) {
    assert!(
        !out.stdout.contains(ENV_SECRET) && !out.stderr.contains(ENV_SECRET),
        "{}\n{}",
        out.stdout,
        out.stderr
    );
    for call in calls(stub) {
        assert!(!call.contains(ENV_SECRET), "secret on argv: {call}");
    }
}

#[test]
fn org_list_redacts_the_secret() {
    let stub = stub(&[]);
    let out = org_cmd(&stub, false, &["list"]);

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    never_leaks(&out);
    assert_eq!(calls(&stub), vec!["api orgs/acme/hooks?per_page=100"]);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], "cli.forge-cli.org.hook.list.v1");
    assert_eq!(env["data"]["target"], "acme");
    assert_eq!(env["data"]["hooks"][0]["url"], OLD);
    assert_eq!(env["data"]["hooks"][0]["secret"], "[redacted]");
    assert_eq!(env["data"]["hooks"][1]["secret"], serde_json::Value::Null);
}

#[test]
fn org_set_url_and_set_active_send_only_the_changed_field() {
    let stub = stub(&[(7, OLD, true)]);
    let url = org_cmd(
        &stub,
        false,
        &["set-url", "--hook", "7", "--from", OLD, "--to", NEW],
    );
    let active = org_cmd(&stub, false, &["set-active", "--hook", "7", "--inactive"]);

    assert_eq!(url.code, 0, "stdout={}\nstderr={}", url.stdout, url.stderr);
    assert_eq!(
        active.code, 0,
        "stdout={}\nstderr={}",
        active.stdout, active.stderr
    );
    never_leaks(&url);
    never_leaks(&active);
    assert_eq!(
        patches(&stub),
        vec![
            format!("api orgs/acme/hooks/7/config --method PATCH -f url={NEW}"),
            "api orgs/acme/hooks/7 --method PATCH -F active=false".to_string(),
        ]
    );
    assert_eq!(stored(&stub, "url", 7), NEW);
    assert_eq!(stored(&stub, "active", 7), "false");
    assert_eq!(
        parse_envelope(&url.stdout)["schema_version"],
        "cli.forge-cli.org.hook.set-url.v1"
    );
}

#[test]
fn org_set_url_refuses_on_mismatch_and_dry_run_writes_nothing() {
    let stub = stub(&[(7, "https://elsewhere.example.test/hook", true)]);
    let refused = org_cmd(
        &stub,
        false,
        &["set-url", "--hook", "7", "--from", OLD, "--to", NEW],
    );
    assert_eq!(refused.code, 65, "{}", refused.stdout);
    assert_eq!(
        parse_envelope(&refused.stdout)["error"]["code"],
        "hook_value_mismatch"
    );

    let stub = super::repo_hook::stub(&[(7, OLD, true)]);
    let dry = org_cmd(
        &stub,
        true,
        &["set-url", "--hook", "7", "--from", OLD, "--to", NEW],
    );
    assert_eq!(dry.code, 0, "{}", dry.stdout);
    assert_eq!(
        parse_envelope(&dry.stdout)["data"]["status"],
        "would_update"
    );
    assert!(patches(&stub).is_empty());
}

#[test]
fn org_commands_require_an_organization() {
    let stub = stub(&[(7, OLD, true)]);
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "org",
            "hook",
            "list",
        ],
    );

    assert_eq!(out.code, 65, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(parse_envelope(&out.stdout)["error"]["code"], "org_required");
    assert!(calls(&stub).is_empty());
}

#[test]
fn org_batch_reports_each_row_and_refuses_repository_targets() {
    let stub = stub(&[
        (7, OLD, true),
        (8, "https://elsewhere.example.test/hook", true),
    ]);
    let file = batch_file(
        &stub,
        &format!(
            "[{},{},{}]",
            url_row("acme", 7, OLD, NEW),
            url_row("acme", 8, OLD, NEW),
            active_row("acme", 7, true, false),
        ),
    );
    let out = run_batch(&stub, "org", &file, false);

    assert_eq!(out.code, 1, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], "cli.forge-cli.org.hook.batch.v1");
    assert_eq!(env["data"]["rows"][0]["status"], "updated");
    assert_eq!(
        env["data"]["rows"][1]["error"]["kind"],
        "hook_value_mismatch"
    );
    assert_eq!(env["data"]["rows"][2]["status"], "updated");
    assert_eq!(patches(&stub).len(), 2);

    let stub = super::repo_hook::stub(&[(7, OLD, true)]);
    let file = batch_file(
        &stub,
        &format!("[{}]", url_row("acme/widgets", 7, OLD, NEW)),
    );
    let invalid = run_batch(&stub, "org", &file, false);
    assert_eq!(invalid.code, 65, "{}", invalid.stdout);
    assert_eq!(
        parse_envelope(&invalid.stdout)["error"]["code"],
        "batch_invalid"
    );
    assert!(calls(&stub).is_empty());
}

#[test]
fn org_batch_refuses_a_global_org() {
    let stub = stub(&[(7, OLD, true)]);
    let file = batch_file(&stub, &format!("[{}]", url_row("acme", 7, OLD, NEW)));
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "org",
            "hook",
            "batch",
            "--org",
            "acme",
            "--file",
            file.to_str().unwrap(),
        ],
    );

    assert_eq!(out.code, 65, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(parse_envelope(&out.stdout)["error"]["code"], "org_conflict");
    assert!(calls(&stub).is_empty());
}

#[test]
fn create_sends_the_secret_only_in_the_request_body_file() {
    let stub = stub(&[]);
    let out = create(&stub, false, NEW, &[("ORG_HOOK_SECRET", ENV_SECRET)]);

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    never_leaks(&out);
    assert_no_env_secret(&out, &stub);
    let body: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(stub.tempdir.path().join("post-body.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        body,
        serde_json::json!({
            "name": "web",
            "config": {
                "url": NEW,
                "content_type": "json",
                "secret": ENV_SECRET,
                "insecure_ssl": "0"
            },
            "events": ["push", "pull_request"],
            "active": true
        })
    );
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], "cli.forge-cli.org.hook.create.v1");
    assert_eq!(env["data"]["status"], "created");
    assert_eq!(env["data"]["id"], 42);
    assert_eq!(env["data"]["secret"], "[redacted]");
    assert_eq!(
        calls(&stub)[1],
        format!(
            "api orgs/acme/hooks --method POST --input {}",
            calls(&stub)[1].rsplit(' ').next().unwrap()
        ),
        "the body is passed by file, never as fields"
    );
}

#[test]
fn create_dry_run_posts_nothing() {
    let stub = stub(&[]);
    let dry = create(&stub, true, NEW, &[("ORG_HOOK_SECRET", ENV_SECRET)]);

    assert_eq!(dry.code, 0, "stdout={}\nstderr={}", dry.stdout, dry.stderr);
    assert_no_env_secret(&dry, &stub);
    let env = parse_envelope(&dry.stdout);
    assert_eq!(env["data"]["status"], "would_create");
    assert_eq!(env["data"]["dry_run"], true);
    assert!(!stub.tempdir.path().join("post-body.json").exists());
    assert_eq!(calls(&stub), vec!["api orgs/acme/hooks?per_page=100"]);
}

#[test]
fn create_requires_the_secret_environment_variable() {
    let stub = stub(&[]);
    for env in [vec![], vec![("ORG_HOOK_SECRET", "")]] {
        let out = create(&stub, false, NEW, &env);

        assert_eq!(out.code, 65, "stdout={}\nstderr={}", out.stdout, out.stderr);
        assert_eq!(
            parse_envelope(&out.stdout)["error"]["code"],
            "secret_env_missing"
        );
    }
    assert!(calls(&stub).is_empty());
}

#[test]
fn create_refuses_a_url_the_organization_already_delivers_to() {
    let stub = stub(&[]);
    // Hook 7 in the stub's list already delivers to OLD.
    let out = create(&stub, false, OLD, &[("ORG_HOOK_SECRET", ENV_SECRET)]);

    assert_eq!(out.code, 65, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "hook_url_exists"
    );
    assert!(!stub.tempdir.path().join("post-body.json").exists());
}

#[test]
fn create_rejects_a_secret_on_the_command_line() {
    let stub = stub(&[]);
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "org",
            "hook",
            "create",
            "--org",
            "acme",
            "--url",
            NEW,
            "--events",
            "push",
            "--content-type",
            "json",
            "--secret",
            ENV_SECRET,
        ],
    );

    assert_eq!(out.code, 64, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert!(calls(&stub).is_empty());
}

#[test]
fn a_missing_org_hook_scope_is_named_in_a_typed_error() {
    let stub = stub(&[(7, OLD, true)]);
    flag(&stub, "deny");
    for tail in [
        vec!["list"],
        vec!["set-url", "--hook", "7", "--from", OLD, "--to", NEW],
    ] {
        let out = org_cmd(&stub, false, &tail);

        assert_eq!(out.code, 65, "{tail:?}\nstdout={}", out.stdout);
        let env = parse_envelope(&out.stdout);
        assert_eq!(env["error"]["code"], "org_hook_scope_missing");
        assert!(
            env["error"]["message"]
                .as_str()
                .unwrap()
                .contains("admin:org_hook")
        );
    }
}

#[cfg(unix)]
mod identity {
    use super::super::repo_hook::identity::{CREDENTIAL, Fixture};
    use super::{NEW, OLD, active_row, url_row};
    use pretty_assertions::assert_eq;
    use std::fs;

    #[test]
    fn an_org_batch_verifies_the_actor_once_under_the_org_rule() {
        let f = Fixture::new();
        let rows = f.home.path().join("rows.json");
        fs::write(
            &rows,
            format!(
                "[{},{}]",
                url_row("sandbox", 7, OLD, NEW),
                active_row("sandbox", 8, true, false),
            ),
        )
        .unwrap();

        let out = f
            .command("contributor")
            .args(["org", "hook", "batch", "--file"])
            .arg(&rows)
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(0), "{stdout}");
        assert_eq!(f.count("calls"), 6, "two rows of read, write, read-back");
        assert_eq!(f.count("probes"), 1, "one actor probe per process");
        assert!(!stdout.contains(CREDENTIAL));
    }

    #[test]
    fn org_calls_need_an_org_rule_for_the_principal() {
        let f = Fixture::new();
        // The coordinator has a repository rule and a default repository in
        // this organization, but no org rule.
        let out = f
            .command("coordinator")
            .args(["org", "hook", "list", "--org", "sandbox"])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(65), "{stdout}");
        assert!(stdout.contains("identity_repository_unknown"), "{stdout}");
        assert_eq!(f.count("calls"), 0);
    }
}
