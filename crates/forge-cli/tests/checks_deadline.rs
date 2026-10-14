//! Whole-operation budget and additive scope regressions use offline backends.
#[allow(dead_code)]
#[path = "integration/support.rs"]
mod support;

use pretty_assertions::assert_eq;
use std::time::{Duration, Instant};
use support::{StubEnv, parse_envelope, run_forge_cli};

fn backend(delay: &str) -> StubEnv {
    let stub = StubEnv::new().gh_stub(&format!(
        r#"#!/bin/sh
set -eu
case "$1 $2" in
  "pr view")
    echo '{{"headRefOid":"test-head","baseRefName":"main","url":"https://github.com/example/project/pull/1"}}';;
  "pr checks")
    case " $* " in
      *" --required "*) echo '[]';;
      *) echo '[{{"name":"build","bucket":"pass","state":"COMPLETED"}}]';;
    esac;;
  "api graphql")
    echo '{{"data":{{"repository":{{"ref":{{"branchProtectionRule":null}}}}}}}}';;
  "api repos/example/project/rules/branches/main?per_page=100")
    {delay}
    echo '[]';;
  "api repos/example/project/commits/test-head/check-runs?per_page=100")
    echo '{{"total_count":1,"check_runs":[{{"name":"build","status":"completed","conclusion":"success"}}]}}';;
  "api repos/example/project/commits/test-head/status?per_page=100")
    echo '{{"total_count":0,"statuses":[]}}';;
  *) echo "unexpected fixture request: $*" >&2; exit 99;;
esac
"#
    ));
    warm_binary(&stub);
    stub
}

fn warm_binary(stub: &StubEnv) {
    // Keep first-launch artifact inspection outside the measured wait operation.
    let out = run_forge_cli(stub, &["--version"]);
    assert_eq!(out.code, 0);
}

#[test]
fn wait_budget_includes_delayed_backend_and_rejects_late_success() {
    let stub = backend("sleep 3");
    let started = Instant::now();
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "example/project",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--timeout",
            "1s",
            "--interval",
            "1s",
        ],
    );
    let elapsed = started.elapsed();
    assert_eq!(out.code, 69, "elapsed={elapsed:?}; stderr={}", out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "checks_timeout"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "elapsed={elapsed:?}; stdout={}",
        out.stdout
    );
}

#[test]
fn checks_and_wait_expose_zero_required_visible_scope() {
    for command in ["checks", "wait-checks"] {
        let stub = backend(":");
        let mut args = vec![
            "--provider",
            "github",
            "--repo",
            "example/project",
            "--format",
            "json",
            "pr",
            command,
            "1",
        ];
        if command == "wait-checks" {
            args.extend(["--timeout", "5s", "--interval", "100ms"]);
        }
        let out = run_forge_cli(&stub, &args);
        assert_eq!(out.code, 0, "{command}: {}", out.stderr);
        let data = &parse_envelope(&out.stdout)["data"];
        assert_eq!(data["configured_required_count"], 0, "{command}");
        assert_eq!(data["gate_scope"], "visible_fallback", "{command}");
        assert_eq!(data["gate_count"], 1, "{command}");
        assert_eq!(data["visible_state"], "success", "{command}");
        assert_eq!(data["checks"][0]["required"], false, "{command}");
    }
}

fn rewrite_backend(stub: &StubEnv, rewrite: impl FnOnce(String) -> String) {
    let path = stub.tempdir.path().join("gh");
    let body = std::fs::read_to_string(&path).unwrap();
    std::fs::write(path, rewrite(body)).unwrap();
}

fn wait(stub: &StubEnv) -> support::CmdOutput {
    run_forge_cli(
        stub,
        &[
            "--provider",
            "github",
            "--repo",
            "example/project",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--timeout",
            "1s",
            "--interval",
            "1s",
        ],
    )
}

#[test]
fn rate_limit_probe_and_wait_share_the_command_budget() {
    for probe in [
        "sleep 3; echo '{\"resources\":{\"graphql\":{\"remaining\":100}}}'",
        "echo '{\"resources\":{\"graphql\":{\"remaining\":0}}}'",
    ] {
        let stub = backend(":").env("FORGE_CLI_RATE_LIMIT_GATE", "on");
        rewrite_backend(&stub, |body| {
            body.replace(
                "  \"api graphql\")",
                &format!("  \"api rate_limit\") {probe};;\n  \"api graphql\")"),
            )
        });
        let start = Instant::now();
        let out = wait(&stub);
        assert_eq!(out.code, 69, "{} {}", out.stdout, out.stderr);
        assert_eq!(
            parse_envelope(&out.stdout)["error"]["code"],
            "checks_timeout"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}

#[test]
fn deadline_returns_last_complete_pending_snapshot() {
    let stub = backend("if [ -f \"$0.second\" ]; then sleep 3; fi");
    rewrite_backend(&stub, |body| {
        body.replace(
        "    echo '{\"total_count\":1,\"check_runs\":[{\"name\":\"build\",\"status\":\"completed\",\"conclusion\":\"success\"}]}'",
        "    if [ ! -f \"$0.first\" ]; then\n      : > \"$0.first\"\n      echo '{\"total_count\":1,\"check_runs\":[{\"name\":\"build\",\"status\":\"in_progress\",\"conclusion\":null}]}'\n    else\n      : > \"$0.second\"\n      echo '{\"total_count\":1,\"check_runs\":[{\"name\":\"build\",\"status\":\"completed\",\"conclusion\":\"success\"}]}'\n    fi",
    )
    });
    let start = Instant::now();
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "example/project",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--timeout",
            "1s",
            "--interval",
            "10ms",
        ],
    );
    assert_eq!(out.code, 69, "{} {}", out.stdout, out.stderr);
    let data = &parse_envelope(&out.stdout)["data"];
    assert_eq!(data["state"], "pending");
    assert_eq!(data["gate_count"], 1);
    assert_eq!(data["checks"][0]["name"], "build");
    assert_eq!(data["checks"][0]["state"], "pending");
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn deadline_covers_backend_descendants_holding_capture_pipes() {
    let stub = backend("sleep 3 & exit 0");
    let start = Instant::now();
    let out = wait(&stub);
    assert_eq!(out.code, 69, "{} {}", out.stdout, out.stderr);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "elapsed={:?}",
        start.elapsed()
    );
}

#[test]
fn gitlab_backend_uses_the_same_wait_budget() {
    let stub = StubEnv::new().glab_stub("#!/bin/sh\nsleep 3\necho '{}'\n");
    warm_binary(&stub);
    let start = Instant::now();
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "gitlab",
            "--repo",
            "example/project",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--timeout",
            "1s",
            "--interval",
            "1s",
        ],
    );
    assert_eq!(out.code, 69, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "checks_timeout"
    );
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn scopes_distinguish_required_gate_from_visible_failure() {
    for command in ["checks", "wait-checks"] {
        for required_only in ["true", "false"] {
            let stub = backend(":");
            rewrite_backend(&stub, |body| {
                body
                .replace("*\" --required \"*) echo '[]'", "*\" --required \"*) echo '[{\"name\":\"build\",\"bucket\":\"pass\",\"state\":\"COMPLETED\"}]'")
                .replace("[{\"name\":\"build\",\"bucket\":\"pass\",\"state\":\"COMPLETED\"}]';;\n    esac", "[{\"name\":\"build\",\"bucket\":\"pass\",\"state\":\"COMPLETED\"},{\"name\":\"lint\",\"bucket\":\"fail\",\"state\":\"COMPLETED\"}]';;\n    esac")
                .replace("\"branchProtectionRule\":null", "\"branchProtectionRule\":{\"requiredStatusChecks\":[{\"context\":\"build\",\"app\":null}]}")
                .replace("\"conclusion\":\"success\"}]", "\"conclusion\":\"success\"},{\"name\":\"lint\",\"status\":\"completed\",\"conclusion\":\"failure\"}]")
            });
            let mut args = vec![
                "--provider",
                "github",
                "--repo",
                "example/project",
                "--format",
                "json",
                "pr",
                command,
                "1",
                "--required-only",
                required_only,
            ];
            if command == "wait-checks" {
                args.extend(["--timeout", "5s"]);
            }
            let out = run_forge_cli(&stub, &args);
            let envelope = parse_envelope(&out.stdout);
            let data = &envelope["data"];
            assert_eq!(
                data["configured_required_count"], 1,
                "{command}: {}",
                out.stdout
            );
            assert_eq!(data["visible_count"], 2);
            assert_eq!(data["visible_state"], "failure");
            if required_only == "true" {
                assert_eq!(out.code, 0);
                assert_eq!(data["gate_scope"], "required");
                assert_eq!(data["gate_count"], 1);
                assert_eq!(data["gate_state"], "success");
            } else {
                assert_eq!(data["gate_scope"], "visible");
                assert_eq!(data["gate_count"], 2);
                assert_eq!(data["gate_state"], "failure");
            }
        }
    }
}

#[test]
fn expired_budget_does_not_start_a_backend_or_admit_an_opt_out() {
    let stub = backend(":");
    rewrite_backend(&stub, |body| {
        body.replace("set -eu", "set -eu\n: > \"$0.called\"")
    });
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "example/project",
            "--format",
            "json",
            "pr",
            "wait-checks",
            "1",
            "--timeout",
            "0s",
            "--allow-no-checks",
        ],
    );
    assert_eq!(out.code, 69, "{} {}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "checks_timeout"
    );
    assert!(!stub.tempdir.path().join("gh.called").exists());
}

#[test]
fn text_checks_and_wait_expose_the_effective_scope() {
    for command in ["checks", "wait-checks"] {
        let stub = backend(":");
        let out = run_forge_cli(
            &stub,
            &[
                "--provider",
                "github",
                "--repo",
                "example/project",
                "pr",
                command,
                "1",
            ],
        );
        assert_eq!(out.code, 0, "{}", out.stderr);
        assert!(
            out.stdout.contains("gate_scope=visible_fallback"),
            "{}",
            out.stdout
        );
        assert!(
            out.stdout.contains("configured_required_count=0"),
            "{}",
            out.stdout
        );
        assert!(out.stdout.contains("gate_count=1"), "{}", out.stdout);
        assert!(
            out.stdout.contains("visible_state=success"),
            "{}",
            out.stdout
        );
    }
}

#[test]
fn a_successful_wait_never_reports_an_expired_duration() {
    use forge_cli::backend::{BackendCall, BackendRunner, BackendSuccess};
    use forge_cli::cli::{GlobalFlags, PrWaitChecksArgs};
    use forge_cli::error::ForgeError;
    use forge_cli::ops::pr_wait_checks::{Clock, WaitOutcome, compute};
    use forge_cli::provider::{DetectionSource, Provider, ProviderContext};
    use std::cell::Cell;

    struct ReadyBackend;
    impl BackendRunner for ReadyBackend {
        fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
            let stdout = if call.argv[0] == "mr" {
                r#"{"iid":42,"sha":"head","target_branch":"main","head_pipeline":{"id":99,"status":"success"}}"#
            } else {
                r#"[{"name":"build","status":"success","allow_failure":false}]"#
            };
            Ok(BackendSuccess {
                stdout: stdout.into(),
                stderr: String::new(),
            })
        }
    }
    struct TickingClock {
        start: Instant,
        elapsed: Cell<Duration>,
    }
    impl Clock for TickingClock {
        fn now(&self) -> Instant {
            let elapsed = self.elapsed.get();
            self.elapsed.set(elapsed + Duration::from_millis(10));
            self.start + elapsed
        }
        fn sleep(&self, duration: Duration) {
            self.elapsed.set(self.elapsed.get() + duration);
        }
    }
    let global = GlobalFlags {
        format: None,
        remote: "origin".into(),
        provider: None,
        host: None,
        repo: Some("example/project".into()),
        store_root: None,
        dry_run: false,
    };
    let ctx = ProviderContext {
        provider: Provider::GitLab,
        host: "gitlab.com".into(),
        source: DetectionSource::Flag,
        repo: global.repo.clone(),
    };
    let mut saw_success = false;
    let mut saw_timeout = false;
    for budget_ms in (10..=400).step_by(10) {
        let clock = TickingClock {
            start: Instant::now(),
            elapsed: Cell::new(Duration::ZERO),
        };
        let args = PrWaitChecksArgs {
            id: "42".into(),
            timeout: Duration::from_millis(budget_ms),
            interval: Duration::from_millis(1),
            required_only: true,
            allow_no_checks: false,
        };
        match compute(&ReadyBackend, &clock, &global, &ctx, &args).unwrap() {
            WaitOutcome::Success(snapshot) => {
                saw_success = true;
                assert!(
                    snapshot.duration_ms.unwrap() < budget_ms,
                    "success duration {:?} expired budget {budget_ms}",
                    snapshot.duration_ms
                );
            }
            WaitOutcome::TimedOut(_) => saw_timeout = true,
            _ => panic!("ready backend should succeed or exhaust its budget"),
        }
    }
    assert!(saw_success && saw_timeout);
}
