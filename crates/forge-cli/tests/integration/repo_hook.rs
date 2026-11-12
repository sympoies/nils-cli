//! `repo hook` integration tests against a stub `gh`. The stub keeps one URL
//! and one active flag per hook id in its state directory, logs every call,
//! and answers the GitHub webhook endpoints for repositories and organizations
//! (the shared helpers here are reused by the `org hook` tests).

use std::fs;
use std::path::PathBuf;

use pretty_assertions::assert_eq;

use super::support::{CmdOutput, StubEnv, parse_envelope, run_forge_cli};

pub(super) const OLD: &str = "https://old.example.test/hook";
pub(super) const NEW: &str = "https://new.example.test/hook";
/// A secret value the stub returns from reads; it must never be output or
/// sent back.
pub(super) const SECRET: &str = "S3CRET-CANARY-VALUE";

const LIST_JSON: &str = r#"[{"id":7,"name":"web","active":true,"events":["push","pull_request"],"config":{"url":"https://old.example.test/hook","content_type":"json","insecure_ssl":"0","secret":"S3CRET-CANARY-VALUE"}},{"id":9,"name":"web","active":false,"events":["*"],"config":{"url":"https://other.example.test/hook","content_type":"form","insecure_ssl":"0"}}]"#;

/// A stub `gh` for hooks `(id, url, active)`. Flag files in its state
/// directory: `deny` makes organization calls fail for a missing
/// `admin:org_hook` scope; `drop-secret` makes any write silently remove the
/// secret, as a wrong PATCH would; `fail-after-patch` makes every hook read
/// after a write fail with a 502.
pub(super) fn stub(hooks: &[(u64, &str, bool)]) -> StubEnv {
    let stub = StubEnv::new();
    let dir = stub.tempdir.path().to_path_buf();
    fs::write(dir.join("list.json"), LIST_JSON).unwrap();
    for (id, url, active) in hooks {
        fs::write(dir.join(format!("url-{id}")), url).unwrap();
        fs::write(dir.join(format!("active-{id}")), active.to_string()).unwrap();
    }
    let body = format!(
        r#"#!/bin/sh
set -eu
dir={dir}
printf '%s\n' "$*" >> "$dir/calls"
[ "$1" = api ] || {{ echo "unexpected gh args: $*" >&2; exit 99; }}
case "$2" in
  orgs/*) if [ -e "$dir/deny" ]; then
    echo 'gh: Not Found (HTTP 404)' >&2
    echo 'This API operation needs the "admin:org_hook" scope. To request it, run:  gh auth refresh -h github.com -s admin:org_hook' >&2
    exit 1
  fi ;;
esac
hook() {{
  secret="{secret}"; [ -e "$dir/secret-gone" ] && secret=""
  printf '{{"id":%s,"active":%s,"events":["push"],"config":{{"url":"%s","content_type":"json","insecure_ssl":"0","secret":"%s"}}}}\n' \
    "$1" "$(cat "$dir/active-$1")" "$(cat "$dir/url-$1")" "$secret"
}}
case "$2" in
  */hooks\?per_page=100) cat "$dir/list.json" ;;
  orgs/acme/hooks)
    [ "$#" -eq 6 ] && [ "$3" = --method ] && [ "$4" = POST ] && [ "$5" = --input ] || {{ echo "unexpected create args: $*" >&2; exit 96; }}
    stat -c %a "$6" > "$dir/post-mode"
    printf '%s' "$6" > "$dir/post-path"
    cp "$6" "$dir/post-body.json"
    printf '{{"id":42,"active":true,"events":["push","pull_request"],"config":{{"url":"https://new.example.test/hook","content_type":"json","insecure_ssl":"0","secret":"********"}}}}\n'
    ;;
  */hooks/*/config)
    id=${{2%/config}}; id=${{id##*/}}
    [ "$#" -eq 6 ] && [ "$3" = --method ] && [ "$4" = PATCH ] && [ "$5" = -f ] && case "$6" in url=*) true;; *) false;; esac || {{ echo "unexpected config args: $*" >&2; exit 98; }}
    printf '%s' "${{6#url=}}" > "$dir/url-$id"
    touch "$dir/patched"
    if [ -e "$dir/drop-secret" ]; then touch "$dir/secret-gone"; fi
    printf '{{"url":"%s"}}\n' "${{6#url=}}"
    ;;
  */hooks/*)
    id=${{2##*/}}
    if [ "$#" -gt 2 ]; then
      [ "$#" -eq 6 ] && [ "$3" = --method ] && [ "$4" = PATCH ] && [ "$5" = -F ] && case "$6" in active=true|active=false) true;; *) false;; esac || {{ echo "unexpected hook args: $*" >&2; exit 97; }}
      printf '%s' "${{6#active=}}" > "$dir/active-$id"
      touch "$dir/patched"
      if [ -e "$dir/drop-secret" ]; then touch "$dir/secret-gone"; fi
      echo '{{}}'
    else
      if [ -e "$dir/fail-after-patch" ] && [ -e "$dir/patched" ]; then echo 'gh: Bad Gateway (HTTP 502)' >&2; exit 1; fi
      hook "$id"
    fi
    ;;
  *) echo "unexpected gh args: $*" >&2; exit 99 ;;
esac
"#,
        dir = dir.display(),
        secret = SECRET,
    );
    stub.gh_stub(&body)
}

pub(super) fn flag(stub: &StubEnv, name: &str) {
    fs::write(stub.tempdir.path().join(name), "").unwrap();
}

pub(super) fn calls(stub: &StubEnv) -> Vec<String> {
    fs::read_to_string(stub.tempdir.path().join("calls"))
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

pub(super) fn patches(stub: &StubEnv) -> Vec<String> {
    calls(stub)
        .into_iter()
        .filter(|call| call.contains("--method PATCH"))
        .collect()
}

pub(super) fn stored(stub: &StubEnv, kind: &str, id: u64) -> String {
    fs::read_to_string(stub.tempdir.path().join(format!("{kind}-{id}"))).unwrap()
}

pub(super) fn never_leaks(out: &CmdOutput) {
    assert!(
        !out.stdout.contains(SECRET) && !out.stderr.contains(SECRET),
        "secret leaked: {}\n{}",
        out.stdout,
        out.stderr
    );
}

fn repo_cmd(stub: &StubEnv, dry: bool, tail: &[&str]) -> CmdOutput {
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
    ];
    if dry {
        args.push("--dry-run");
    }
    args.extend_from_slice(&["repo", "hook"]);
    args.extend_from_slice(tail);
    run_forge_cli(stub, &args)
}

fn set_url(stub: &StubEnv, hook: &str, from: &str, to: &str, dry: bool) -> CmdOutput {
    repo_cmd(
        stub,
        dry,
        &["set-url", "--hook", hook, "--from", from, "--to", to],
    )
}

pub(super) fn batch_file(stub: &StubEnv, rows: &str) -> PathBuf {
    let path = stub.tempdir.path().join("rows.json");
    fs::write(&path, rows).unwrap();
    path
}

pub(super) fn run_batch(
    stub: &StubEnv,
    family: &str,
    file: &std::path::Path,
    dry: bool,
) -> CmdOutput {
    let mut args = vec!["--provider", "github", "--format", "json"];
    if dry {
        args.push("--dry-run");
    }
    args.extend_from_slice(&[family, "hook", "batch", "--file", file.to_str().unwrap()]);
    run_forge_cli(stub, &args)
}

pub(super) fn url_row(target: &str, hook: u64, from: &str, to: &str) -> String {
    format!(
        r#"{{"target":"{target}","hook":{hook},"action":"set-url","from":"{from}","to":"{to}"}}"#
    )
}

pub(super) fn active_row(target: &str, hook: u64, from: bool, to: bool) -> String {
    format!(
        r#"{{"target":"{target}","hook":{hook},"action":"set-active","from":{from},"to":{to}}}"#
    )
}

#[test]
fn hook_list_redacts_the_secret() {
    let stub = stub(&[]);
    let out = repo_cmd(&stub, false, &["list"]);

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    never_leaks(&out);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], "cli.forge-cli.repo.hook.list.v1");
    assert_eq!(env["data"]["target"], "acme/widgets");
    let hooks = &env["data"]["hooks"];
    assert_eq!(hooks[0]["id"], 7);
    assert_eq!(hooks[0]["active"], true);
    assert_eq!(
        hooks[0]["events"],
        serde_json::json!(["push", "pull_request"])
    );
    assert_eq!(hooks[0]["content_type"], "json");
    assert_eq!(hooks[0]["url"], OLD);
    assert_eq!(hooks[0]["secret"], "[redacted]");
    assert_eq!(hooks[1]["secret"], serde_json::Value::Null);
}

#[test]
fn hook_list_text_never_prints_the_secret() {
    let stub = stub(&[]);
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "repo",
            "hook",
            "list",
        ],
    );

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    never_leaks(&out);
    assert!(out.stdout.contains(OLD), "{}", out.stdout);
}

#[test]
fn set_url_sends_only_the_url_and_never_the_secret() {
    let stub = stub(&[(7, OLD, true)]);
    let out = set_url(&stub, "7", OLD, NEW, false);

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    never_leaks(&out);
    assert_eq!(
        calls(&stub),
        vec![
            "api repos/acme/widgets/hooks/7".to_string(),
            format!("api repos/acme/widgets/hooks/7/config --method PATCH -f url={NEW}"),
            "api repos/acme/widgets/hooks/7".to_string(),
        ]
    );
    assert_eq!(stored(&stub, "url", 7), NEW);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], "cli.forge-cli.repo.hook.set-url.v1");
    assert_eq!(env["data"]["status"], "updated");
    assert_eq!(env["data"]["observed"], NEW);
}

#[test]
fn set_url_refuses_when_the_current_url_is_not_from() {
    let stub = stub(&[(7, "https://elsewhere.example.test/hook", true)]);
    let out = set_url(&stub, "7", OLD, NEW, false);

    assert_eq!(out.code, 65, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "hook_value_mismatch"
    );
    assert!(patches(&stub).is_empty());
    assert_eq!(
        stored(&stub, "url", 7),
        "https://elsewhere.example.test/hook"
    );
}

#[test]
fn set_url_rerun_after_success_is_a_clean_no_op() {
    let stub = stub(&[(7, NEW, true)]);
    let out = set_url(&stub, "7", OLD, NEW, false);

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(parse_envelope(&out.stdout)["data"]["status"], "already_set");
    assert!(patches(&stub).is_empty());
}

#[test]
fn set_url_rollback_is_the_same_command_with_from_and_to_swapped() {
    let stub = stub(&[(7, OLD, true)]);
    assert_eq!(set_url(&stub, "7", OLD, NEW, false).code, 0);
    let back = set_url(&stub, "7", NEW, OLD, false);

    assert_eq!(
        back.code, 0,
        "stdout={}\nstderr={}",
        back.stdout, back.stderr
    );
    assert_eq!(stored(&stub, "url", 7), OLD);
}

#[test]
fn set_url_dry_run_reads_but_never_writes() {
    let stub = stub(&[(7, OLD, true)]);
    let out = set_url(&stub, "7", OLD, NEW, true);

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["status"], "would_update");
    assert_eq!(env["data"]["dry_run"], true);
    assert!(patches(&stub).is_empty());
    assert_eq!(stored(&stub, "url", 7), OLD);
}

#[test]
fn set_url_rejects_identical_from_and_to_before_any_call() {
    let stub = stub(&[(7, OLD, true)]);
    let out = set_url(&stub, "7", OLD, OLD, false);

    assert_eq!(out.code, 65, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "hook_url_unchanged"
    );
    assert!(calls(&stub).is_empty());
}

#[test]
fn set_active_sends_only_the_flag_and_reruns_as_a_no_op() {
    let stub = stub(&[(7, OLD, true)]);
    let out = repo_cmd(&stub, false, &["set-active", "--hook", "7", "--inactive"]);

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    never_leaks(&out);
    assert_eq!(
        patches(&stub),
        vec!["api repos/acme/widgets/hooks/7 --method PATCH -F active=false".to_string()]
    );
    assert_eq!(stored(&stub, "active", 7), "false");
    assert_eq!(stored(&stub, "url", 7), OLD);
    let env = parse_envelope(&out.stdout);
    assert_eq!(
        env["schema_version"],
        "cli.forge-cli.repo.hook.set-active.v1"
    );
    assert_eq!(env["data"]["status"], "updated");
    assert_eq!(env["data"]["observed"], false);

    let again = repo_cmd(&stub, false, &["set-active", "--hook", "7", "--inactive"]);
    assert_eq!(again.code, 0);
    assert_eq!(
        parse_envelope(&again.stdout)["data"]["status"],
        "already_set"
    );
    assert_eq!(patches(&stub).len(), 1);
}

#[test]
fn set_active_dry_run_writes_nothing() {
    let stub = stub(&[(7, OLD, false)]);
    let out = repo_cmd(&stub, true, &["set-active", "--hook", "7", "--active"]);

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["data"]["status"],
        "would_update"
    );
    assert!(patches(&stub).is_empty());
}

#[test]
fn set_active_needs_exactly_one_state() {
    let stub = stub(&[(7, OLD, true)]);
    for tail in [
        vec!["set-active", "--hook", "7"],
        vec!["set-active", "--hook", "7", "--active", "--inactive"],
    ] {
        let out = repo_cmd(&stub, false, &tail);
        assert_eq!(out.code, 64, "{tail:?}\nstderr={}", out.stderr);
    }
    assert!(calls(&stub).is_empty());
}

#[test]
fn a_write_that_drops_the_secret_is_reported_as_a_readback_mismatch() {
    let stub = stub(&[(7, OLD, true)]);
    flag(&stub, "drop-secret");
    let out = repo_cmd(&stub, false, &["set-active", "--hook", "7", "--inactive"]);

    assert_eq!(out.code, 1, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "hook_readback_mismatch"
    );
}

#[test]
fn set_url_refuses_credentials_in_a_url() {
    let stub = stub(&[(7, OLD, true)]);
    let out = set_url(
        &stub,
        "7",
        OLD,
        "https://user:pw@new.example.test/hook",
        false,
    );

    assert_eq!(out.code, 65, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "hook_url_invalid"
    );
    assert!(calls(&stub).is_empty());
}

#[test]
fn a_failing_readback_after_the_write_is_reported_as_possibly_applied() {
    let stub = stub(&[(7, OLD, true)]);
    flag(&stub, "fail-after-patch");
    let out = set_url(&stub, "7", OLD, NEW, false);

    assert_eq!(out.code, 1, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["error"]["code"], "hook_readback_mismatch");
    assert!(
        env["error"]["message"]
            .as_str()
            .unwrap()
            .contains("may have been applied")
    );
    assert_eq!(stored(&stub, "url", 7), NEW, "the write did land");
}

#[test]
fn batch_stops_after_a_failing_readback() {
    let stub = stub(&[(7, OLD, true), (8, OLD, true)]);
    flag(&stub, "fail-after-patch");
    let file = batch_file(
        &stub,
        &format!(
            "[{},{}]",
            url_row("acme/widgets", 7, OLD, NEW),
            url_row("acme/widgets", 8, OLD, NEW)
        ),
    );
    let out = run_batch(&stub, "repo", &file, false);

    assert_eq!(out.code, 1, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    let rows = &env["data"]["rows"];
    assert_eq!(rows[0]["error"]["kind"], "hook_readback_mismatch");
    assert_eq!(rows[1]["status"], "skipped");
    assert_eq!(patches(&stub).len(), 1);
    assert_eq!(stored(&stub, "url", 8), OLD);
}

#[test]
fn batch_reports_each_row_and_exits_non_zero_when_any_row_fails() {
    let stub = stub(&[
        (7, OLD, true),
        (8, "https://elsewhere.example.test/hook", true),
        (9, NEW, true),
        (10, OLD, true),
    ]);
    let file = batch_file(
        &stub,
        &format!(
            "[{},{},{},{}]",
            url_row("acme/widgets", 7, OLD, NEW),
            url_row("acme/gadgets", 8, OLD, NEW),
            url_row("acme/widgets", 9, OLD, NEW),
            active_row("acme/widgets", 10, true, false),
        ),
    );
    let out = run_batch(&stub, "repo", &file, false);

    assert_eq!(out.code, 1, "stdout={}\nstderr={}", out.stdout, out.stderr);
    never_leaks(&out);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], "cli.forge-cli.repo.hook.batch.v1");
    let rows = &env["data"]["rows"];
    assert_eq!(rows[0]["status"], "updated");
    assert_eq!(rows[1]["status"], "failed");
    assert_eq!(rows[1]["error"]["kind"], "hook_value_mismatch");
    assert_eq!(rows[2]["status"], "already_set");
    assert_eq!(rows[3]["status"], "updated");
    assert_eq!(rows[3]["action"], "set-active");
    assert_eq!(env["data"]["failed"], 1);
    assert_eq!(patches(&stub).len(), 2, "only matching rows write");
    assert_eq!(stored(&stub, "url", 7), NEW);
    assert_eq!(
        stored(&stub, "url", 8),
        "https://elsewhere.example.test/hook"
    );
    assert_eq!(stored(&stub, "active", 10), "false");
}

#[test]
fn batch_text_prints_one_line_per_row() {
    let stub = stub(&[(7, OLD, true), (9, NEW, true)]);
    let file = batch_file(
        &stub,
        &format!(
            "[{},{}]",
            url_row("acme/widgets", 7, OLD, NEW),
            url_row("acme/widgets", 9, OLD, NEW)
        ),
    );
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "repo",
            "hook",
            "batch",
            "--file",
            file.to_str().unwrap(),
        ],
    );

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(lines.len(), 2, "{}", out.stdout);
    assert!(lines[0].contains("acme/widgets") && lines[0].contains("updated"));
    assert!(lines[1].contains("already_set"));
}

#[test]
fn batch_stops_after_a_readback_mismatch() {
    let stub = stub(&[(7, OLD, true), (8, OLD, true)]);
    flag(&stub, "drop-secret");
    let file = batch_file(
        &stub,
        &format!(
            "[{},{}]",
            active_row("acme/widgets", 7, true, false),
            url_row("acme/widgets", 8, OLD, NEW)
        ),
    );
    let out = run_batch(&stub, "repo", &file, false);

    assert_eq!(out.code, 1, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    let rows = &env["data"]["rows"];
    assert_eq!(rows[0]["error"]["kind"], "hook_readback_mismatch");
    assert_eq!(rows[1]["status"], "skipped");
    assert_eq!(patches(&stub).len(), 1);
    assert_eq!(stored(&stub, "url", 8), OLD);
}

#[test]
fn batch_validates_the_whole_file_before_any_call() {
    let stub = stub(&[(7, OLD, true)]);
    let good = url_row("acme/widgets", 7, OLD, NEW);
    for rows in [
        // A good row followed by a row whose URL is not absolute.
        format!(
            r#"[{good},{{"target":"acme/widgets","hook":8,"action":"set-url","from":"{OLD}","to":"not a url"}}]"#
        ),
        // The same field of the same hook twice has no defined order.
        format!("[{good},{good}]"),
        // Unknown keys are refused rather than ignored.
        format!(
            r#"[{good},{{"target":"acme/widgets","hook":8,"action":"set-url","from":"{OLD}","to":"{NEW}","secret":"x"}}]"#
        ),
        // Wrong value types for the action.
        format!(
            r#"[{good},{{"target":"acme/widgets","hook":8,"action":"set-active","from":"{OLD}","to":"{NEW}"}}]"#
        ),
        format!(
            r#"[{good},{{"target":"acme/widgets","hook":8,"action":"delete","from":true,"to":false}}]"#
        ),
        // Embedded credentials are refused.
        format!(
            r#"[{good},{{"target":"acme/widgets","hook":8,"action":"set-url","from":"{OLD}","to":"https://user:pw@new.example.test/hook"}}]"#
        ),
        // An organization is not a repository target, and from == to is refused.
        format!("[{}]", url_row("acme", 7, OLD, NEW)),
        format!("[{}]", active_row("acme/widgets", 7, true, true)),
        "[]".to_string(),
    ] {
        let file = batch_file(&stub, &rows);
        let out = run_batch(&stub, "repo", &file, false);

        assert_eq!(
            out.code, 65,
            "{rows}\nstdout={}\nstderr={}",
            out.stdout, out.stderr
        );
        assert_eq!(
            parse_envelope(&out.stdout)["error"]["code"],
            "batch_invalid",
            "{rows}"
        );
        assert!(
            calls(&stub).is_empty(),
            "{rows}: no call may precede validation"
        );
    }
}

#[test]
fn batch_dry_run_prints_the_plan_and_writes_nothing() {
    let stub = stub(&[(7, OLD, true), (8, OLD, true)]);
    let file = batch_file(
        &stub,
        &format!(
            "[{},{}]",
            url_row("acme/widgets", 7, OLD, NEW),
            active_row("acme/gadgets", 8, true, false)
        ),
    );
    let out = run_batch(&stub, "repo", &file, true);

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["dry_run"], true);
    assert_eq!(env["data"]["rows"][0]["status"], "would_update");
    assert_eq!(env["data"]["rows"][1]["status"], "would_update");
    assert!(patches(&stub).is_empty());
}

#[test]
fn batch_refuses_a_global_repo() {
    let stub = stub(&[(7, OLD, true)]);
    let file = batch_file(
        &stub,
        &format!("[{}]", url_row("acme/widgets", 7, OLD, NEW)),
    );
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "repo",
            "hook",
            "batch",
            "--file",
            file.to_str().unwrap(),
        ],
    );

    assert_eq!(out.code, 65, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "repo_conflict"
    );
    assert!(calls(&stub).is_empty());
}

#[test]
fn set_url_requires_hook_from_and_to() {
    let stub = stub(&[(7, OLD, true)]);
    let out = repo_cmd(&stub, false, &["set-url", "--hook", "7"]);

    assert_eq!(out.code, 64, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert!(calls(&stub).is_empty());
}

#[cfg(unix)]
pub(super) mod identity {
    use super::{NEW, OLD, active_row, url_row};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    const POLICY: &str = include_str!("../../../nils-common/tests/fixtures/identity/policy.toml");
    pub(in super::super) const CREDENTIAL: &str = "FIXTURE_CANARY_NEVER_OUTPUT_83";

    /// A home with a single-profile identity policy and a `gh` that counts
    /// actor probes and managed calls, serving hooks 7 and 8 under any target.
    pub(in super::super) struct Fixture {
        pub home: tempfile::TempDir,
        gh: std::path::PathBuf,
    }

    impl Fixture {
        pub fn new() -> Self {
            let policy = POLICY
                .replace(
                    "profiles = [\"account-a\", \"account-b\"]",
                    "profiles = [\"account-a\"]",
                )
                .replace(
                    "org = \"github.com/sandbox\"\nprofile = \"account-b\"",
                    "org = \"github.com/sandbox\"\nprofile = \"account-a\"",
                );
            let home = tempfile::tempdir().unwrap();
            fs::create_dir(home.path().join("forge-cli")).unwrap();
            fs::write(home.path().join("forge-cli/identity.toml"), policy).unwrap();
            for id in [7, 8] {
                fs::write(home.path().join(format!("url-{id}")), OLD).unwrap();
                fs::write(home.path().join(format!("active-{id}")), "true").unwrap();
            }
            let gh = home.path().join("gh");
            fs::write(
                &gh,
                format!(
                    r#"#!/bin/sh
dir={dir}
case "$1:$2" in
 api:user) printf 'probe\n' >> "$dir/probes"; printf '{{"login":"account-a"}}'; exit 0;;
esac
test "$GH_TOKEN" = "$FIXTURE_ACCOUNT_A_CREDENTIAL" && test -z "$GITHUB_TOKEN" || exit 9
printf '%s\n' "$*" >> "$dir/calls"
id=${{2%/config}}; id=${{id##*/}}
if [ "$#" -gt 2 ]; then
  case "$6" in url=*) printf '%s' "${{6#url=}}" > "$dir/url-$id";; active=*) printf '%s' "${{6#active=}}" > "$dir/active-$id";; esac
fi
printf '{{"id":%s,"active":%s,"events":["push"],"config":{{"url":"%s","content_type":"json","insecure_ssl":"0"}}}}' "$id" "$(cat "$dir/active-$id")" "$(cat "$dir/url-$id")"
"#,
                    dir = home.path().display()
                ),
            )
            .unwrap();
            fs::set_permissions(&gh, fs::Permissions::from_mode(0o700)).unwrap();
            Self { home, gh }
        }

        pub fn command(&self, principal: &str) -> Command {
            let mut cmd = Command::new(super::super::support::forge_cli_bin());
            cmd.current_dir(self.home.path())
                .args(["--provider", "github", "--format", "json"])
                .env("HOME", self.home.path())
                .env("XDG_CACHE_HOME", self.home.path().join("cache"))
                .env("XDG_CONFIG_HOME", self.home.path())
                .env("XDG_STATE_HOME", self.home.path().join("state"))
                .env("FORGE_IDENTITY_PRINCIPAL", principal)
                .env("FORGE_CLI_GH_BIN", &self.gh)
                .env("FORGE_CLI_RATE_LIMIT_GATE", "off")
                .env("FIXTURE_ACCOUNT_A_CREDENTIAL", CREDENTIAL);
            cmd
        }

        pub fn count(&self, name: &str) -> usize {
            fs::read_to_string(self.home.path().join(name))
                .map(|text| text.lines().count())
                .unwrap_or(0)
        }
    }

    #[test]
    fn a_batch_across_repositories_verifies_the_actor_once() {
        let f = Fixture::new();
        let rows = f.home.path().join("rows.json");
        fs::write(
            &rows,
            format!(
                "[{},{},{}]",
                url_row("sandbox/widget", 7, OLD, NEW),
                url_row("sandbox/gadget", 8, OLD, NEW),
                active_row("sandbox/gadget", 8, true, false),
            ),
        )
        .unwrap();

        let out = f
            .command("contributor")
            .args(["repo", "hook", "batch", "--file"])
            .arg(&rows)
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(0), "{stdout}");
        assert_eq!(f.count("calls"), 9, "three rows of read, write, read-back");
        assert_eq!(f.count("probes"), 1, "one actor probe per process");
        assert_eq!(
            fs::read_to_string(f.home.path().join("url-8")).unwrap(),
            NEW
        );
        assert!(!stdout.contains(CREDENTIAL));
    }
}
