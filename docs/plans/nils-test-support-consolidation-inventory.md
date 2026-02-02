# nils-test-support Consolidation Inventory

## Overview
This inventory captures duplicated test helpers across crates and maps them to shared modules in `crates/nils-test-support`. It includes both `crates/*/tests` and `#[cfg(test)]` helpers in `crates/*/src`.

## Keep-local criteria
- Helper is unique to a single test file or scenario and not reused elsewhere.
- Helper behavior is tightly coupled to crate-specific semantics and would add unclear scope to shared helpers.
- Helper lives in `#[cfg(test)]` blocks where reuse would require invasive refactors without clear duplication.

## Categories and candidates

### Binary resolver helpers (`*_bin`)
| Locations | Proposed module | Action | Notes |
| --- | --- | --- | --- |
| `crates/cli-template/tests/cli.rs` | `nils_test_support::bin` | migrate | `cli_template_bin` |
| `crates/api-gql/tests/cli_smoke.rs` | `nils_test_support::bin` | migrate | `api_gql_bin` |
| `crates/api-gql/tests/env_and_auth_resolution.rs` | `nils_test_support::bin` | migrate | `api_gql_bin` |
| `crates/api-gql/tests/integration.rs` | `nils_test_support::bin` | migrate | `api_gql_bin` |
| `crates/api-gql/tests/schema_command.rs` | `nils_test_support::bin` | migrate | `api_gql_bin` |
| `crates/api-rest/tests/auth_resolution.rs` | `nils_test_support::bin` | migrate | `api_rest_bin` |
| `crates/api-rest/tests/cli_smoke.rs` | `nils_test_support::bin` | migrate | `api_rest_bin` |
| `crates/api-rest/tests/endpoint_resolution.rs` | `nils_test_support::bin` | migrate | `api_rest_bin` |
| `crates/api-rest/tests/integration.rs` | `nils_test_support::bin` | migrate | `api_rest_bin` |
| `crates/api-rest/tests/report_from_cmd.rs` | `nils_test_support::bin` | migrate | `api_rest_bin` |
| `crates/api-test/tests/cli_smoke.rs` | `nils_test_support::bin` | migrate | `api_test_bin` |
| `crates/api-test/tests/e2e.rs` | `nils_test_support::bin` | migrate | `api_test_bin` |
| `crates/codex-cli/tests/auth_auto_refresh.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/auth_current_sync.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/auth_refresh.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/auth_use.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/agent_commit.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/agent_templates.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/config.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/dispatch.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/main_entrypoint.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/rate_limits_all.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/rate_limits_async.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/rate_limits_client.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/rate_limits_client_more.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/rate_limits_network.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/rate_limits_render.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/rate_limits_single.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/starship_cached.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/codex-cli/tests/starship_refresh.rs` | `nils_test_support::bin` | migrate | `codex_cli_bin` |
| `crates/fzf-cli/tests/common.rs` | `nils_test_support::bin` | migrate | `fzf_cli_bin` |
| `crates/git-lock/tests/common.rs` | `nils_test_support::bin` | migrate | `git_lock_bin` |
| `crates/git-scope/tests/common.rs` | `nils_test_support::bin` | migrate | `git_scope_bin` |
| `crates/git-summary/tests/common.rs` | `nils_test_support::bin` | migrate | `git_summary_bin` |
| `crates/image-processing/tests/common.rs` | `nils_test_support::bin` | migrate | `image_processing_bin` |
| `crates/plan-tooling/tests/common.rs` | `nils_test_support::bin` | migrate | `plan_tooling_bin` |
| `crates/semantic-commit/tests/common.rs` | `nils_test_support::bin` | migrate | `semantic_commit_bin` |

### Command runners + output structs (`CmdOutput`, `CmdOut`)
| Locations | Proposed module | Action | Notes |
| --- | --- | --- | --- |
| `crates/api-gql/tests/cli_smoke.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_gql` |
| `crates/api-gql/tests/env_and_auth_resolution.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_gql` |
| `crates/api-gql/tests/integration.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_gql` |
| `crates/api-gql/tests/schema_command.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_gql` |
| `crates/api-rest/tests/auth_resolution.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_rest` |
| `crates/api-rest/tests/cli_smoke.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_rest` |
| `crates/api-rest/tests/endpoint_resolution.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_rest` |
| `crates/api-rest/tests/integration.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_rest` |
| `crates/api-rest/tests/report_from_cmd.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_rest` |
| `crates/api-test/tests/cli_smoke.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_test` |
| `crates/api-test/tests/e2e.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_api_test` |
| `crates/fzf-cli/tests/common.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_fzf_cli` |
| `crates/image-processing/tests/common.rs` | `nils_test_support::cmd` | migrate | `CmdOutput` + `run_image_processing` |
| `crates/plan-tooling/tests/common.rs` | `nils_test_support::cmd` | migrate | `CmdOut` + `run_plan_tooling` |
| `crates/codex-cli/tests/*` | `nils_test_support::cmd` | migrate | `Command::new(codex_cli_bin)` patterns |
| `crates/cli-template/tests/cli.rs` | `nils_test_support::cmd` | migrate | `Command::new(cli_template_bin)` |

### Git repo helpers (init repo, commit helpers, git wrapper)
| Locations | Proposed module | Action | Notes |
| --- | --- | --- | --- |
| `crates/git-lock/tests/common.rs` | `nils_test_support::git` | migrate | includes `init_repo`, `commit_file`, `repo_id` |
| `crates/git-scope/tests/common.rs` | `nils_test_support::git` | migrate | includes deterministic branch creation |
| `crates/git-summary/tests/common.rs` | `nils_test_support::git` | migrate | similar to git-scope without branch reset |
| `crates/plan-tooling/tests/common.rs` | `nils_test_support::git` | migrate | includes `init_repo` and `git` |
| `crates/semantic-commit/tests/common.rs` | `nils_test_support::git` | migrate | includes tag signing config |

### Filesystem writer helpers
| Locations | Proposed module | Action | Notes |
| --- | --- | --- | --- |
| `crates/api-gql/tests/integration.rs` | `nils_test_support::fs` | migrate | `write_file`, `write_str` |
| `crates/api-rest/tests/endpoint_resolution.rs` | `nils_test_support::fs` | migrate | `write_json`, `write_file` |
| `crates/api-rest/tests/auth_resolution.rs` | `nils_test_support::fs` | migrate | `write_file` |
| `crates/api-rest/tests/integration.rs` | `nils_test_support::fs` | migrate | `write_file`, `write_str` |
| `crates/api-rest/tests/report_from_cmd.rs` | `nils_test_support::fs` | migrate | `write_json`, `write_file` |
| `crates/api-test/tests/e2e.rs` | `nils_test_support::fs` | migrate | `write_str` |
| `crates/plan-tooling/tests/common.rs` | `nils_test_support::fs` | migrate | `write_file` |
| `crates/semantic-commit/tests/common.rs` | `nils_test_support::fs` | migrate | `write_file`, `write_executable` |
| `crates/api-testing-core/src/suite/schema.rs` | `nils_test_support::fs` | keep local | helper is private to unit tests and tied to schema fixtures |

### HTTP test server + request parsing helpers
| Locations | Proposed module | Action | Notes |
| --- | --- | --- | --- |
| `crates/api-gql/tests/integration.rs` | `nils_test_support::http` | migrate | request parsing + header capture + router |
| `crates/api-gql/tests/env_and_auth_resolution.rs` | `nils_test_support::http` | migrate | simple header capture server |
| `crates/api-rest/tests/integration.rs` | `nils_test_support::http` | migrate | request parsing + router |
| `crates/api-rest/tests/endpoint_resolution.rs` | `nils_test_support::http` | migrate | simple JSON server |
| `crates/api-rest/tests/auth_resolution.rs` | `nils_test_support::http` | migrate | header capture server |
| `crates/api-test/tests/e2e.rs` | `nils_test_support::http` | migrate | request parsing + router |
| `crates/api-testing-core/src/suite/runner/mod.rs` | `nils_test_support::http` | keep local | unit-test-only server; reuse is optional after API crates migrate |

### Existing shared helpers already in nils-test-support
| Locations | Module | Action | Notes |
| --- | --- | --- | --- |
| `crates/nils-test-support/src/lib.rs` | `EnvGuard`, `CwdGuard`, `GlobalStateLock`, `StubBinDir` | keep shared | already used by codex-cli and fzf/image tests |
| `crates/nils-test-support/src/stubs.rs` | stub scripts for fzf/image tools | keep shared | used by `fzf-cli` and `image-processing` tests |
| `crates/nils-test-support/src/fixtures.rs` | REST/GraphQL setup fixtures | keep shared | already used in `api-testing-core` tests |
| `crates/nils-test-support/src/http.rs` | `LoopbackServer` | extend | currently used by `api-testing-core` + `codex-cli` tests |

## Proposed modules and API mapping
| New module | Intended API (draft) | Maps from |
| --- | --- | --- |
| `nils_test_support::bin` | `fn resolve(bin_name: &str) -> PathBuf` (handles hyphen/underscore env vars) | all `*_bin()` helpers |
| `nils_test_support::cmd` | `struct CmdOutput { code: i32, stdout: Vec<u8>, stderr: Vec<u8> }`, `fn run(bin: &Path, args: &[&str], env: &[(&str,&str)], stdin: Option<&[u8]>) -> CmdOutput` | per-crate `run_*` helpers |
| `nils_test_support::git` | `fn git(dir: &Path, args: &[&str]) -> String`, `fn init_repo(opts: InitRepo) -> TempDir`, `fn commit_file(...)` | git-* and semantic-commit/plan-tooling helpers |
| `nils_test_support::fs` | `fn write_text(path: &Path, contents: &str) -> PathBuf`, `fn write_bytes(...)`, `fn write_json(...)`, `fn write_executable(...)` | write helpers in api-* / plan-tooling / semantic-commit |
| `nils_test_support::http` | `TestServer` (router + request capture), `RecordedRequest` (method/path/headers/body), `HttpResponse` helpers | API test server helpers in api-* tests |

## Review results (Task 3.5)
- `api-testing-core`: no additional shared helpers migrated beyond existing `nils-test-support` usage; keep `#[cfg(test)]` helpers local.
- `nils-term`: tests are self-contained without shared helper duplication; keep local.
