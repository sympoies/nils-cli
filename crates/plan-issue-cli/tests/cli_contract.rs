use std::path::PathBuf;

use clap::Parser;
use pretty_assertions::assert_eq;

use plan_issue_cli::cli::{Cli, OutputFormat};
use plan_issue_cli::commands::{Command, PrGrouping};

mod common;

#[test]
fn cli_help_lists_full_surface_for_live_and_local_bins() {
    let live = common::run_plan_issue(&["--help"]);
    assert_eq!(live.code, 0, "stderr: {}", live.stderr);

    for token in [
        "build-task-spec",
        "build-plan-task-spec",
        "start-plan",
        "status-plan",
        "ready-plan",
        "close-plan",
        "cleanup-worktrees",
        "start-sprint",
        "ready-sprint",
        "accept-sprint",
        "multi-sprint-guide",
        "-V, --version",
        "Usage paths",
        "plan-issue-local",
    ] {
        assert!(
            live.stdout.contains(token),
            "help output missing token `{token}`\n{}",
            live.stdout
        );
    }

    let local = common::run_plan_issue_local(&["--help"]);
    assert_eq!(local.code, 0, "stderr: {}", local.stderr);
    assert!(
        local.stdout.contains("plan-issue-local"),
        "{}",
        local.stdout
    );
    assert!(local.stdout.contains("Usage paths"), "{}", local.stdout);
}

#[test]
fn cli_parse_contract_build_task_spec_accepts_per_spring_alias() {
    let cli = Cli::try_parse_from([
        "plan-issue",
        "build-task-spec",
        "--plan",
        "docs/plans/plan-issue-rust-cli-full-delivery-plan.md",
        "--sprint",
        "2",
        "--pr-grouping",
        "per-spring",
    ])
    .expect("parse build-task-spec");

    assert_eq!(
        cli.resolve_output_format().expect("output format"),
        OutputFormat::Text
    );
    cli.validate().expect("validation");

    match &cli.command {
        Command::BuildTaskSpec(args) => {
            assert_eq!(
                args.plan,
                PathBuf::from("docs/plans/plan-issue-rust-cli-full-delivery-plan.md")
            );
            assert_eq!(args.sprint, 2);
            assert_eq!(args.grouping.pr_grouping, PrGrouping::PerSprint);
            assert!(args.grouping.pr_group.is_empty());
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn cli_parse_contract_start_sprint_parses_typed_group_mapping() {
    let cli = Cli::try_parse_from([
        "plan-issue",
        "start-sprint",
        "--plan",
        "docs/plans/plan-issue-rust-cli-full-delivery-plan.md",
        "--issue",
        "217",
        "--sprint",
        "2",
        "--pr-grouping",
        "group",
        "--pr-group",
        "S2T1=s2-core",
        "--pr-group",
        "Task 2.2=s2-core",
    ])
    .expect("parse start-sprint");

    cli.validate().expect("validation");

    match &cli.command {
        Command::StartSprint(args) => {
            assert_eq!(args.issue, 217);
            assert_eq!(args.sprint, 2);
            assert_eq!(args.grouping.pr_grouping, PrGrouping::Group);
            assert_eq!(args.grouping.pr_group.len(), 2);
            assert_eq!(args.grouping.pr_group[0].task, "S2T1");
            assert_eq!(args.grouping.pr_group[0].group, "s2-core");
            assert_eq!(args.grouping.pr_group[1].task, "Task 2.2");
            assert_eq!(args.grouping.pr_group[1].group, "s2-core");
        }
        other => panic!("unexpected command parsed: {other:?}"),
    }
}

#[test]
fn cli_conflict_rules_reject_pr_group_without_group_mode() {
    let cli = Cli::try_parse_from([
        "plan-issue",
        "build-plan-task-spec",
        "--plan",
        "plan.md",
        "--pr-grouping",
        "per-sprint",
        "--pr-group",
        "S2T1=s2-core",
    ])
    .expect("parse should succeed before semantic validation");

    let err = cli.validate().expect_err("semantic validation should fail");
    assert_eq!(err.code, "invalid-pr-grouping");
    assert!(err.message.contains("only valid"), "{}", err.message);
}

#[test]
fn cli_conflict_rules_require_pr_group_mapping_for_group_mode() {
    let cli = Cli::try_parse_from([
        "plan-issue",
        "build-plan-task-spec",
        "--plan",
        "plan.md",
        "--pr-grouping",
        "group",
    ])
    .expect("parse should succeed before semantic validation");

    let err = cli.validate().expect_err("semantic validation should fail");
    assert_eq!(err.code, "invalid-pr-grouping");
    assert!(err.message.contains("requires at least one --pr-group"));
}

#[test]
fn cli_conflict_rules_reject_summary_and_summary_file_together() {
    let err = Cli::try_parse_from([
        "plan-issue",
        "ready-plan",
        "--issue",
        "217",
        "--summary",
        "done",
        "--summary-file",
        "summary.md",
    ])
    .expect_err("clap should reject conflicting args");

    let rendered = err.to_string();
    assert!(
        rendered.contains("cannot be used with") || rendered.contains("conflicts with"),
        "{rendered}"
    );
}
