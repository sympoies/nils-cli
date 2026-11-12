//! `claude-cli usage`: scoped weekly windows and the "Limit resets" section.
//!
//! Bodies are synthetic. Epochs: `2023-11-14T22:13:20Z` is `1_700_000_000`.

use crate::support::*;
use nils_test_support::cmd::CmdOutput;
use nils_test_support::http::{HttpResponse, LoopbackServer, RecordedRequest};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

/// An individual plan: no 5h window, a scoped weekly window, no session-reset
/// program, and two grants that share one upstream `type` but clear different
/// limits. The `full` grant is next.
const INDIVIDUAL_PLAN: &str = r#"{
  "five_hour": null,
  "seven_day": { "utilization": 40.0, "resets_at": "2023-11-20T17:06:40+00:00" },
  "limits": [
    {
      "kind": "weekly_scoped",
      "scope": { "model": { "display_name": "Model A" } },
      "percent": 30.4,
      "resets_at": "2023-11-21T17:06:40+00:00"
    }
  ],
  "juniper_tide": null,
  "cedar_ember": {
    "eligible": true,
    "next_grant_id": "grant-full",
    "grants": [
      {
        "id": "grant-full",
        "type": "reset",
        "label": "Full reset",
        "resets_left": 2,
        "resets_total": 2,
        "ends_at": "2023-12-01T00:00:00+00:00",
        "clears": ["five_hour", "seven_day"],
        "usable_now": true
      },
      {
        "id": "grant-model",
        "type": "reset",
        "label": "Model A reset",
        "resets_left": 1,
        "resets_total": 3,
        "ends_at": "2023-11-30T12:00:00+00:00",
        "clears": ["weekly_scoped"],
        "usable_now": false
      }
    ]
  }
}"#;

/// A team plan: a 5h window, a scoped weekly window, a session-reset program
/// that is offered but not usable yet, and two grants (model first, `full`
/// next) that share one upstream `type`.
const TEAM_PLAN: &str = r#"{
  "five_hour": { "utilization": 25.0, "resets_at": "2023-11-14T22:13:20+00:00" },
  "seven_day": { "utilization": 40.0, "resets_at": "2023-11-20T17:06:40+00:00" },
  "limits": [
    { "kind": "five_hour", "percent": 25 },
    {
      "kind": "weekly_scoped",
      "scope": { "model": { "display_name": "Model B" } },
      "percent": 5,
      "resets_at": "2023-11-21T17:06:40+00:00"
    }
  ],
  "juniper_tide": {
    "eligible": false,
    "available": false,
    "ineligible_reason": "not_at_wall",
    "arm": "reset",
    "resets_per_week": 1,
    "next_available_at": "2023-11-18T09:00:00+00:00"
  },
  "cedar_ember": {
    "eligible": true,
    "next_grant_id": "grant-full",
    "grants": [
      {
        "id": "grant-model",
        "type": "reset",
        "label": "Model B reset",
        "resets_left": 3,
        "resets_total": 3,
        "ends_at": "2023-11-30T12:00:00+00:00",
        "clears": ["weekly_scoped"],
        "usable_now": true
      },
      {
        "id": "grant-full",
        "type": "reset",
        "resets_left": 1,
        "ends_at": "2023-12-01T00:00:00+00:00",
        "clears": ["seven_day"],
        "usable_now": true
      }
    ]
  }
}"#;

fn run_usage(body: &'static str, format: &str) -> (CmdOutput, Vec<RecordedRequest>) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/usage",
        HttpResponse::new(200, body).with_header("Content-Type", "application/json"),
    );
    let output = run(
        &["usage", "--format", format, "--source", "oauth"],
        &base_options(tmp.path())
            .with_env("CLAUDE_PROMPT_SEGMENT_ACCESS_TOKEN", "secret-token-limits")
            .with_env(
                "CLAUDE_PROMPT_SEGMENT_ENDPOINT",
                &format!("{}/usage", server.url()),
            )
            .with_env("CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION", "9.8.7"),
    );
    assert_exit(&output, 0);
    assert!(!stdout(&output).contains("secret-token-limits"));
    assert!(!stdout(&output).contains("grant-"), "grant ids stay local");
    (output, server.take_requests())
}

fn usage_result(output: &CmdOutput) -> Value {
    let payload: Value = serde_json::from_str(&stdout(output)).expect("json");
    assert_eq!(payload["schema_version"], "claude-cli.usage.v1");
    assert_eq!(payload["ok"], true);
    payload["result"].clone()
}

fn key_percent_reset(result: &Value) -> Vec<Value> {
    result["windows"]
        .as_array()
        .expect("windows")
        .iter()
        .map(|window| {
            json!([
                window["key"],
                window["remaining_percent"],
                window["resets_at_epoch"]
            ])
        })
        .collect()
}

#[test]
fn usage_individual_plan_json_reports_scoped_window_and_limit_resets() {
    let (output, requests) = run_usage(INDIVIDUAL_PLAN, "json");

    let result = usage_result(&output);
    assert_eq!(result["source"], "oauth");
    assert_eq!(
        key_percent_reset(&result),
        vec![
            json!(["weekly", 60.0, 1_700_500_000]),
            json!(["weekly_scoped:Model A", 70.0, 1_700_586_400]),
        ]
    );
    let scoped = &result["windows"][1];
    assert_eq!(scoped["label"], "Weekly (Model A)");
    assert_eq!(scoped["window_minutes"], 10_080);
    assert_eq!(scoped["used_percent"], 30.4);
    assert_eq!(
        result["limit_resets"],
        json!({
            "session": { "state": "not_offered" },
            "grants": [
                {
                    "kind": "full",
                    "label": "Full reset",
                    "resets_left": 2,
                    "resets_total": 2,
                    "ends_at": 1_701_388_800,
                    "usable_now": true,
                    "next": true
                },
                {
                    "kind": "model",
                    "label": "Model A reset",
                    "resets_left": 1,
                    "resets_total": 3,
                    "ends_at": 1_701_345_600,
                    "usable_now": false,
                    "next": false
                }
            ]
        })
    );

    // The read uses the Claude Code status identity, which is what makes the
    // endpoint return grants.
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].query.as_deref(), Some("at_wall=1&skip_spend=1"));
    assert_eq!(
        requests[0].header_value("user-agent").as_deref(),
        Some("claude-cli/9.8.7 (external, cli)")
    );
}

#[test]
fn usage_team_plan_json_reports_five_hour_scoped_window_and_unavailable_session_reset() {
    let (output, _) = run_usage(TEAM_PLAN, "json");

    let result = usage_result(&output);
    assert_eq!(
        key_percent_reset(&result),
        vec![
            json!(["5h", 75.0, 1_700_000_000]),
            json!(["weekly", 60.0, 1_700_500_000]),
            json!(["weekly_scoped:Model B", 95.0, 1_700_586_400]),
        ]
    );
    assert_eq!(
        result["limit_resets"],
        json!({
            "session": {
                "state": "not_available",
                "reason": "not_at_wall",
                "next_available_at": 1_700_298_000
            },
            "grants": [
                {
                    "kind": "model",
                    "label": "Model B reset",
                    "resets_left": 3,
                    "resets_total": 3,
                    "ends_at": 1_701_345_600,
                    "usable_now": true,
                    "next": false
                },
                {
                    "kind": "full",
                    "resets_left": 1,
                    "ends_at": 1_701_388_800,
                    "usable_now": true,
                    "next": true
                }
            ]
        })
    );
}

#[test]
fn usage_individual_plan_text_lists_scoped_window_and_limit_resets() {
    let (output, _) = run_usage(INDIVIDUAL_PLAN, "text");

    assert_eq!(
        stdout(&output),
        "source=oauth Weekly:60% Weekly (Model A):70%\n\
         Limit resets\n\
         \x20 session: not offered\n\
         \x20 grant: full \"Full reset\" 2/2 left, expires 2023-12-01T00:00Z, usable now, next\n\
         \x20 grant: model \"Model A reset\" 1/3 left, expires 2023-11-30T12:00Z, not usable now\n"
    );
}

#[test]
fn usage_team_plan_text_lists_scoped_window_and_limit_resets() {
    let (output, _) = run_usage(TEAM_PLAN, "text");

    assert_eq!(
        stdout(&output),
        "source=oauth 5h:75% Weekly:60% Weekly (Model B):95%\n\
         Limit resets\n\
         \x20 session: not available (not_at_wall), next 2023-11-18T09:00Z\n\
         \x20 grant: model \"Model B reset\" 3/3 left, expires 2023-11-30T12:00Z, usable now\n\
         \x20 grant: full 1 left, expires 2023-12-01T00:00Z, usable now, next\n"
    );
}

#[test]
fn usage_body_without_reset_programs_reports_not_offered_and_no_grants() {
    let (output, _) = run_usage(
        r#"{"five_hour":{"utilization":25.0},"seven_day":{"utilization":50.0}}"#,
        "json",
    );

    let result = usage_result(&output);
    assert_eq!(
        key_percent_reset(&result),
        vec![json!(["5h", 75.0, null]), json!(["weekly", 50.0, null])]
    );
    // Neither program was in the body: both read as absent, not as a missing read.
    assert_eq!(
        result["limit_resets"],
        json!({ "session": { "state": "not_offered" }, "grants": [] })
    );
}

#[test]
fn usage_cache_source_keeps_the_scoped_window_and_omits_limit_resets() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("usage.json"), INDIVIDUAL_PLAN).expect("cache");

    let output = run(
        &["usage", "--format", "json", "--source", "cache"],
        &base_options(tmp.path()),
    );

    assert_exit(&output, 0);
    let result = usage_result(&output);
    assert_eq!(result["source"], "cache");
    assert_eq!(
        key_percent_reset(&result),
        vec![
            json!(["weekly", 60.0, 1_700_500_000]),
            json!(["weekly_scoped:Model A", 70.0, 1_700_586_400]),
        ]
    );
    // Reset programs are only read from a live response; a cache read is not
    // a read of them.
    assert!(result.get("limit_resets").is_none(), "{result}");
}
