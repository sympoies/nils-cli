//! Hermetic workflow rerun fencing, identity, and single-write receipts.
use super::support::{StubEnv, parse_envelope, run_forge_cli};
use pretty_assertions::{assert_eq, assert_ne};
use std::fs;

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const STUB: &str = r#"#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
path = next((a for a in args if a.startswith('repos/')), 'user')
post = 'POST' in args
root = os.environ['FIXTURE_ROOT']
actor = 'account-b' if os.environ.get('GH_TOKEN') == 'fixture-b' else 'account-a'
if path == 'user':
    print(json.dumps({'login': actor})); sys.exit(0)
with open(root + '/calls', 'a') as f: f.write(('POST ' if post else 'GET ') + path + '\n')
head = 'a' * 40
attempt = 2 if os.path.exists(root + '/posted') else 1
run = {'id':42, 'head_sha':head, 'run_attempt':attempt, 'html_url':'https://github.com/example/widget/actions/runs/42', 'event':os.environ.get('EVENT', 'pull_request'), 'status':'completed', 'pull_requests':([] if os.environ.get('NO_PRS') else [{'number':7}]), 'triggering_actor':{'login':actor}}
if '/commits/' in path:
    print(json.dumps([[{'number':7}]])); sys.exit(0)
if post:
    with open(root + '/posted', 'a') as f: f.write(path + '\n')
    if os.environ.get('POST_FAIL'):
        print(os.environ.get('GH_TOKEN', 'provider unavailable'), file=sys.stderr); sys.exit(1)
    sys.exit(0)
if path.endswith('/pulls/7'):
    print(json.dumps({'number':7, 'head':{'sha':os.environ.get('PR_HEAD', head)}}))
elif '/jobs/19' in path:
    print(json.dumps({'id':19, 'run_id':int(os.environ.get('JOB_RUN', '42')), 'run_attempt':int(os.environ.get('JOB_ATTEMPT', '1')), 'head_sha':head, 'name':'test'}))
elif path.endswith('/jobs?per_page=100'):
    print(json.dumps([{'jobs':[{'id':119, 'run_id':42, 'run_attempt':2, 'head_sha':head, 'name':'test', 'html_url':'https://github.com/example/widget/actions/runs/42/job/119'}]}]))
elif path.endswith('/runs/42'):
    if os.environ.get('RECEIPT_LAG'): run['run_attempt'] = 1
    if os.environ.get('RECEIPT_FAIL') and attempt == 2: sys.exit(1)
    if os.environ.get('SUPERSEDED') and attempt == 2: run['run_attempt'] = 3
    print(json.dumps(run))
else:
    print('unexpected fixture request: ' + path, file=sys.stderr); sys.exit(9)
"#;

fn fixture() -> StubEnv {
    let stub = StubEnv::new().gh_stub(STUB);
    let root = stub.tempdir.path().to_string_lossy().into_owned();
    stub.env("FIXTURE_ROOT", root)
}
fn invoke(stub: &StubEnv, args: &[&str]) -> super::support::CmdOutput {
    let mut argv = vec![
        "--provider",
        "github",
        "--repo",
        "example/widget",
        "--format",
        "json",
        "run",
        "rerun",
        "42",
        "--expected-head",
        HEAD,
        "--receipt-timeout-secs",
        "0",
    ];
    argv.extend_from_slice(args);
    run_forge_cli(stub, &argv)
}
fn posts(stub: &StubEnv) -> String {
    fs::read_to_string(stub.tempdir.path().join("posted")).unwrap_or_default()
}
#[test]
fn run_rerun_reports_new_attempt_and_jobs_for_each_selection() {
    for (args, endpoint, selection) in [
        (vec![], "runs/42/rerun", "all"),
        (vec!["--failed-only"], "runs/42/rerun-failed-jobs", "failed"),
        (vec!["--job", "19"], "jobs/19/rerun", "job"),
    ] {
        let stub = fixture();
        let out = invoke(&stub, &args);
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        let data = parse_envelope(&out.stdout)["data"].clone();
        assert_eq!(data["run_id"], 42);
        assert_eq!(data["previous_attempt"], 1);
        assert_eq!(data["attempt"], 2);
        assert_eq!(data["receipt_status"], "observed");
        assert_eq!(data["selection"], selection);
        assert_eq!(data["actor"], "account-a");
        assert_eq!(data["jobs"][0]["id"], 119);
        assert_eq!(
            data["attempt_url"],
            "https://github.com/example/widget/actions/runs/42/attempts/2"
        );
        assert_eq!(
            posts(&stub),
            format!("repos/example/widget/actions/{endpoint}\n")
        );
    }
}
#[test]
fn run_rerun_refuses_head_and_current_pr_head_mismatches_without_writes() {
    for stub in [fixture().env("PR_HEAD", "b".repeat(40)), fixture()] {
        let args = if stub.envs.iter().any(|(key, _)| key == "PR_HEAD") {
            vec![]
        } else {
            vec![
                "--expected-head",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ]
        };
        // A separate invocation avoids clap's repeated-argument error.
        let out = if args.is_empty() {
            invoke(&stub, &[])
        } else {
            run_forge_cli(
                &stub,
                &[
                    "--provider",
                    "github",
                    "--repo",
                    "example/widget",
                    "--format",
                    "json",
                    "run",
                    "rerun",
                    "42",
                    "--expected-head",
                    args[1],
                ],
            )
        };
        assert_eq!(out.code, 65, "{} {}", out.stdout, out.stderr);
        assert!(out.stdout.contains("head_mismatch"));
        assert!(posts(&stub).is_empty());
    }
}
#[test]
fn run_rerun_selected_job_must_belong_to_current_attempt() {
    for (key, value) in [("JOB_RUN", "99"), ("JOB_ATTEMPT", "0")] {
        let stub = fixture().env(key, value);
        let out = invoke(&stub, &["--job", "19"]);
        assert_eq!(out.code, 65, "{}", out.stdout);
        assert!(out.stdout.contains("job_mismatch"));
        assert!(posts(&stub).is_empty());
    }
}
#[test]
fn run_rerun_dry_run_checks_fences_but_never_posts() {
    let stub = fixture();
    let out = invoke(&stub, &["--job", "19", "--dry-run"]);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert!(parse_envelope(&out.stdout)["data"]["plan"].is_array());
    assert!(posts(&stub).is_empty());
    let stub = fixture().env("PR_HEAD", "b".repeat(40));
    assert_eq!(invoke(&stub, &["--dry-run"]).code, 65);
    assert!(posts(&stub).is_empty());
}
#[test]
fn run_rerun_never_replays_write_for_failure_or_delayed_receipt() {
    let stub = fixture().env("POST_FAIL", "1");
    let out = invoke(&stub, &[]);
    assert_ne!(out.code, 0);
    assert_eq!(posts(&stub).lines().count(), 1);
    let stub = fixture().env("RECEIPT_LAG", "1");
    let out = invoke(&stub, &[]);
    assert_eq!(out.code, 0, "{}", out.stdout);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["receipt_status"], "pending");
    assert!(data["attempt"].is_null());
    assert!(data["jobs"].as_array().unwrap().is_empty());
    assert_eq!(posts(&stub).lines().count(), 1);
}
#[test]
fn run_rerun_selected_identity_is_audited_and_secrets_are_not_printed() {
    let stub = fixture();
    let config = stub.tempdir.path().join("config");
    fs::create_dir_all(config.join("forge-cli")).unwrap();
    fs::write(
        config.join("forge-cli/identity.toml"),
        r#"version=1
[credentials.b]
kind='env'
name='FIXTURE_WRITE_B'
[profiles.b]
expected_login='account-b'
credential='b'
commit_name='Example Contributor'
commit_email='contributor@example.invalid'
signing_fingerprint='BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB'
operations=['api_read','api_write']
[principals.writer]
profiles=['b']
[[rules]]
id='writer'
principal='writer'
profile='b'
repo='github.com/example/widget'
"#,
    )
    .unwrap();
    let stub = stub
        .env("XDG_CONFIG_HOME", config.to_string_lossy())
        .env("FORGE_IDENTITY_PRINCIPAL", "writer")
        .env("FIXTURE_WRITE_B", "fixture-b")
        .env("GH_TOKEN", "ambient-canary");
    let out = invoke(&stub, &["--job", "19"]);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert_eq!(parse_envelope(&out.stdout)["data"]["actor"], "account-b");
    let audit = fs::read_to_string(
        stub.tempdir
            .path()
            .join("xdg-state/forge-cli/identity-audit.jsonl"),
    )
    .unwrap();
    assert!(audit.contains("account-b"));
    assert!(audit.contains("api_write"));
    assert!(audit.contains("execution_succeeded"));
    for canary in ["fixture-b", "ambient-canary"] {
        assert!(!out.stdout.contains(canary));
        assert!(!out.stderr.contains(canary));
        assert!(!audit.contains(canary));
    }
    let policy_path = config.join("forge-cli/identity.toml");
    let policy = fs::read_to_string(&policy_path).unwrap();
    fs::write(
        &policy_path,
        policy.replace("'api_read','api_write'", "'api_read'"),
    )
    .unwrap();
    let denied = invoke(&stub, &[]);
    assert_eq!(denied.code, 65, "{}", denied.stdout);
    assert_eq!(posts(&stub).lines().count(), 1);
    fs::write(policy_path, policy).unwrap();
    let failed = invoke(&stub.env("POST_FAIL", "1"), &[]);
    assert_ne!(failed.code, 0);
    for canary in ["fixture-b", "ambient-canary"] {
        assert!(!failed.stdout.contains(canary));
        assert!(!failed.stderr.contains(canary));
    }
}

#[test]
fn run_rerun_missing_pr_metadata_fails_closed_and_push_runs_discover_prs() {
    let stub = fixture().env("NO_PRS", "1");
    let out = invoke(&stub, &[]);
    assert_eq!(out.code, 65, "{}", out.stdout);
    assert!(out.stdout.contains("run_pr_head_unavailable"));
    assert!(posts(&stub).is_empty());
    let stub = fixture()
        .env("NO_PRS", "1")
        .env("EVENT", "push")
        .env("PR_HEAD", "b".repeat(40));
    let out = invoke(&stub, &[]);
    assert_eq!(out.code, 65, "{}", out.stdout);
    assert!(out.stdout.contains("pr_head_mismatch"));
    assert!(posts(&stub).is_empty());
}
#[test]
fn run_rerun_receipt_read_errors_and_concurrent_attempts_do_not_replay() {
    for (key, status) in [("RECEIPT_FAIL", "pending"), ("SUPERSEDED", "superseded")] {
        let stub = fixture().env(key, "1");
        let out = invoke(&stub, &[]);
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        let data = parse_envelope(&out.stdout)["data"].clone();
        assert_eq!(data["rerun_requested"], true);
        assert_eq!(data["receipt_status"], status);
        assert!(data["attempt"].is_null());
        assert_eq!(posts(&stub).lines().count(), 1);
    }
}
#[test]
fn run_rerun_declares_write_effect_and_rejects_conflicting_or_invalid_arguments() {
    let stub = fixture();
    let out = run_forge_cli(
        &stub,
        &[
            "operation-effect",
            "--format",
            "json",
            "--",
            "run",
            "rerun",
            "42",
            "--expected-head",
            HEAD,
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stdout);
    let data = parse_envelope(&out.stdout)["data"].clone();
    assert_eq!(data["operation"], "run.rerun");
    assert_eq!(data["effect"], "mutation");
    assert_eq!(data["provider_effect"], "network_write");
    for (args, code) in [
        (vec!["--job", "19", "--failed-only"], 64),
        (vec!["--job", "0"], 65),
    ] {
        let out = invoke(&stub, &args);
        assert_eq!(out.code, code, "{}", out.stdout);
        assert!(posts(&stub).is_empty());
    }
    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "--provider",
            "github",
            "--repo",
            "example/widget",
            "run",
            "rerun",
            "42",
        ],
    );
    assert_eq!(out.code, 64);
    let out = run_forge_cli(
        &stub,
        &[
            "--format",
            "json",
            "--provider",
            "gitlab",
            "--repo",
            "example/widget",
            "run",
            "rerun",
            "42",
            "--expected-head",
            HEAD,
        ],
    );
    assert_eq!(out.code, 64);
    assert!(out.stdout.contains("provider_unsupported"));
    assert!(posts(&stub).is_empty());
}
