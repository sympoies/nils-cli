# API Testing Core Shared Extraction Inventory

## Overview
This inventory captures duplicated CLI helper logic across `api-gql`, `api-rest`, `api-test`, and
`api-testing-core` internal modules. Each helper is mapped to a single shared target in
`api_testing_core::cli_util`, with explicit warning-prefix and output-sink requirements to preserve
existing CLI behavior.

## Helper inventory + target API mapping
| Helper | Current locations | Target API | Notes (warnings + output) |
| --- | --- | --- | --- |
| `trim_non_empty` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs`, `crates/api-testing-core/src/graphql/auth.rs`, `crates/api-testing-core/src/suite/resolve.rs`, `crates/api-testing-core/src/suite/runner/graphql.rs`, `crates/api-testing-core/src/suite/runner/rest.rs` | `cli_util::trim_non_empty` | Pure helper; no warning output. |
| `bool_from_env` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs`, `crates/api-test/src/main.rs`, `crates/api-testing-core/src/graphql/auth.rs` | `cli_util::bool_from_env` | Must preserve warning prefixes: `api-gql`/`api-rest`/`api-test` include `"<tool>: warning:"` to stderr; core `graphql/auth.rs` writes bare warning strings into `Vec<String>` (no tool prefix, no `warning:` label). |
| `parse_u64_default` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs`, `crates/api-testing-core/src/graphql/auth.rs`, `crates/api-testing-core/src/suite/runner/mod.rs` | `cli_util::parse_u64_default` | Shared parsing + min behavior. |
| `shell_quote` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs`, `crates/api-testing-core/src/suite/runner/mod.rs` | `cli_util::shell_quote` | Used for command snippets and report rendering. |
| `slugify` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs` | `cli_util::slugify` | Used for report output names. |
| `maybe_relpath` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs` | `cli_util::maybe_relpath` | Used in report output + history record formatting. |
| `list_available_suffixes` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs`, `crates/api-testing-core/src/suite/resolve.rs` | `cli_util::list_available_suffixes` | Must preserve `export` handling, sorting, and dedup semantics. |
| `to_env_key` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs` | `cli_util::to_env_key` | Thin wrapper over `env_file::normalize_env_key`. |
| `find_git_root` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs` | `cli_util::find_git_root` | Returns `Option<PathBuf>` (CLI report behavior). |
| `history_timestamp_now` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs` | `cli_util::history_timestamp_now` | Time format preserved. |
| `report_stamp_now` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs` | `cli_util::report_stamp_now` | Time format preserved. |
| `report_date_now` | `crates/api-gql/src/util.rs`, `crates/api-rest/src/util.rs` | `cli_util::report_date_now` | Time format preserved. |

## Target signature notes
- `cli_util::bool_from_env(raw, name, default, tool_label, warnings)`
  - `tool_label: Option<&str>`
  - `warnings`: accepts `&mut dyn Write` (CLI stderr) or `&mut Vec<String>` (core warnings)
  - Message format when `tool_label` is present:
    - `"<tool>: warning: {name} must be true|false (got: {raw}); treating as false"`
  - Message format when `tool_label` is absent:
    - `"{name} must be true|false (got: {raw}); treating as false"`

## Call-site mappings (warnings)
- `api-gql`: use `tool_label = Some("api-gql")`, warnings sink is stderr.
- `api-rest`: use `tool_label = Some("api-rest")`, warnings sink is stderr.
- `api-test`: use `tool_label = Some("api-test")`, warnings sink is stderr.
- `api-testing-core/graphql/auth.rs`: use `tool_label = None`, warnings sink is `Vec<String>`.
