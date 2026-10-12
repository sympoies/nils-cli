use std::fs;
use std::os::unix::fs::PermissionsExt;

use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::common;

fn exec_fixture(argv: &[&str], upstream: &Value, upstream_exit: i32) -> (i32, Value, Value) {
    exec_fixture_mode(argv, upstream, upstream_exit, "minimal")
}

fn exec_fixture_mode(
    argv: &[&str],
    upstream: &Value,
    upstream_exit: i32,
    mode: &str,
) -> (i32, Value, Value) {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("cwd");
    let fake = cwd.path().join("peekaboo");
    let payload = cwd.path().join("payload.json");
    fs::write(&payload, upstream.to_string()).expect("payload");
    fs::write(
        &fake,
        "#!/bin/sh\ncat \"$FIXTURE_PAYLOAD\"\nexit \"$FIXTURE_EXIT\"\n",
    )
    .expect("fixture");
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod");
    let out_dir = cwd.path().join("journal");
    let options = harness
        .cmd_options(cwd.path())
        .with_env(
            "NILS_MACOS_AGENT_PEEKABOO_BIN",
            fake.to_str().expect("path"),
        )
        .with_env("FIXTURE_PAYLOAD", payload.to_str().expect("path"))
        .with_env("FIXTURE_EXIT", &upstream_exit.to_string());
    let mut args = vec![
        "--format",
        "json",
        "exec",
        "--evidence-mode",
        mode,
        "--out-dir",
        out_dir.to_str().expect("out"),
        "--expected",
        "Synthetic counter increments",
        "--",
    ];
    args.extend_from_slice(argv);
    let out = harness.run_with_options(cwd.path(), &args, options);
    let step = serde_json::from_str(
        fs::read_to_string(out_dir.join("steps.jsonl"))
            .expect("journal")
            .trim(),
    )
    .expect("step");
    (out.code, out.stdout_json()["result"].clone(), step)
}

#[test]
fn foreground_coordinate_refusal_keeps_specific_cause_and_focus_effects() {
    let upstream = json!({"success":false, "outcome":{
        "effect":"refused", "refusal_reason":"target_unavailable", "mutation_dispatched":false,
        "setup_effects":["focus_requested"], "retry_safe":true},
        "error":{"code":"VALIDATION_ERROR", "message":"Window rejected: minimized",
            "hint":"Select a capture-eligible window"}});
    let (code, result, step) = exec_fixture(
        &[
            "click",
            "--window-id",
            "101",
            "--at",
            "20,30",
            "--foreground",
            "--json",
        ],
        &upstream,
        0,
    );
    assert_eq!(code, 70, "a structured refusal at exit zero must fail");
    assert_eq!(result["upstream"]["json"], upstream);
    assert_eq!(result["coordinate_click"]["dispatch"], "not_dispatched");
    assert_eq!(result["coordinate_click"]["reason"], "target_unavailable");
    assert_eq!(
        result["coordinate_click"]["message"],
        "Window rejected: minimized"
    );
    assert!(
        result["coordinate_click"]["guidance"]
            .as_str()
            .expect("guidance")
            .contains("fresh")
    );
    assert_eq!(step["status"], "failed");
    assert_eq!(step["retries"], 0);
}

#[test]
fn uncertain_coordinate_delivery_is_never_a_refusal_or_replay_candidate() {
    let upstream = json!({"success":false,"outcome":{
        "state":"indeterminate", "effect":"unverifiable", "mutation_dispatched":true,
        "retry_safe":false},"error":{"code":"INTERACTION_FAILED",
            "message":"Same application different window occludes the target"}});
    let (code, result, step) = exec_fixture(
        &[
            "click",
            "--window-id",
            "101",
            "--snapshot",
            "fixture-snapshot",
            "--at",
            "20,30",
            "--json",
        ],
        &upstream,
        1,
    );
    assert_eq!(code, 70);
    assert_eq!(result["upstream"]["json"], upstream);
    assert_eq!(result["coordinate_click"]["dispatch"], "dispatched");
    assert!(
        result["coordinate_click"]["guidance"]
            .as_str()
            .expect("guidance")
            .contains("Do not replay")
    );
    assert_eq!(step["status"], "unknown");
    assert_eq!(step["replay_class"], "never");
    assert_eq!(step["retries"], 0);
}

#[test]
fn coordinate_failure_without_dispatch_evidence_stays_unknown_in_diagnostic() {
    let (code, result, step) = exec_fixture(
        &["click", "--at=20,30", "--foreground", "--json"],
        &json!({"success":false,"error":{"code":"WINDOW_NOT_FOUND","message":"No eligible target window"}}),
        1,
    );
    assert_eq!(code, 70);
    assert_eq!(result["coordinate_click"]["dispatch"], "unknown");
    assert_eq!(result["coordinate_click"]["reason"], "WINDOW_NOT_FOUND");
    assert_eq!(step["status"], "unknown");
    assert_eq!(step["replay_class"], "never");
    assert!(
        result["coordinate_click"]["guidance"]
            .as_str()
            .expect("guidance")
            .contains("Do not replay")
    );
}

#[test]
fn roi_ax_controls_inside_the_crop_are_explicitly_visibility_unverified() {
    // Bounds inside the crop cannot prove that an opaque overlay does not cover the control.
    let upstream = json!({"success":true,"data":{"semantic_scope":"exact_or_requested",
        "ui_elements":[{"id":"B1","label":"Synthetic underlying control","is_actionable":true,
            "bounds":{"x":20,"y":30,"width":10,"height":10}}], "element_count":1}});
    let (code, result, _) = exec_fixture(
        &[
            "see",
            "--window-id",
            "101",
            "--roi",
            "0,0,100,100",
            "--json",
        ],
        &upstream,
        0,
    );
    assert_eq!(code, 0);
    assert_eq!(
        result["upstream"]["json"]["data"]["ui_elements"][0]["adapter_visibility"],
        "unverified"
    );
    assert_eq!(result["observation_scope"]["ax_visibility"], "unverified");
    assert_eq!(result["observation_scope"]["roi_requested"], true);
    assert_eq!(
        result["upstream"]["json"]["data"]["ui_elements"][0]["label"],
        upstream["data"]["ui_elements"][0]["label"]
    );
}

#[test]
fn accepted_coordinate_click_keeps_receipt_and_dispatch_evidence() {
    let upstream = json!({"success":true,"outcome":{"effect":"confirmed","mutation_dispatched":true},
        "data":{"receipt":"synthetic-receipt"}});
    let (code, result, step) = exec_fixture(
        &["click", "--at", "20,30", "--foreground", "--json"],
        &upstream,
        0,
    );
    assert_eq!(code, 0);
    assert_eq!(result["upstream"]["json"], upstream);
    assert!(result.get("coordinate_click").is_none());
    assert_eq!(step["status"], "passed");
}

#[test]
fn mcp_observations_qualify_structured_ax_and_plain_text_without_altering_images() {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("cwd");
    let fake = cwd.path().join("peekaboo");
    fs::write(&fake, r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"id":1'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"structuredContent":{"ui_elements":[{"id":"B1","label":"Synthetic obscured control"}]},"content":[{"type":"text","text":"Visible controls: Synthetic obscured control"},{"type":"image","data":"synthetic-image","mimeType":"image/png"}]}}' ;;
    *'"id":2'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":null}' ;;
  esac
done
"#).expect("fixture");
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod");
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"see\",\"arguments\":{\"roi\":\"0,0,100,100\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"shutdown\"}\n"
    );
    let out_dir = cwd.path().join("journal");
    let out = harness.run_with_options(
        cwd.path(),
        &["mcp", "--out-dir", out_dir.to_str().expect("out")],
        harness
            .cmd_options(cwd.path())
            .with_env(
                "NILS_MACOS_AGENT_PEEKABOO_BIN",
                fake.to_str().expect("path"),
            )
            .with_stdin_str(input),
    );
    assert_eq!(out.code, 0, "{}", out.stderr_text());
    let response: Value = out
        .stdout_text()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("frame"))
        .find(|frame| frame["id"] == 1)
        .expect("observation");
    assert_eq!(
        response["result"]["structuredContent"]["ui_elements"][0]["adapter_visibility"],
        "unverified"
    );
    assert_eq!(
        response["result"]["_meta"]["macos-agent/observation_scope"]["ax_visibility"],
        "unverified"
    );
    assert_eq!(
        response["result"]["_meta"]["macos-agent/observation_scope"]["roi_requested"],
        true
    );
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("warning")
            .contains("occluded")
    );
    assert_eq!(response["result"]["content"][2]["data"], "synthetic-image");
}

#[test]
fn sensitive_coordinate_refusal_retains_only_protocol_reason() {
    let (code, result, _) = exec_fixture_mode(
        &["click", "--at", "20,30", "--foreground", "--json"],
        &json!({"success":false,"outcome":{"effect":"refused","refusal_reason":"target_unavailable","mutation_dispatched":false},
            "error":{"code":"VALIDATION_ERROR","message":"Synthetic private content"}}),
        1,
        "sensitive",
    );
    assert_eq!(code, 70);
    assert_eq!(result["coordinate_click"]["reason"], "target_unavailable");
    assert_eq!(result["coordinate_click"]["message"], "<suppressed>");
}

#[test]
fn coordinate_transport_interruption_cannot_prove_zero_dispatch() {
    for finish in ["sleep 3", "kill -TERM $$"] {
        let harness = common::MacosAgentHarness::new();
        let cwd = TempDir::new().expect("cwd");
        let fake = cwd.path().join("peekaboo");
        fs::write(&fake, format!("#!/bin/sh\nprintf '%s\\n' '{{\"success\":false,\"outcome\":{{\"effect\":\"refused\",\"mutation_dispatched\":false}}}}'\n{finish}\n")).expect("fixture");
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod");
        let out_dir = cwd.path().join("journal");
        let out = harness.run_with_options(
            cwd.path(),
            &[
                "--format",
                "json",
                "exec",
                "--out-dir",
                out_dir.to_str().expect("out"),
                "--timeout-seconds",
                "1",
                "--expected",
                "Counter increments",
                "--",
                "click",
                "--snapshot",
                "fixture-snapshot",
                "--at",
                "20,30",
                "--json",
            ],
            harness.cmd_options(cwd.path()).with_env(
                "NILS_MACOS_AGENT_PEEKABOO_BIN",
                fake.to_str().expect("path"),
            ),
        );
        assert_eq!(out.code, 70);
        let step: Value = serde_json::from_str(
            fs::read_to_string(out_dir.join("steps.jsonl"))
                .expect("journal")
                .trim(),
        )
        .expect("step");
        assert_eq!(step["status"], "unknown");
        assert_eq!(step["replay_class"], "never");
        assert_eq!(
            out.stdout_json()["result"]["coordinate_click"]["dispatch"],
            "unknown"
        );
    }
}

fn mcp_fixture(tool: &str, arguments: Value, result: Value) -> (Value, Value) {
    mcp_fixture_status(tool, arguments, result, 0)
}

fn mcp_fixture_status(
    tool: &str,
    arguments: Value,
    result: Value,
    expected_code: i32,
) -> (Value, Value) {
    mcp_response_fixture(
        tool,
        arguments,
        json!({"jsonrpc":"2.0","id":1,"result":result}),
        expected_code,
    )
}

fn mcp_response_fixture(
    tool: &str,
    arguments: Value,
    response: Value,
    expected_code: i32,
) -> (Value, Value) {
    let harness = common::MacosAgentHarness::new();
    let cwd = TempDir::new().expect("cwd");
    let fake = cwd.path().join("peekaboo");
    let payload = cwd.path().join("response.json");
    fs::write(&payload, response.to_string()).expect("response");
    fs::write(
        &fake,
        r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"id":1'*) cat "$FIXTURE_PAYLOAD"; printf '\n' ;;
    *'"id":2'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":null}' ;;
  esac
done
"#,
    )
    .expect("fixture");
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod");
    let input = format!(
        "{}\n{}\n",
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":arguments}}),
        json!({"jsonrpc":"2.0","id":2,"method":"shutdown"})
    );
    let out_dir = cwd.path().join("journal");
    let out = harness.run_with_options(
        cwd.path(),
        &[
            "mcp",
            "--tool-profile",
            "interact",
            "--out-dir",
            out_dir.to_str().expect("out"),
        ],
        harness
            .cmd_options(cwd.path())
            .with_env(
                "NILS_MACOS_AGENT_PEEKABOO_BIN",
                fake.to_str().expect("path"),
            )
            .with_env("FIXTURE_PAYLOAD", payload.to_str().expect("path"))
            .with_stdin_str(&input),
    );
    assert_eq!(out.code, expected_code, "{}", out.stderr_text());
    let frame = out
        .stdout_text()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("frame"))
        .find(|frame| frame["id"] == 1)
        .unwrap_or(Value::Null);
    let step = fs::read_to_string(out_dir.join("steps.jsonl"))
        .expect("journal")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("step"))
        .find(|step| step["mcp_tool"] == tool || step["mcp_method"] == "session")
        .expect("tool or terminal step");
    (frame, step)
}

#[test]
fn mcp_coordinate_protocol_errors_preserve_error_and_expose_replay_guidance() {
    for data in [
        None,
        Some(json!({"detail":"Synthetic validation detail"})),
        Some(json!("Synthetic detail")),
    ] {
        let mut error = json!({"code":-32602,"message":"Invalid arguments"});
        if let Some(data) = &data {
            error["data"] = data.clone();
        }
        let (frame, step) = mcp_response_fixture(
            "click",
            json!({"coords":"20,30","foreground":true}),
            json!({"jsonrpc":"2.0","id":1,"error":error}),
            0,
        );
        assert!(frame.get("result").is_none());
        assert_eq!(frame["error"]["code"], error["code"]);
        assert_eq!(frame["error"]["message"], error["message"]);
        let diagnostic = if data.as_ref().is_some_and(|data| !data.is_object()) {
            assert_eq!(frame["error"]["data"], error["data"]);
            &frame["error"]["macos-agent/coordinate_click"]
        } else {
            if let Some(data) = &data {
                for (key, value) in data.as_object().expect("data object") {
                    assert_eq!(&frame["error"]["data"][key], value);
                }
            }
            &frame["error"]["data"]["macos-agent/coordinate_click"]
        };
        assert_eq!(diagnostic["dispatch"], "unknown");
        assert!(
            diagnostic["guidance"]
                .as_str()
                .expect("guidance")
                .contains("Do not replay")
        );
        assert_eq!(step["status"], "unknown");
        assert_eq!(step["failure_class"], "unknown_mutation");
        assert_eq!(step["replay_class"], "never");
        assert_eq!(step["retries"], 0);
    }
}

#[test]
fn mcp_coordinate_refusal_and_uncertainty_preserve_canonical_metadata() {
    for dispatched in [false, true] {
        let metadata = if dispatched {
            json!({"mutation_dispatched":true,"effect":"unverifiable","state":"indeterminate","retry_safe":false})
        } else {
            json!({"mutation_dispatched":false,"effect":"refused","refusal_reason":"target_unavailable","retry_safe":true})
        };
        let (frame, step) = mcp_fixture(
            "click",
            json!({"coords":"20,30","foreground":true}),
            json!({"isError":true,"_meta":metadata,"content":[{"type":"text","text":"Synthetic target unavailable"}]}),
        );
        assert_eq!(
            frame["result"]["_meta"]["mutation_dispatched"],
            metadata["mutation_dispatched"]
        );
        assert_eq!(
            frame["result"]["_meta"]["macos-agent/coordinate_click"]["dispatch"],
            if dispatched {
                "dispatched"
            } else {
                "not_dispatched"
            }
        );
        assert_eq!(
            step["status"],
            if dispatched { "unknown" } else { "failed" }
        );
        assert_eq!(
            step["failure_class"],
            if dispatched {
                "unknown_mutation"
            } else {
                "upstream_refused"
            }
        );
        if dispatched {
            assert_eq!(step["replay_class"], "never");
        }
    }
}

#[test]
fn malformed_mcp_observation_fields_cannot_suppress_scope_qualification() {
    let (frame, _) = mcp_fixture(
        "inspect_ui",
        json!({}),
        json!({"_meta":"malformed","content":null,"structuredContent":{"summary":"Visible controls"}}),
    );
    assert_eq!(
        frame["result"]["_meta"]["macos-agent/observation_scope"]["ax_visibility"],
        "unverified"
    );
    assert!(
        frame["result"]["content"][0]["text"]
            .as_str()
            .expect("notice")
            .contains("occluded")
    );
}

#[test]
fn non_object_mcp_observations_are_rejected_instead_of_forwarded_unqualified() {
    for result in [Value::Null, json!("Visible controls"), json!([])] {
        let (frame, step) = mcp_fixture_status("inspect_ui", json!({}), result, 70);
        assert_eq!(frame, Value::Null);
        assert_eq!(step["failure_class"], "mcp_protocol");
    }
}
