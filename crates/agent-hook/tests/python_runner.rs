mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use support::Fixture;

const POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "runtime-kit"
version = "test"

[[rules]]
id = "runtime.second"
products = ["codex", "claude"]
events = ["PreToolUse"]
matcher = "Bash"
priority = 20
mode = "enforce"
failure_posture = "open"
override_class = "downgrade-only"
capability = { id = "runtime-kit.handler.v1", handler_id = "block-direct-python" }

[[rules]]
id = "runtime.first"
products = ["codex", "claude"]
events = ["PreToolUse"]
matcher = "Bash"
priority = 10
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "runtime-kit.handler.v1", handler_id = "checkout-lease-guard" }

[[rules]]
id = "runtime.coordination"
products = ["codex", "claude"]
events = ["PreToolUse", "PostToolUse", "PostToolUseFailure"]
matcher = "Bash"
priority = 1
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.coordination.v1", reason_code = "coordination" }
"#;

// Synthetic peer for the runtime-kit's framed runner contract.
const RUNNER: &str = r#"#!/usr/bin/env python3
from contextlib import redirect_stdout
import io
from pathlib import Path
import runpy
import sys

source = sys.stdin.buffer
sink = sys.stdout.buffer
payload = source.read(int(source.readline()))
for request in source:
    limit, name = request.decode().split()
    sys.stdin = io.StringIO(payload.decode())
    output = io.StringIO()
    status = 0
    with redirect_stdout(output):
        try:
            runpy.run_path(str(Path(__file__).with_name(name)), run_name="__main__")
        except SystemExit as error:
            status = error.code
    result = output.getvalue().encode()[:int(limit) + 1]
    sink.write(f"{status} {len(result)}\n".encode() + result)
    sink.flush()
"#;

const RECORD: &str = r#"#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

payload = sys.stdin.read()
with open(os.environ["RUNNER_LOG"], "a") as log:
    print(json.dumps([Path(__file__).name, os.getpid(), payload]), file=log)
"#;

const FIRST: &str = "checkout-lease-guard.py";
const SECOND: &str = "block-direct-python.py";
const COORDINATION: &str = "session-coordination-guard.py";
const CONTEXT: &str = r#"print('{"additionalContext":"fixture handler context"}')"#;
const COORDINATION_RESULT: &str = r#"print('{"schema_version":"runtime-kit.session-coordination-result.v1","status":"clean","message":"fixture coordination completed"}')"#;
const BLOCK: &str = r#"print('{"decision":"block","reason":"fixture policy block"}')"#;
const MANAGED: [(&str, &str); 2] = [
    ("AGENT_SESSION_ID", "fixture"),
    ("AGENT_SESSION_RUNTIME_ID", "fixture-runtime"),
];

fn install(hooks: &Path, name: &str, contents: &str) {
    let script = hooks.join(name);
    fs::write(&script, contents).expect("script");
    fs::set_permissions(script, fs::Permissions::from_mode(0o700)).expect("script mode");
}

fn fixture(product: &str, batched: bool, first: &str, second: &str) -> (Fixture, PathBuf) {
    let policy = if batched {
        POLICY.replace(
            "version = \"test\"",
            "version = \"test\"\npython_runner = true",
        )
    } else {
        POLICY.to_string()
    };
    let fixture = Fixture::new(&policy);
    let hooks = fixture.home.join(format!(".{product}/hooks"));
    fs::create_dir_all(&hooks).expect("hook directory");
    install(&hooks, "event-runner.py", RUNNER);
    install(&hooks, FIRST, &format!("{RECORD}\n{first}"));
    install(&hooks, SECOND, &format!("{RECORD}\n{second}"));
    install(
        &hooks,
        COORDINATION,
        &format!("{RECORD}\n{COORDINATION_RESULT}"),
    );

    let session = fixture.session_state.join("sessions/fixture");
    fs::create_dir_all(&session).expect("session directory");
    let record = session.join("session.json");
    fs::write(
        &record,
        r#"{"schema_version":"agent-session.session.v1","id":"fixture","coordination_mode":"off","runtime":{"launch_id":"fixture-runtime"}}"#,
    )
    .expect("session record");
    Fixture::set_private(&record);
    let log = fixture.root.join("requests.jsonl");
    (fixture, log)
}

fn payload(event: &str) -> String {
    json!({
        "hook_event_name": event,
        "session_id": "fixture-provider",
        "tool_use_id": "fixture-tool",
        "tool_name": "Bash",
        "tool_input": {"command":"true"},
    })
    .to_string()
}

fn dispatch(
    fixture: &Fixture,
    log: &Path,
    product: &str,
    payload: &str,
    format: &str,
    extra_env: &[(&str, &str)],
) -> (i32, String) {
    let mut env = MANAGED.to_vec();
    env.push(("RUNNER_LOG", log.to_str().expect("log path")));
    env.extend_from_slice(extra_env);
    let result = fixture.run_with_env(
        &["dispatch", "--product", product, "--format", format],
        Some(payload),
        &env,
    );
    (result.code, result.stdout_text())
}

fn requests(log: &Path, expected_names: &[&str], payload: &str) -> Vec<u64> {
    let received = fs::read_to_string(log).expect("handler log");
    let mut names = Vec::new();
    let mut pids = Vec::new();
    for line in received.lines() {
        let entry: (String, u64, String) = serde_json::from_str(line).expect("handler entry");
        names.push(entry.0);
        pids.push(entry.1);
        assert_eq!(entry.2, payload);
    }
    assert_eq!(names, expected_names);
    pids
}

#[test]
fn python_runner_preserves_provider_decisions_and_dispatch_order() {
    for product in ["codex", "claude"] {
        for (event, first, names) in [
            ("PreToolUse", CONTEXT, &[FIRST, SECOND, COORDINATION][..]),
            ("PreToolUse", BLOCK, &[FIRST, SECOND][..]),
            ("PostToolUse", "", &[COORDINATION][..]),
            ("PostToolUseFailure", "", &[COORDINATION][..]),
        ] {
            let payload = payload(event);
            let (baseline, baseline_log) = fixture(product, false, first, "");
            let (batched, batched_log) = fixture(product, true, first, "");
            let expected = dispatch(&baseline, &baseline_log, product, &payload, "provider", &[]);
            let actual = dispatch(&batched, &batched_log, product, &payload, "provider", &[]);
            assert_eq!(actual, expected, "{product} {event}");
            let sequential = requests(&baseline_log, names, &payload);
            assert!(sequential.windows(2).all(|pair| pair[0] != pair[1]));
            let shared = requests(&batched_log, names, &payload);
            assert!(shared.windows(2).all(|pair| pair[0] == pair[1]));
        }
    }
}

#[test]
fn python_runner_scopes_failed_requests_and_restarts_only_after_runner_loss() {
    let payload = payload("PreToolUse");
    for (first, second, names, shares_runner) in [
        (
            "",
            "raise SystemExit(9)",
            &[FIRST, SECOND, COORDINATION][..],
            true,
        ),
        ("os._exit(9)", "", &[FIRST, SECOND][..], false),
    ] {
        let (baseline, baseline_log) = fixture("codex", false, first, second);
        let (batched, batched_log) = fixture("codex", true, first, second);
        let expected = dispatch(&baseline, &baseline_log, "codex", &payload, "json", &[]);
        let actual = dispatch(&batched, &batched_log, "codex", &payload, "json", &[]);
        let expected: Value = serde_json::from_str(&expected.1).unwrap();
        let actual: Value = serde_json::from_str(&actual.1).unwrap();
        assert_eq!(actual["data"]["reasons"], expected["data"]["reasons"]);
        assert_eq!(actual["data"]["action"], expected["data"]["action"]);

        let failed_rule = if shares_runner {
            "runtime.second:capability-failure-open"
        } else {
            "runtime.first:capability-failure-closed"
        };
        assert!(
            actual["data"]["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason["code"] == failed_rule)
        );
        let pids = requests(&batched_log, names, &payload);
        assert!(
            pids.windows(2)
                .all(|pair| (pair[0] == pair[1]) == shares_runner)
        );
    }
}

#[test]
fn python_runner_preserves_handler_and_dispatch_output_budgets() {
    let payload = payload("PreToolUse");
    let oversized = "print('x' * (256 * 1024))";
    for second in ["", oversized] {
        let (baseline, baseline_log) = fixture("codex", false, oversized, second);
        let (batched, batched_log) = fixture("codex", true, oversized, second);
        let expected = dispatch(&baseline, &baseline_log, "codex", &payload, "json", &[]);
        let actual = dispatch(&batched, &batched_log, "codex", &payload, "json", &[]);
        assert_eq!(actual.0, expected.0);
        let expected: Value = serde_json::from_str(&expected.1).unwrap();
        let actual: Value = serde_json::from_str(&actual.1).unwrap();
        if second.is_empty() {
            assert_eq!(actual["data"]["reasons"], expected["data"]["reasons"]);
            assert_eq!(actual["data"]["context"], expected["data"]["context"]);
            assert!(
                actual["data"]["context"]
                    .as_str()
                    .unwrap()
                    .contains("handler-output-too-large")
            );
        } else {
            assert_eq!(actual["error"], expected["error"]);
            assert_eq!(actual["error"]["code"], "dispatch-output-budget-exceeded");
        }
        let pids = requests(&batched_log, &[FIRST, SECOND], &payload);
        assert_eq!(pids[0], pids[1]);
    }
}

#[test]
fn python_runner_timeout_terminates_descendants_and_continues_with_next_request() {
    use std::fs::File;
    use std::thread;
    use std::time::{Duration, Instant};

    let stalled_handler = r#"
import subprocess
subprocess.run([sys.executable, "-c", """
import fcntl
from pathlib import Path
import time

with open("descendant.lock", "w") as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    Path("descendant-ready").touch()
    time.sleep(30)
"""])
"#;
    let payload = payload("PreToolUse");
    let (fixture, log) = fixture("codex", true, stalled_handler, "");
    let actual = dispatch(
        &fixture,
        &log,
        "codex",
        &payload,
        "json",
        &[("AGENT_HOOK_HANDLER_TIMEOUT_MS", "2500")],
    );
    let decision: Value = serde_json::from_str(&actual.1).unwrap();
    assert_eq!(decision["data"]["action"], "block");
    assert_eq!(
        decision["data"]["reasons"][0]["code"],
        "runtime.first:capability-timeout-closed"
    );
    let pids = requests(&log, &[FIRST, SECOND], &payload);
    assert_ne!(pids[0], pids[1]);
    assert!(fixture.root.join("descendant-ready").is_file());

    let descendant_lock = File::open(fixture.root.join("descendant.lock")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while let Err(error) = descendant_lock.try_lock() {
        assert!(Instant::now() < deadline, "descendant survived: {error}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn opted_in_dispatch_rejects_untrusted_runner_without_running_handlers() {
    let payload = payload("PreToolUse");
    let (fixture, log) = fixture("codex", true, "", "");
    let runner = fixture.home.join(".codex/hooks/event-runner.py");
    let target = runner.with_file_name("synthetic-runner.py");
    fs::rename(&runner, &target).unwrap();
    std::os::unix::fs::symlink(&target, &runner).unwrap();

    let actual = dispatch(&fixture, &log, "codex", &payload, "json", &[]);
    let decision: Value = serde_json::from_str(&actual.1).unwrap();
    assert_eq!(decision["data"]["action"], "block");
    assert!(
        decision["data"]["context"]
            .as_str()
            .unwrap()
            .contains("handler-untrusted")
    );
    assert!(!log.exists());
}
