# Markdown + GitHub Handling Audit v1

## Purpose

Provide a workspace-wide, source-only inventory of markdown handling and GitHub markdown-write touchpoints, and track remediation status
with machine-checkable rows.

## Inventory boundary

- Include scope:
  - `crates/**/src/**/*.rs` production Rust source files.
- Exclude scope:
  - `crates/**/tests/**`, `**/fixtures/**`, `**/docs/**`, generated outputs, and shell scripts.
- Discovery baseline command:

  ```bash
  rg -n --glob 'crates/**/src/**/*.rs' \
    'render_.*markdown|markdown::|validate_markdown_payload|canonicalize_table_cell|code_block\(|heading\(|write_temp_markdown|run_output\("gh"|parse_markdown_row|parse_sprint_heading|render_summary_markdown' \
    crates
  ```

## Risk classes

- `payload-guard`: markdown payload safety validation before write.
- `table-canonicalization`: markdown table-cell normalization for round-trip safety.
- `markdown-render`: markdown string construction/rendering.
- `markdown-write`: markdown file write paths.
- `markdown-parse`: markdown parser/heading/table interpretation logic.
- `markdown-detect`: markdown type detection/validation heuristics.
- `github-markdown-write`: GitHub CLI/API write paths using markdown body/comment payloads.
- `github-non-markdown`: GitHub CLI usage not writing markdown payloads.

## Row schema

Each row must contain:

- `crate=<name>`
- `file=<repo-relative path>`
- `function=<function or responsibility>`
- `risk_class=<risk class>`
- `owner=<crate/module owner>`
- `test_ref=<test command or contract check>`
- `status=<open|resolved>`
- `notes=<short rationale>`

## Audit table

| crate                  | file                                                 | function                                              | risk_class                        | owner                       | test_ref                                                                                                                    | status          | notes                                                                          |
| ---------------------- | ---------------------------------------------------- | ----------------------------------------------------- | --------------------------------- | --------------------------- | --------------------------------------------------------------------------------------------------------------------------- | --------------- | ------------------------------------------------------------------------------ |
| crate=nils-common      | file=crates/nils-common/src/markdown.rs              | function=validate_markdown_payload                    | risk_class=payload-guard          | owner=nils-common           | test_ref=cargo test -p nils-common markdown_payload_validator_rejects_literal_escaped_controls -- --exact                   | status=resolved | notes=Shared guard rejects literal escaped controls.                           |
| crate=nils-common      | file=crates/nils-common/src/markdown.rs              | function=canonicalize_table_cell                      | risk_class=table-canonicalization | owner=nils-common           | test_ref=cargo test -p nils-common canonicalize_table_cell_is_idempotent -- --exact                                         | status=resolved | notes=Shared canonical table-cell implementation.                              |
| crate=nils-common      | file=crates/nils-common/src/markdown.rs              | function=heading                                      | risk_class=markdown-render        | owner=nils-common           | test_ref=cargo test -p nils-common markdown_heading_trims_and_clamps_level -- --exact                                       | status=resolved | notes=Shared heading renderer clamps and normalizes.                           |
| crate=nils-common      | file=crates/nils-common/src/markdown.rs              | function=code_block                                   | risk_class=markdown-render        | owner=nils-common           | test_ref=cargo test -p nils-common markdown_code_block_is_newline_stable -- --exact                                         | status=resolved | notes=Shared code-fence renderer with newline stability.                       |
| crate=nils-common      | file=crates/nils-common/src/markdown.rs              | function=format_json_pretty_sorted                    | risk_class=markdown-render        | owner=nils-common           | test_ref=cargo test -p nils-common json_format_sorts_keys_recursively -- --exact                                            | status=resolved | notes=Shared sorted JSON formatter for markdown sections.                      |
| crate=api-testing-core | file=crates/api-testing-core/src/markdown.rs         | function=format_json_pretty_sorted wrapper            | risk_class=markdown-render        | owner=nils-api-testing-core | test_ref=cargo test -p nils-api-testing-core --test report_history                                                          | status=resolved | notes=Wrapper delegates to nils-common canonical helpers.                      |
| crate=api-testing-core | file=crates/api-testing-core/src/markdown.rs         | function=heading wrapper                              | risk_class=markdown-render        | owner=nils-api-testing-core | test_ref=cargo test -p nils-api-testing-core --test report_history                                                          | status=resolved | notes=Uses shared heading behavior through wrapper.                            |
| crate=api-testing-core | file=crates/api-testing-core/src/markdown.rs         | function=code_block wrapper                           | risk_class=markdown-render        | owner=nils-api-testing-core | test_ref=cargo test -p nils-api-testing-core --test report_history                                                          | status=resolved | notes=Uses shared code-block behavior through wrapper.                         |
| crate=api-testing-core | file=crates/api-testing-core/src/report.rs           | function=ReportBuilder markdown composition           | risk_class=markdown-render        | owner=nils-api-testing-core | test_ref=cargo test -p nils-api-testing-core --test report_history                                                          | status=resolved | notes=Report builder composes markdown via shared wrappers.                    |
| crate=api-testing-core | file=crates/api-testing-core/src/rest/report.rs      | function=render_rest_report_markdown                  | risk_class=markdown-render        | owner=nils-api-testing-core | test_ref=cargo test -p nils-api-testing-core rest_report_renders_markdown_with_optional_sections -- --exact                 | status=resolved | notes=REST report markdown output covered by unit tests.                       |
| crate=api-testing-core | file=crates/api-testing-core/src/graphql/report.rs   | function=render_graphql_report_markdown               | risk_class=markdown-render        | owner=nils-api-testing-core | test_ref=cargo test -p nils-api-testing-core --test report_history                                                          | status=resolved | notes=GraphQL report markdown output covered by integration tests.             |
| crate=api-testing-core | file=crates/api-testing-core/src/grpc/report.rs      | function=render_grpc_report_markdown                  | risk_class=markdown-render        | owner=nils-api-testing-core | test_ref=cargo test -p nils-api-testing-core --test report_history                                                          | status=resolved | notes=gRPC report markdown output covered by integration tests.                |
| crate=api-testing-core | file=crates/api-testing-core/src/websocket/report.rs | function=render_websocket_report_markdown             | risk_class=markdown-render        | owner=nils-api-testing-core | test_ref=cargo test -p nils-api-testing-core --test report_history                                                          | status=resolved | notes=WebSocket report markdown output covered by integration tests.           |
| crate=api-testing-core | file=crates/api-testing-core/src/suite/summary.rs    | function=render_summary_markdown                      | risk_class=markdown-render        | owner=nils-api-testing-core | test_ref=cargo test -p nils-api-testing-core render_summary_markdown_handles_successful_runs -- --exact                     | status=resolved | notes=Suite-level markdown summary renderer has dedicated tests.               |
| crate=api-rest         | file=crates/api-rest/src/commands/report.rs          | function=report command markdown output path          | risk_class=markdown-render        | owner=nils-api-rest         | test_ref=cargo test -p nils-api-rest --tests                                                                                | status=resolved | notes=Command uses api-testing-core markdown report pipeline.                  |
| crate=api-gql          | file=crates/api-gql/src/commands/report.rs           | function=report command markdown output path          | risk_class=markdown-render        | owner=nils-api-gql          | test_ref=cargo test -p nils-api-gql --tests                                                                                 | status=resolved | notes=Command uses api-testing-core markdown report pipeline.                  |
| crate=api-grpc         | file=crates/api-grpc/src/commands/report.rs          | function=report command markdown output path          | risk_class=markdown-render        | owner=nils-api-grpc         | test_ref=cargo test -p nils-api-grpc --tests                                                                                | status=resolved | notes=Command uses api-testing-core markdown report pipeline.                  |
| crate=api-websocket    | file=crates/api-websocket/src/commands/report.rs     | function=report command markdown output path          | risk_class=markdown-render        | owner=nils-api-websocket    | test_ref=cargo test -p nils-api-websocket --tests                                                                           | status=resolved | notes=Command uses api-testing-core markdown report pipeline.                  |
| crate=plan-issue-cli   | file=crates/plan-issue-cli/src/github.rs             | function=guard_markdown_payload                       | risk_class=payload-guard          | owner=nils-plan-issue-cli   | test_ref=cargo test -p nils-plan-issue-cli gh_adapter_guard_rejects_escaped_payload_without_force -- --exact                | status=resolved | notes=Live GitHub writes enforce markdown payload guard.                       |
| crate=plan-issue-cli   | file=crates/plan-issue-cli/src/github.rs             | function=create_issue/edit_issue_body/comment_issue   | risk_class=github-markdown-write  | owner=nils-plan-issue-cli   | test_ref=cargo test -p nils-plan-issue-cli gh_adapter_live_methods_work_with_stubbed_gh -- --exact                          | status=resolved | notes=All body-file write paths are guard-wrapped.                             |
| crate=plan-issue-cli   | file=crates/plan-issue-cli/src/github.rs             | function=close_issue --comment guard                  | risk_class=github-markdown-write  | owner=nils-plan-issue-cli   | test_ref=cargo test -p nils-plan-issue-cli gh_adapter_guard_rejects_escaped_payload_without_force -- --exact                | status=resolved | notes=Inline close comment guarded unless force enabled.                       |
| crate=plan-issue-cli   | file=crates/plan-issue-cli/src/execute.rs            | function=write_temp_markdown                          | risk_class=markdown-write         | owner=nils-plan-issue-cli   | test_ref=cargo test -p nils-plan-issue-cli temp_markdown_and_prompt_outputs_use_agent_home_and_expected_paths -- --exact    | status=resolved | notes=Temporary markdown artifacts use deterministic output path.              |
| crate=plan-issue-cli   | file=crates/plan-issue-cli/src/render.rs             | function=render_plan_issue_body/render_sprint_comment | risk_class=markdown-render        | owner=nils-plan-issue-cli   | test_ref=cargo test -p nils-plan-issue-cli render_issue_body_start_plan_falls_back_when_preface_sections_missing -- --exact | status=resolved | notes=Issue/sprint markdown rendering paths are covered.                       |
| crate=plan-issue-cli   | file=crates/plan-issue-cli/src/render.rs             | function=parse_markdown_row                           | risk_class=markdown-parse         | owner=nils-plan-issue-cli   | test_ref=cargo test -p nils-plan-issue-cli --test runtime_truth_plan_and_sprint_flow                                        | status=resolved | notes=Table row parsing participates in runtime-truth tests.                   |
| crate=plan-issue-cli   | file=crates/plan-issue-cli/src/issue_body.rs         | function=canonicalize_table_value                     | risk_class=table-canonicalization | owner=nils-plan-issue-cli   | test_ref=cargo test -p nils-plan-issue-cli --test task_spec_flow                                                            | status=resolved | notes=Task decomposition markdown cells normalized by shared helper.           |
| crate=plan-issue-cli   | file=crates/plan-issue-cli/src/task_spec.rs          | function=notes canonicalization for task-spec rows    | risk_class=table-canonicalization | owner=nils-plan-issue-cli   | test_ref=cargo test -p nils-plan-issue-cli --test task_spec_flow                                                            | status=resolved | notes=Runtime metadata notes are canonicalized for markdown round-trip safety. |
| crate=memo-cli         | file=crates/memo-cli/src/preprocess/detect.rs        | function=looks_like_markdown                          | risk_class=markdown-detect        | owner=nils-memo-cli         | test_ref=cargo test -p nils-memo-cli --tests                                                                                | status=resolved | notes=Content-type detection is parse-only and not a markdown write path.      |
| crate=memo-cli         | file=crates/memo-cli/src/preprocess/validate.rs      | function=validate_markdown                            | risk_class=markdown-detect        | owner=nils-memo-cli         | test_ref=cargo test -p nils-memo-cli --tests                                                                                | status=resolved | notes=Validation checks markdown syntax heuristics only.                       |
| crate=plan-tooling     | file=crates/plan-tooling/src/parse.rs                | function=parse_sprint_heading/parse_task_heading      | risk_class=markdown-parse         | owner=nils-plan-tooling     | test_ref=cargo test -p nils-plan-tooling --test to_json                                                                     | status=resolved | notes=Plan markdown parser behavior covered by parse/to-json tests.            |
| crate=plan-tooling     | file=crates/plan-tooling/src/validate.rs             | function=validate_sprint_metadata/validate_task       | risk_class=markdown-parse         | owner=nils-plan-tooling     | test_ref=cargo test -p nils-plan-tooling --test validate                                                                    | status=resolved | notes=Plan markdown contract linting covered by validate tests.                |
| crate=plan-tooling     | file=crates/plan-tooling/src/split_prs.rs            | function=split-prs plan markdown split logic          | risk_class=markdown-parse         | owner=nils-plan-tooling     | test_ref=cargo test -p nils-plan-tooling --test split_prs                                                                   | status=resolved | notes=Plan-derived split behavior covered by split_prs tests.                  |
| crate=git-cli          | file=crates/git-cli/src/open.rs                      | function=run_gh_pr_view                               | risk_class=github-non-markdown    | owner=nils-git-cli          | test_ref=cargo test -p nils-git-cli --tests                                                                                 | status=resolved | notes=Uses gh web open only; not a markdown body/comment write path.           |

## Boundary decisions

- Keep live GitHub issue/PR body/comment writes in crate-local adapters (`plan-issue-cli` ownership).
- Keep shared markdown rendering/validation primitives in `nils-common::markdown`.
- Do not add `crates/nils-common/src/github.rs`.
