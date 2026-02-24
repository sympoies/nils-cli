use std::fs;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use serde_json::Value;
use tempfile::TempDir;

mod common;

const DUCK_PLAN: &str = "crates/plan-tooling/tests/fixtures/split_prs/duck-plan.md";
const SHELL_PARITY_DIR: &str = "tests/fixtures/shell_parity";

fn parse_json(stdout: &str) -> Value {
    serde_json::from_str(stdout).expect("stdout should be valid JSON")
}

fn shell_fixture(name: &str) -> String {
    let path = PathBuf::from(SHELL_PARITY_DIR).join(name);
    fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("failed to read fixture {}: {err}", path.display());
    })
}

fn normalize_shell_text(text: &str, agent_home: &str) -> String {
    text.trim()
        .replace(agent_home, "$AGENT_HOME")
        .replace("\\<", "<")
        .replace("\\>", ">")
}

fn fixture_subcommands(help_fixture: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut in_section = false;
    for line in help_fixture.lines() {
        if line.trim() == "Subcommands:" {
            in_section = true;
            continue;
        }
        if in_section && line.trim().is_empty() {
            break;
        }
        if !in_section {
            continue;
        }

        if !line.starts_with("  ") {
            continue;
        }
        let trimmed = line.trim();
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let Some(name) = parts.next() else { continue };
        let desc = parts.next().unwrap_or("").trim();
        if !name.is_empty() && !desc.is_empty() {
            rows.push((name.to_string(), desc.to_string()));
        }
    }
    rows
}

#[test]
fn parity_shell_help_surface_tracks_shell_fixture_commands() {
    let fixture = shell_fixture("help.txt");
    let out = common::run_plan_issue(&["--help"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    for (name, description) in fixture_subcommands(&fixture) {
        assert!(
            out.stdout.contains(&name),
            "help output missing subcommand `{name}`\n{}",
            out.stdout
        );
        assert!(
            out.stdout.contains(&description),
            "help output missing description `{description}`\n{}",
            out.stdout
        );
    }

    for token in [
        "--repo <owner/repo>",
        "Pass-through repository target for GitHub operations",
        "--dry-run",
        "Print write actions without mutating GitHub state",
        "--force",
        "Bypass markdown payload guard for GitHub body/comment writes",
    ] {
        assert!(
            out.stdout.contains(token),
            "help output missing token `{token}`\n{}",
            out.stdout
        );
    }
}

#[test]
fn parity_shell_multi_sprint_guide_matches_shell_fixture_after_normalization() {
    let tmp = TempDir::new().expect("temp dir");
    let agent_home = tmp.path().join("agent-home");
    fs::create_dir_all(&agent_home).expect("agent home");
    let agent_home_s = agent_home.to_string_lossy().to_string();

    let out = common::run_plan_issue_local_with_env(
        &[
            "--format",
            "json",
            "--dry-run",
            "multi-sprint-guide",
            "--plan",
            DUCK_PLAN,
            "--from-sprint",
            "1",
            "--to-sprint",
            "2",
        ],
        &[("AGENT_HOME", &agent_home_s)],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let payload = parse_json(&out.stdout);
    let actual = payload["payload"]["result"]["guide"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let expected = shell_fixture("multi_sprint_guide_dry_run.txt");

    assert_eq!(
        normalize_shell_text(&actual, &agent_home_s),
        normalize_shell_text(&expected, &agent_home_s),
    );
}

#[test]
fn parity_shell_start_comment_template_matches_shell_fixture() {
    let tmp = TempDir::new().expect("temp dir");
    let agent_home = tmp.path().join("agent-home");
    fs::create_dir_all(&agent_home).expect("agent home");
    let agent_home_s = agent_home.to_string_lossy().to_string();

    let out = common::run_plan_issue_local_with_env(
        &[
            "--format",
            "json",
            "--dry-run",
            "start-sprint",
            "--plan",
            DUCK_PLAN,
            "--issue",
            "217",
            "--sprint",
            "1",
            "--pr-grouping",
            "group",
            "--pr-group",
            "S1T1=s1-foundation",
            "--pr-group",
            "S1T2=s1-fixtures",
            "--pr-group",
            "S1T3=s1-fixtures",
            "--no-comment",
        ],
        &[("AGENT_HOME", &agent_home_s)],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let payload = parse_json(&out.stdout);
    let comment_path = payload["payload"]["result"]["comment_path"]
        .as_str()
        .expect("comment path");
    let actual = fs::read_to_string(comment_path).expect("read rendered comment");
    let expected = shell_fixture("comment_template_start.md");

    assert!(actual.contains("pr-isolated"), "{actual}");
    assert!(actual.contains("pr-shared"), "{actual}");
    assert_eq!(actual.trim(), expected.trim());
}

#[test]
fn command_guardrails_close_plan_requires_body_file_in_dry_run_mode() {
    let out = common::run_plan_issue(&[
        "--format",
        "json",
        "--dry-run",
        "close-plan",
        "--issue",
        "217",
        "--approved-comment-url",
        "https://github.com/graysurf/nils-cli/issues/217#issuecomment-5000000001",
    ]);

    assert_eq!(out.code, 1, "stderr: {}", out.stderr);
    let payload = parse_json(&out.stdout);
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["error"]["code"], "missing-body-file");
    assert_eq!(
        payload["error"]["message"],
        "--body-file is required for close-plan --dry-run"
    );
}

#[test]
fn command_guardrails_close_plan_rejects_body_file_without_dry_run() {
    let tmp = TempDir::new().expect("temp dir");
    let body_file = tmp.path().join("body.md");
    fs::write(&body_file, "## Task Decomposition\n").expect("body file");
    let body_file_s = body_file.to_string_lossy().to_string();

    let out = common::run_plan_issue(&[
        "--format",
        "json",
        "close-plan",
        "--issue",
        "217",
        "--body-file",
        &body_file_s,
        "--approved-comment-url",
        "https://github.com/graysurf/nils-cli/issues/217#issuecomment-5000000002",
    ]);

    assert_eq!(out.code, 1, "stderr: {}", out.stderr);
    let payload = parse_json(&out.stdout);
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["error"]["code"], "conflicting-issue-source");
    assert_eq!(
        payload["error"]["message"],
        "use either --issue or --body-file for close-plan, not both"
    );
}

#[test]
fn command_guardrails_multi_sprint_guide_rejects_reverse_range() {
    let out = common::run_plan_issue(&[
        "--format",
        "json",
        "multi-sprint-guide",
        "--plan",
        DUCK_PLAN,
        "--from-sprint",
        "3",
        "--to-sprint",
        "1",
    ]);

    assert_eq!(out.code, 1, "stderr: {}", out.stderr);
    let payload = parse_json(&out.stdout);
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["error"]["code"], "invalid-sprint-range");
    assert_eq!(
        payload["error"]["message"],
        "--from-sprint must be <= --to-sprint"
    );
}

#[test]
fn json_contract_multi_sprint_guide_success_envelope_is_stable() {
    let out = common::run_plan_issue(&[
        "--format",
        "json",
        "--dry-run",
        "multi-sprint-guide",
        "--plan",
        DUCK_PLAN,
        "--from-sprint",
        "1",
        "--to-sprint",
        "2",
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let payload = parse_json(&out.stdout);
    assert_eq!(
        payload["schema_version"],
        "plan-issue-cli.multi.sprint.guide.v1"
    );
    assert_eq!(payload["command"], "multi-sprint-guide");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["payload"]["binary"], "plan-issue");
    assert_eq!(payload["payload"]["execution_mode"], "live");
    assert_eq!(payload["payload"]["dry_run"], true);
    assert_eq!(payload["payload"]["arguments"]["from_sprint"], 1);
    assert_eq!(payload["payload"]["arguments"]["to_sprint"], 2);
    assert!(
        payload["payload"]["result"]["guide"]
            .as_str()
            .is_some_and(|guide| guide.contains("MULTI_SPRINT_GUIDE_BEGIN")),
        "{}",
        out.stdout
    );
}

#[test]
fn json_contract_guardrail_error_envelope_is_stable() {
    let out = common::run_plan_issue(&[
        "--format",
        "json",
        "--dry-run",
        "close-plan",
        "--issue",
        "217",
        "--approved-comment-url",
        "https://github.com/graysurf/nils-cli/issues/217#issuecomment-5000000003",
    ]);
    assert_eq!(out.code, 1, "stderr: {}", out.stderr);

    let payload = parse_json(&out.stdout);
    assert_eq!(payload["schema_version"], "plan-issue-cli.close.plan.v1");
    assert_eq!(payload["command"], "close-plan");
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["error"]["code"], "missing-body-file");
    assert!(payload["error"]["message"].is_string(), "{}", out.stdout);
}

#[test]
fn json_contract_local_binary_success_envelope_is_stable() {
    let out = common::run_plan_issue_local(&[
        "--format",
        "json",
        "build-task-spec",
        "--plan",
        "docs/plans/plan-issue-rust-cli-full-delivery-plan.md",
        "--sprint",
        "2",
        "--pr-grouping",
        "per-sprint",
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let payload = parse_json(&out.stdout);
    assert_eq!(payload["command"], "build-task-spec");
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["payload"]["binary"], "plan-issue-local");
    assert_eq!(payload["payload"]["execution_mode"], "local");
}
