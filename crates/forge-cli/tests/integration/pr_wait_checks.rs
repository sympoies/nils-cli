//! End-to-end `pr wait-checks` integration tests.
//!
//! Each test wires the gh stub to emit a *sequence* of check-snapshot JSON
//! payloads, one per poll, so the polling loop's exit-code matrix can be
//! exercised deterministically against tiny `--interval`/`--timeout` values:
//!
//! - succeeds on third poll → `SUCCESS 0`
//! - first poll required failure → `RUNTIME 1` + `error.kind=checks_failed`
//! - times out after N intervals → `UNAVAILABLE 69` + `error.kind=checks_timeout`

use pretty_assertions::assert_eq;

use super::support::{StubEnv, parse_envelope, run_forge_cli};

/// Build a dispatching gh stub that maintains a per-invocation counter file
/// and returns the Nth element of a snapshot sequence each time `pr checks`
/// is invoked. Once the counter passes the last in-range index it clamps to
/// the final snapshot, so callers that poll longer than the prepared
/// sequence keep seeing the trailing snapshot (essential for timeout tests).
fn gh_sequence_stub(stub: &StubEnv, sequence: &[&str]) {
    gh_sequence_stub_with(stub, sequence, false);
}

/// Like [`gh_sequence_stub`], but when `no_required_checks` is set the
/// `--required` list answers the way `gh` does for a repository without
/// branch-protection required checks: empty stdout, a "no required checks
/// reported" notice on stderr, and exit 1.
fn gh_sequence_stub_with(stub: &StubEnv, sequence: &[&str], no_required_checks: bool) {
    assert!(!sequence.is_empty(), "sequence must have at least one snap");
    for (idx, snap) in sequence.iter().enumerate() {
        let path = stub.tempdir.path().join(format!("snap-{idx}.json"));
        std::fs::write(&path, snap).expect("write snap");
    }
    let counter = stub.tempdir.path().join("counter");
    std::fs::write(&counter, "0").expect("write counter");
    let dir = stub.tempdir.path().to_string_lossy().to_string();
    let max_idx = sequence.len() - 1;
    let body = format!(
        r#"#!/bin/sh
set -e
case "$1 $2" in
  "pr checks")
    counter="{dir}/counter"
    idx=$(cat "$counter")
    case " $* " in
      *" --required "*)
        if [ "{no_required_checks}" = "true" ]; then
            echo "no required checks reported on the 'feature' branch" >&2
            exit 1
        fi
        idx=$((idx - 1))
        if [ "$idx" -lt 0 ]; then
            idx=0
        fi
        ;;
      *)
        next=$((idx + 1))
        echo "$next" > "$counter"
        ;;
    esac
    if [ "$idx" -ge "{max_idx}" ]; then
        eff="{max_idx}"
    else
        eff="$idx"
    fi
    cat "{dir}/snap-$eff.json"
    ;;
  *)
    echo "stub: unexpected gh args: $*" >&2
    exit 99
    ;;
esac
"#,
    );
    stub.write_gate_stub(&body);
}

const PENDING_SNAP: &str =
    r#"[{"name":"build","bucket":"pending","state":"IN_PROGRESS","link":"https://ci/1"}]"#;
const SUCCESS_SNAP: &str =
    r#"[{"name":"build","bucket":"pass","state":"COMPLETED","link":"https://ci/1"}]"#;
const QUEUED_SNAP: &str =
    r#"[{"name":"build","bucket":"pending","state":"QUEUED","link":"https://ci/1"}]"#;
const FAILURE_SNAP: &str =
    r#"[{"name":"build","bucket":"fail","state":"COMPLETED","link":"https://ci/1"}]"#;

#[test]
fn pr_wait_checks_succeeds_when_terminal_on_third_poll() {
    let mut stub = StubEnv::new();
    gh_sequence_stub(&stub, &[PENDING_SNAP, PENDING_SNAP, SUCCESS_SNAP]);
    let gh_path = stub.tempdir.path().join("gh");
    stub = stub.env("FORGE_CLI_GH_BIN", gh_path.to_string_lossy());

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--interval",
            "10ms",
            "--timeout",
            "20s",
        ],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], "cli.forge-cli.pr.checks.v1");
    assert_eq!(env["data"]["state"], "success");
    assert!(env["data"]["duration_ms"].as_u64().is_some());
}

#[test]
fn pr_wait_checks_required_failure_exits_runtime_with_kind_checks_failed() {
    let mut stub = StubEnv::new();
    gh_sequence_stub(&stub, &[FAILURE_SNAP]);
    let gh_path = stub.tempdir.path().join("gh");
    stub = stub.env("FORGE_CLI_GH_BIN", gh_path.to_string_lossy());

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--interval",
            "10ms",
            "--timeout",
            "1s",
        ],
    );
    assert_eq!(out.code, 1, "expected RUNTIME 1, stderr={}", out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"]["code"], "checks_failed");
    // Payload still carries the snapshot so callers can introspect.
    assert_eq!(env["data"]["state"], "failure");
    assert_eq!(env["data"]["failed"].as_array().unwrap().len(), 1);
}

#[test]
fn pr_wait_checks_timeout_exits_unavailable_with_kind_checks_timeout() {
    let mut stub = StubEnv::new();
    gh_sequence_stub(
        &stub,
        &[PENDING_SNAP, PENDING_SNAP, PENDING_SNAP, PENDING_SNAP],
    );
    let gh_path = stub.tempdir.path().join("gh");
    stub = stub.env("FORGE_CLI_GH_BIN", gh_path.to_string_lossy());

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--interval",
            "20ms",
            "--timeout",
            "100ms",
        ],
    );
    assert_eq!(
        out.code, 69,
        "expected UNAVAILABLE 69 on timeout, stderr={}",
        out.stderr
    );
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["error"]["code"], "checks_timeout");
    let duration_ms = env["data"]["duration_ms"].as_u64().expect("duration_ms");
    // Sleeping at least one interval before the timeout fires means
    // duration_ms must be > 0 and within a reasonable upper bound.
    assert!(duration_ms >= 20, "duration_ms={duration_ms}");
}

#[test]
fn pr_wait_checks_dry_run_renders_plan_envelope_without_calling_backend() {
    // The stub should never run during dry-run; configure it to exit 99 to
    // assert that.
    let stub = StubEnv::new().gh_gate_stub("#!/bin/sh\necho 'should not run' >&2\nexit 99\n");

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--dry-run",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "42",
            "--interval",
            "5s",
            "--timeout",
            "1m",
        ],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], "cli.forge-cli.pr.checks.v1");
    let timeout_ms = env["data"]["timeout_ms"].as_u64().expect("timeout_ms");
    let interval_ms = env["data"]["interval_ms"].as_u64().expect("interval_ms");
    assert_eq!(timeout_ms, 60_000);
    assert_eq!(interval_ms, 5_000);
}

#[test]
fn pr_wait_checks_gitlab_api_succeeds_without_version_probe() {
    let stub = StubEnv::new().glab_stub(
        r#"#!/bin/sh
set -e
case "$1" in
  "--version")
    echo "version probe should not run for API-backed wait-checks" >&2
    exit 99
    ;;
  "mr")
    if [ "$2" = "view" ]; then
      cat <<'EOF'
{
  "iid": 42,
  "web_url": "https://gitlab.com/group/project/-/merge_requests/42",
  "source_branch": "feat/sample",
  "target_branch": "main",
  "sha": "abc123",
  "head_pipeline": {
    "id": 99,
    "status": "success",
    "web_url": "https://gitlab.com/group/project/-/pipelines/99"
  }
}
EOF
      exit 0
    fi
    ;;
  "api")
    case "$*" in
      *"projects/group%2Fproject/pipelines/99/jobs?per_page=100"*)
        cat <<'EOF'
[
  {
    "name": "build",
    "stage": "test",
    "status": "success",
    "allow_failure": false,
    "web_url": "https://gitlab.com/group/project/-/jobs/1"
  }
]
EOF
        exit 0
        ;;
    esac
    ;;
esac
echo "stub: unexpected glab args: $*" >&2
exit 99
"#,
    );

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "gitlab",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "42",
            "--interval",
            "10ms",
            "--timeout",
            "1s",
        ],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], "cli.forge-cli.pr.checks.v1");
    assert_eq!(env["data"]["provider"], "gitlab");
    assert_eq!(env["data"]["state"], "success");
    assert_eq!(env["data"]["required_count"], 1);
}

const EMPTY_SNAP: &str = "[]";

/// The emitter arm for a head that never registers a check: DATA 65 and its own
/// kind, not the UNAVAILABLE 69 a genuine timeout gets. The two need different
/// fixes, so automation must be able to tell them apart.
#[test]
fn pr_wait_checks_expires_as_not_registered_when_nothing_is_ever_reported() {
    let mut stub = StubEnv::new();
    gh_sequence_stub(&stub, &[EMPTY_SNAP]);
    let gh_path = stub.tempdir.path().join("gh");
    stub = stub.env("FORGE_CLI_GH_BIN", gh_path.to_string_lossy());

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--interval",
            "10ms",
            "--timeout",
            "30ms",
        ],
    );

    assert_eq!(out.code, 65, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["error"]["code"], "checks_not_registered");
    assert!(
        env["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("[checks] none = true")),
        "the envelope must name the opt-outs: {}",
        out.stdout
    );
    // The snapshot is reported verbatim, so `data.state` is still "success"
    // while `ok` is false. Consumers must gate on ok / error.kind — this
    // asserts the combination so any future normalization is deliberate.
    assert_eq!(env["ok"], false);
    assert_eq!(env["data"]["state"], "success");
    assert_eq!(env["data"]["required_count"], 0);
}

/// The reason-free opt-out on this non-mutating op: a project that configures
/// no checks terminates immediately instead of burning the whole budget.
#[test]
fn pr_wait_checks_allow_no_checks_completes_immediately() {
    let mut stub = StubEnv::new();
    gh_sequence_stub(&stub, &[EMPTY_SNAP]);
    let gh_path = stub.tempdir.path().join("gh");
    stub = stub.env("FORGE_CLI_GH_BIN", gh_path.to_string_lossy());

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--interval",
            "10ms",
            "--timeout",
            "30s",
            "--allow-no-checks",
        ],
    );

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["required_count"], 0);
}

/// `[checks] none` in `.forge-cli.toml` is the durable form of
/// `--allow-no-checks`: the wait completes at once without the flag.
#[test]
fn pr_wait_checks_repo_declared_no_checks_completes_immediately() {
    let mut stub = StubEnv::new();
    gh_sequence_stub(&stub, &[EMPTY_SNAP]);
    std::fs::write(
        stub.tempdir.path().join(".forge-cli.toml"),
        "[checks]\nnone = true\nnone_reason = \"no CI in this repository\"\n",
    )
    .expect("write config");
    let gh_path = stub.tempdir.path().join("gh");
    stub = stub.env("FORGE_CLI_GH_BIN", gh_path.to_string_lossy());

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--interval",
            "10ms",
            "--timeout",
            "30s",
        ],
    );

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["required_count"], 0);
}

/// #2018: a repository without branch-protection required checks still runs
/// CI. While that visible check is queued or in progress the wait must keep
/// polling, the same way `pr deliver` gates its visible checks, instead of
/// reporting success over the empty required set.
#[test]
fn pr_wait_checks_without_required_checks_waits_for_visible_checks() {
    let mut stub = StubEnv::new();
    gh_sequence_stub_with(&stub, &[QUEUED_SNAP, PENDING_SNAP, SUCCESS_SNAP], true);
    let gh_path = stub.tempdir.path().join("gh");
    stub = stub.env("FORGE_CLI_GH_BIN", gh_path.to_string_lossy());

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--interval",
            "10ms",
            "--timeout",
            "20s",
        ],
    );

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["state"], "success");
    assert_eq!(env["data"]["required_count"], 1);
    assert_eq!(env["data"]["success_count"], 1);
    let polls = std::fs::read_to_string(stub.tempdir.path().join("counter")).expect("counter");
    assert_eq!(polls.trim(), "3", "success must wait for the third poll");
}

/// A visible check that stays queued must expire as `checks_timeout`, never
/// as success.
#[test]
fn pr_wait_checks_without_required_checks_times_out_on_a_queued_check() {
    let mut stub = StubEnv::new();
    gh_sequence_stub_with(&stub, &[QUEUED_SNAP], true);
    let gh_path = stub.tempdir.path().join("gh");
    stub = stub.env("FORGE_CLI_GH_BIN", gh_path.to_string_lossy());

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--interval",
            "10ms",
            "--timeout",
            "50ms",
        ],
    );

    assert_eq!(out.code, 69, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"]["code"], "checks_timeout");
    assert_eq!(env["data"]["state"], "pending");
    assert_eq!(env["data"]["pending"][0]["name"], "build");
}

/// Head-pinned provider fixture: the rollup is green, but the exact
/// current head is still registering the base branch's required checks.
mod registration {
    use std::cell::{Cell, RefCell};
    use std::time::{Duration, Instant};

    use forge_cli::backend::{BackendCall, BackendRunner, BackendSuccess};
    use forge_cli::cli::{GlobalFlags, PrWaitChecksArgs};
    use forge_cli::error::ForgeError;
    use forge_cli::ops::pr_wait_checks::{Clock, WaitOutcome, compute};
    use forge_cli::provider::{DetectionSource, Provider, ProviderContext};
    use pretty_assertions::assert_eq;

    struct TestClock(Cell<Instant>);
    impl Clock for TestClock {
        fn now(&self) -> Instant {
            self.0.get()
        }
        fn sleep(&self, d: Duration) {
            self.0.set(self.0.get() + d);
        }
    }

    struct RegistrationRunner {
        polls: Cell<usize>,
        late: bool,
        move_head: bool,
        requested_heads: RefCell<Vec<String>>,
    }
    impl BackendRunner for RegistrationRunner {
        fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
            let argv: Vec<_> = call.argv.iter().map(|s| s.to_string_lossy()).collect();
            let text = match (argv[0].as_ref(), argv[1].as_ref()) {
                // Old gh rollups can contain only a fast optional check.
                ("pr", "checks") => r#"[{"name":"optional","bucket":"pass"}]"#.to_string(),
                ("pr", "view") => {
                    let sha = if self.move_head && self.polls.get() > 0 { "new-head" } else { "head" };
                    format!(r#"{{"headRefOid":"{sha}","baseRefName":"main","url":"https://github.com/example/project/pull/42"}}"#)
                }
                ("api", "graphql") =>
                    r#"{"data":{"repository":{"ref":{"branchProtectionRule":{"requiredStatusChecks":[{"context":"test","app":null},{"context":"coverage","app":null}]}}}}}"#.to_string(),
                ("api", endpoint) if endpoint.contains("/rules/branches/") => "[]".to_string(),
                ("api", endpoint) if endpoint.contains("/check-runs?") => {
                    self.requested_heads.borrow_mut().push(endpoint.to_string());
                    self.polls.set(self.polls.get() + 1);
                    if self.move_head || (self.late && self.polls.get() >= 3) {
                        r#"{"total_count":2,"check_runs":[{"name":"test","status":"completed","conclusion":"success"},{"name":"coverage","status":"completed","conclusion":"success"}]}"#.to_string()
                    } else {
                        r#"{"total_count":1,"check_runs":[{"name":"optional","status":"completed","conclusion":"success"}]}"#.to_string()
                    }
                }
                ("api", endpoint) if endpoint.contains("/status?") => r#"{"total_count":0,"statuses":[]}"#.to_string(),
                _ => panic!("unexpected call: {argv:?}"),
            };
            Ok(BackendSuccess {
                stdout: text,
                stderr: String::new(),
            })
        }
    }

    fn run(late: bool, move_head: bool) -> (WaitOutcome, usize, Vec<String>) {
        let runner = RegistrationRunner {
            polls: Cell::new(0),
            late,
            move_head,
            requested_heads: RefCell::new(Vec::new()),
        };
        let clock = TestClock(Cell::new(Instant::now()));
        let ctx = ProviderContext {
            provider: Provider::GitHub,
            host: "github.com".into(),
            source: DetectionSource::Flag,
            repo: Some("example/project".into()),
        };
        let global = GlobalFlags {
            format: None,
            remote: "origin".into(),
            provider: None,
            host: None,
            repo: ctx.repo.clone(),
            store_root: None,
            dry_run: false,
        };
        let args = PrWaitChecksArgs {
            id: "42".into(),
            timeout: Duration::from_millis(5),
            interval: Duration::from_millis(1),
            required_only: true,
            allow_no_checks: false,
        };
        (
            compute(&runner, &clock, &global, &ctx, &args).unwrap(),
            runner.polls.get(),
            runner.requested_heads.into_inner(),
        )
    }

    #[test]
    fn finished_optional_check_cannot_hide_unregistered_required_checks() {
        let (outcome, _, _) = run(false, false);
        let WaitOutcome::TimedOut(snapshot) = outcome else {
            panic!("missing required checks must not pass")
        };
        assert_eq!(
            snapshot
                .pending
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["test", "coverage"]
        );
    }

    #[test]
    fn waits_until_all_required_checks_register_on_the_current_head() {
        let (outcome, polls, _) = run(true, false);
        let WaitOutcome::Success(snapshot) = outcome else {
            panic!("registered successful checks must pass")
        };
        assert_eq!(polls, 3);
        assert_eq!(snapshot.required_count, 2);
    }

    #[test]
    fn head_changed_during_snapshot_cannot_pass_old_checks() {
        let (outcome, polls, requested) = run(true, true);
        // Later snapshots may pass, but the first successful old-head snapshot
        // must be discarded when the provider has already advanced the head.
        let WaitOutcome::Success(snapshot) = outcome else {
            panic!("new head eventually passes")
        };
        assert!(snapshot.duration_ms.unwrap() > 0);
        assert_eq!(polls, 2);
        assert_eq!(
            requested,
            [
                "repos/example/project/commits/head/check-runs?per_page=100",
                "repos/example/project/commits/new-head/check-runs?per_page=100",
            ]
        );
    }
}

mod configured_gate {
    use forge_cli::backend::{BackendCall, BackendRunner, BackendSuccess};
    use forge_cli::cli::{GlobalFlags, PrWaitChecksArgs};
    use forge_cli::error::ForgeError;
    use forge_cli::ops::pr_wait_checks::{Clock, WaitOutcome, compute};
    use forge_cli::ops::required_check_gate::{CheckPresence, ensure_required_checks_green};
    use forge_cli::provider::{DetectionSource, Provider, ProviderContext};
    use pretty_assertions::assert_eq;
    use std::time::{Duration, Instant};

    struct Fixture {
        pr_url: &'static str,
        protection: serde_json::Value,
        rules: serde_json::Value,
        runs: serde_json::Value,
        statuses: serde_json::Value,
        refuse_rules: bool,
        rules_error: Option<&'static str>,
        graphql_errors: bool,
    }
    impl Default for Fixture {
        fn default() -> Self {
            Self {
                pr_url: "https://github.com/example/project/pull/42",
                protection: serde_json::json!({"requiredStatusChecks":[{"context":"test","app":null},{"context":"coverage","app":null}]}),
                rules: serde_json::json!([]),
                runs: serde_json::json!({"total_count":1,"check_runs":[{"name":"test","status":"completed","conclusion":"success","app":{"id":1}}]}),
                statuses: serde_json::json!({"total_count":0,"statuses":[]}),
                refuse_rules: false,
                rules_error: None,
                graphql_errors: false,
            }
        }
    }
    impl BackendRunner for Fixture {
        fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
            let argv: Vec<_> = call.argv.iter().map(|s| s.to_string_lossy()).collect();
            let body = match (argv[0].as_ref(), argv[1].as_ref()) {
                ("pr", "view") => {
                    serde_json::json!({"headRefOid":"head","baseRefName":"main","url":self.pr_url})
                }
                ("api", "graphql") => {
                    let mut value = serde_json::json!({"data":{"repository":{"ref":{"branchProtectionRule":self.protection}}}});
                    if self.graphql_errors {
                        value["errors"] = serde_json::json!([{"message":"partial response"}]);
                    }
                    value
                }
                ("api", e) if e.contains("/rules/branches/") => {
                    if let Some(message) = self.rules_error {
                        return Err(ForgeError::backend_error(
                            "test",
                            "provider command failed",
                            Some(message.into()),
                        ));
                    }
                    if self.refuse_rules {
                        return Err(ForgeError::validation(
                            "test",
                            "permission_denied",
                            "rules unavailable",
                            None,
                        ));
                    }
                    self.rules.clone()
                }
                ("api", "repos/example/project/commits/head/check-runs?per_page=100") => {
                    self.runs.clone()
                }
                ("api", "repos/example/project/commits/head/status?per_page=100") => {
                    self.statuses.clone()
                }
                _ => panic!("gate must read the exact current head: {argv:?}"),
            };
            Ok(BackendSuccess {
                stdout: body.to_string(),
                stderr: String::new(),
            })
        }
    }
    fn context() -> (GlobalFlags, ProviderContext) {
        let ctx = ProviderContext {
            provider: Provider::GitHub,
            host: "github.com".into(),
            source: DetectionSource::Flag,
            repo: Some("example/project".into()),
        };
        let global = GlobalFlags {
            format: None,
            remote: "origin".into(),
            provider: None,
            host: None,
            repo: ctx.repo.clone(),
            store_root: None,
            dry_run: false,
        };
        (global, ctx)
    }
    fn gate(f: &Fixture) -> Result<forge_cli::ops::pr_checks::PrChecksPayload, ForgeError> {
        let (global, ctx) = context();
        ensure_required_checks_green(f, &global, &ctx, "42", CheckPresence::Required)
    }
    #[test]
    fn partial_required_registration_blocks_merge() {
        assert_eq!(
            gate(&Fixture::default()).unwrap_err().kind(),
            "checks_pending"
        );
    }
    #[test]
    fn ruleset_context_is_pending_before_registration() {
        let f = Fixture {
            protection: serde_json::Value::Null,
            rules: serde_json::json!([{"type":"required_status_checks","parameters":{"required_status_checks":[{"context":"coverage","integration_id":null}]}}]),
            ..Fixture::default()
        };
        assert_eq!(gate(&f).unwrap_err().kind(), "checks_pending");
    }

    #[test]
    fn free_plan_rules_403_falls_back_to_graphql_branch_protection() {
        const PLAN_LIMITATION: &str = "gh: Upgrade to GitHub Pro or make this repository public to enable this feature. (HTTP 403)";
        let f = Fixture {
            protection: serde_json::json!({"requiredStatusChecks":[{"context":"test","app":null}]}),
            rules_error: Some(PLAN_LIMITATION),
            ..Fixture::default()
        };
        assert!(gate(&f).is_ok());

        let missing_graphql_requirement = Fixture {
            rules_error: Some(PLAN_LIMITATION),
            ..Fixture::default()
        };
        assert_eq!(
            gate(&missing_graphql_requirement).unwrap_err().kind(),
            "checks_pending"
        );
    }

    #[test]
    fn unrelated_rules_403_stays_fail_closed() {
        let f = Fixture {
            rules_error: Some("gh: Resource not accessible by integration (HTTP 403)"),
            ..Fixture::default()
        };
        assert_eq!(gate(&f).unwrap_err().kind(), "backend_error");
    }
    #[test]
    fn required_workflow_rule_cannot_pass_with_only_optional_checks() {
        let f = Fixture {
            protection: serde_json::Value::Null,
            rules: serde_json::json!([{"type":"workflows","parameters":{"workflows":[{"path":".github/workflows/required.yml","repository_id":1}]}}]),
            ..Fixture::default()
        };
        assert_eq!(gate(&f).unwrap_err().kind(), "checks_snapshot_incomplete");
    }

    #[test]
    fn wrong_app_cannot_satisfy_a_required_context() {
        let f = Fixture {
            protection: serde_json::json!({"requiredStatusChecks":[{"context":"test","app":{"databaseId":2}}]}),
            ..Fixture::default()
        };
        assert_eq!(gate(&f).unwrap_err().kind(), "checks_pending");
    }
    fn app_bound_status_fixture(state: &str, app: serde_json::Value) -> Fixture {
        Fixture {
            protection: serde_json::json!({"requiredStatusChecks":[{"context":"test","app":{"databaseId":2}}]}),
            runs: serde_json::json!({"total_count":1,"check_runs":[{"name":"test","status":"completed","conclusion":"success","app":app}]}),
            statuses: serde_json::json!({"total_count":1,"statuses":[{"context":"test","state":state}]}),
            ..Fixture::default()
        }
    }

    fn wait(f: &Fixture) -> WaitOutcome {
        let (global, ctx) = context();
        let args = PrWaitChecksArgs {
            id: "42".into(),
            timeout: Duration::ZERO,
            interval: Duration::ZERO,
            required_only: true,
            allow_no_checks: false,
        };
        compute(f, &FixedClock, &global, &ctx, &args).unwrap()
    }

    #[test]
    fn failed_same_name_status_blocks_app_bound_wait_and_merge() {
        for state in ["failure", "error"] {
            let f = app_bound_status_fixture(state, serde_json::json!({"id":2}));
            assert!(matches!(wait(&f), WaitOutcome::Failed(_)), "{state}");
            assert_eq!(gate(&f).unwrap_err().kind(), "checks_failed");
        }
    }

    #[test]
    fn pending_same_name_status_blocks_app_bound_wait_and_merge() {
        let f = app_bound_status_fixture("pending", serde_json::json!({"id":2}));
        assert!(matches!(wait(&f), WaitOutcome::TimedOut(_)));
        assert_eq!(gate(&f).unwrap_err().kind(), "checks_pending");
    }

    #[test]
    fn successful_same_name_check_and_status_both_count_as_required() {
        let f = app_bound_status_fixture("success", serde_json::json!({"id":2}));
        let WaitOutcome::Success(payload) = wait(&f) else {
            panic!("both successful rows must allow the wait to finish");
        };
        assert_eq!(payload.required_count, 2);
        assert!(payload.checks.iter().all(|check| check.required));
        assert_eq!(gate(&f).unwrap().required_count, 2);
    }

    #[test]
    fn successful_status_cannot_replace_a_check_from_the_required_app() {
        for app in [serde_json::json!({"id":1}), serde_json::Value::Null] {
            let f = app_bound_status_fixture("success", app);
            assert!(matches!(wait(&f), WaitOutcome::TimedOut(_)));
            assert_eq!(gate(&f).unwrap_err().kind(), "checks_pending");
        }
    }

    #[test]
    fn commit_status_can_satisfy_an_unbound_required_context() {
        let f = Fixture {
            statuses: serde_json::json!({"total_count":1,"statuses":[{"context":"coverage","state":"success"}]}),
            ..Fixture::default()
        };
        assert_eq!(gate(&f).unwrap().required_count, 2);
    }
    #[test]
    fn failed_required_context_blocks_merge() {
        let f = Fixture {
            statuses: serde_json::json!({"total_count":1,"statuses":[{"context":"coverage","state":"failure"}]}),
            ..Fixture::default()
        };
        assert_eq!(gate(&f).unwrap_err().kind(), "checks_failed");
    }
    #[test]
    fn unreadable_rules_cannot_be_replaced_with_registered_rows() {
        let f = Fixture {
            refuse_rules: true,
            ..Fixture::default()
        };
        assert_eq!(gate(&f).unwrap_err().kind(), "permission_denied");
    }
    #[test]
    fn partial_graphql_response_is_refused() {
        let f = Fixture {
            graphql_errors: true,
            ..Fixture::default()
        };
        assert_eq!(gate(&f).unwrap_err().kind(), "checks_snapshot_incomplete");
    }
    #[test]
    fn truncated_check_runs_cannot_pass() {
        let f = Fixture {
            protection: serde_json::json!({"requiredStatusChecks":[{"context":"test","app":null}]}),
            runs: serde_json::json!({"total_count":2,"check_runs":[{"name":"test","status":"completed","conclusion":"success"}]}),
            ..Fixture::default()
        };
        assert_eq!(gate(&f).unwrap_err().kind(), "checks_pending");
    }
    #[test]
    fn missing_configuration_field_is_refused() {
        let f = Fixture {
            protection: serde_json::json!({}),
            ..Fixture::default()
        };
        assert_eq!(gate(&f).unwrap_err().kind(), "checks_snapshot_incomplete");
    }
    #[test]
    fn pr_url_must_match_the_selected_provider_authority() {
        for (url, host) in [
            (
                "https://github.example.com/example/project/pull/42",
                "github.com",
            ),
            (
                "https://github.com/example/project/pull/42",
                "github.example.com",
            ),
            (
                "https://github.example.com:8443/example/project/pull/42",
                "github.example.com",
            ),
            ("file:///example/project/pull/42", "github.com"),
        ] {
            let f = Fixture {
                pr_url: url,
                statuses: serde_json::json!({"total_count":1,"statuses":[{"context":"coverage","state":"success"}]}),
                ..Fixture::default()
            };
            let (global, mut ctx) = context();
            ctx.host = host.into();
            ctx.repo = None;
            assert_eq!(
                ensure_required_checks_green(&f, &global, &ctx, "42", CheckPresence::Required)
                    .unwrap_err()
                    .kind(),
                "checks_snapshot_incomplete",
                "{url} vs {host}"
            );
        }
    }
    #[test]
    fn canonical_transport_alias_and_matching_port_are_accepted() {
        for (url, host) in [
            (
                "https://github.com:443/example/project/pull/42",
                "ssh.github.com",
            ),
            (
                "https://github.example.com:8443/example/project/pull/42",
                "github.example.com:8443",
            ),
        ] {
            let f = Fixture {
                pr_url: url,
                statuses: serde_json::json!({"total_count":1,"statuses":[{"context":"coverage","state":"success"}]}),
                ..Fixture::default()
            };
            let (global, mut ctx) = context();
            ctx.host = host.into();
            assert_eq!(
                ensure_required_checks_green(&f, &global, &ctx, "42", CheckPresence::Required)
                    .unwrap()
                    .state,
                "success"
            );
        }
    }

    struct FixedClock;
    impl Clock for FixedClock {
        fn now(&self) -> Instant {
            static NOW: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            *NOW.get_or_init(Instant::now)
        }
        fn sleep(&self, _: Duration) {
            panic!("zero-budget fixture should never sleep")
        }
    }
    #[test]
    fn allow_no_checks_does_not_skip_missing_configured_checks() {
        let (global, ctx) = context();
        let args = PrWaitChecksArgs {
            id: "42".into(),
            timeout: Duration::ZERO,
            interval: Duration::ZERO,
            required_only: true,
            allow_no_checks: true,
        };
        assert!(matches!(
            compute(&Fixture::default(), &FixedClock, &global, &ctx, &args).unwrap(),
            WaitOutcome::TimedOut(_)
        ));
    }
}
