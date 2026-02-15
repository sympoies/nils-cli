# Plan: api-gql/api-rest `report-from-cmd` subcommands

## Overview
This plan adds a new `report-from-cmd` subcommand to both `api-gql` and `api-rest`.
The subcommand accepts a saved `call` command snippet (string arg or stdin), rewrites it into an equivalent `report` invocation,
and then runs report generation (or prints the rewritten command for `--dry-run`).
Compatibility with the legacy `api-report-from-cmd` shell script is best-effort; the priority is a reasonable, ergonomic UX.

## Scope
- In scope:
  - `api-gql report-from-cmd` and `api-rest report-from-cmd`.
  - Best-effort parsing of a single command snippet (supports quotes/escapes, `\` line-continuations, and ignores anything after a `|` pipe).
  - Usability rule: treat the first token as a path and match by **basename + PATH** (do not require the original path to exist).
  - Case name derivation when `--case` is omitted (based on op/request + env/url + jwt/token).
  - `--response <file|->` offline mode, `--out <path>`, `--dry-run`, and `--stdin`.
  - Zsh completions and docs updates.
- Out of scope:
  - Full shell compatibility (multiple pipelines, `$(...)`, process substitution, etc.).
  - Executing the post-pipe command (e.g. `| jq .`); it is ignored.
  - Adding a new binary (no `api-report-from-cmd` crate in this plan).
  - Extending `api-test` with this feature (kept focused on the two runner CLIs).

## Assumptions (if any)
1. Snippets primarily come from `api-gql history --command-only` / `api-rest history --command-only` or copied docs, and represent a single `call` invocation.
2. Report generation should reuse existing report logic (construct `ReportArgs` and call the existing `cmd_report` functions).
3. If `--response -` is used, the snippet must be provided as a positional argument (stdin is reserved for the response body).

## Sprint 1: Shared snippet parsing + rewrite core
**Goal**: Implement a small, reusable parser that turns a command snippet into structured inputs for the `report` subcommand.
**Demo/Validation**:
- Command(s): `cargo test -p api-testing-core cmd_snippet`
- Verify: parser handles typical history snippets, derives case names, and rejects invalid snippets with clear errors.

### Task 1.1: Add `cmd_snippet` module API in `api-testing-core`
- **Location**:
  - `crates/api-testing-core/src/lib.rs`
  - `crates/api-testing-core/src/cmd_snippet.rs`
- **Description**: Add a new module exposing a small public API for parsing a command snippet into a structured representation (kind + extracted flags + positional args) and for producing a `report`-ready struct used by the binaries.
- **Dependencies**: none
- **Complexity**: 6
- **Acceptance criteria**:
  - `api_testing_core::cmd_snippet` compiles and is exported from `api_testing_core::lib`.
  - API has explicit error types/messages suitable for end-user CLI output.
- **Validation**:
  - `cargo test -p api-testing-core`

### Task 1.2: Implement normalization + tokenization (best-effort shell words)
- **Location**:
  - `crates/api-testing-core/src/cmd_snippet.rs`
- **Description**: Implement snippet preprocessing and tokenization: expand `$VARS`/`${VARS}` (best-effort), remove `\\\n` continuations, normalize newlines, split into tokens with a shell-words parser, and truncate tokens at the first `|`.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Handles single-quoted and double-quoted arguments.
  - Ignores everything after the first pipe token (`|`).
  - Does not require the first token to be executable; it is matched by basename only.
- **Validation**:
  - `cargo test -p api-testing-core cmd_snippet`

### Task 1.3: Parse GraphQL `call` snippets into report inputs
- **Location**:
  - `crates/api-testing-core/src/cmd_snippet.rs`
- **Description**: Add GraphQL parsing rules: accept `api-gql`/`gql.sh` basenames, optionally skip an explicit `call` token, extract `--config-dir`, `--env`, `--url`, `--jwt`, ignore `--no-history/--list-envs/--list-jwts`, and capture positional `operation` + optional `variables`.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Missing operation is rejected with a clear error.
  - Produces a derived case name when none is provided: `op_stem (env_or_url, jwt)` where meta includes `url` or env and optionally jwt.
- **Validation**:
  - `cargo test -p api-testing-core cmd_snippet`

### Task 1.4: Parse REST `call` snippets into report inputs
- **Location**:
  - `crates/api-testing-core/src/cmd_snippet.rs`
- **Description**: Add REST parsing rules: accept `api-rest`/`rest.sh` basenames, optionally skip an explicit `call` token, extract `--config-dir`, `--env`, `--url`, `--token`, ignore `--no-history`, and capture positional `request`.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 6
- **Acceptance criteria**:
  - Missing request path is rejected with a clear error.
  - Produces a derived case name when none is provided: `request_stem (env_or_url, token:name)` where meta includes `url` or env and optionally `token:name`.
- **Validation**:
  - `cargo test -p api-testing-core cmd_snippet`

### Task 1.5: Unit tests for tokenization + parsing + case derivation
- **Location**:
  - `crates/api-testing-core/src/cmd_snippet.rs`
- **Description**: Add focused unit tests covering quotes/escapes, `$AGENTS_HOME`-style paths (basename matching), `\\\n` continuations, pipe truncation, and common flag/positional combinations for both GraphQL and REST.
- **Dependencies**:
  - Task 1.3
  - Task 1.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Tests cover both ok and error cases for GraphQL and REST.
  - At least one test asserts that the first token’s directory prefix is ignored (basename-only match).
- **Validation**:
  - `cargo test -p api-testing-core cmd_snippet`

## Sprint 2: Add `report-from-cmd` to api-gql and api-rest
**Goal**: Provide user-facing subcommands that accept snippets and generate reports via existing report logic.
**Demo/Validation**:
- Command(s): `cargo test -p api-gql --test cli_smoke`, `cargo test -p api-rest --test cli_smoke`
- Verify: help text documents the feature; `--dry-run` prints an equivalent `report` command; invalid snippets fail fast.

### Task 2.1: Implement `api-gql report-from-cmd`
- **Location**:
  - `crates/api-gql/src/main.rs`
- **Description**: Add a `report-from-cmd` subcommand that reads a snippet from an argument or stdin, parses it via `api_testing_core::cmd_snippet`, converts it into `ReportArgs`, and invokes the existing `cmd_report`. Support `--case/--out/--response/--allow-empty/--dry-run/--stdin`.
- **Dependencies**:
  - Task 1.5
- **Complexity**: 7
- **Acceptance criteria**:
  - Accepts snippets whose first token is any path ending in `api-gql` or `gql.sh`.
  - `--dry-run` prints an equivalent `api-gql report ...` command and exits 0 (no network).
  - If `--response -` is used, stdin is reserved for the response body (snippet must be positional).
- **Validation**:
  - `cargo test -p api-gql --test cli_smoke`

### Task 2.2: Implement `api-rest report-from-cmd`
- **Location**:
  - `crates/api-rest/src/main.rs`
- **Description**: Add a `report-from-cmd` subcommand mirroring the GraphQL behavior, but for REST snippets (`api-rest`/`rest.sh`) and REST report args (`--request`, `--token`).
- **Dependencies**:
  - Task 1.5
- **Complexity**: 6
- **Acceptance criteria**:
  - Accepts snippets whose first token is any path ending in `api-rest` or `rest.sh`.
  - `--dry-run` prints an equivalent `api-rest report ...` command and exits 0 (no network).
  - If `--response -` is used, stdin is reserved for the response body (snippet must be positional).
- **Validation**:
  - `cargo test -p api-rest --test cli_smoke`

### Task 2.3: Extend CLI smoke tests for `report-from-cmd`
- **Location**:
  - `crates/api-gql/tests/cli_smoke.rs`
  - `crates/api-rest/tests/cli_smoke.rs`
- **Description**: Add minimal tests ensuring the new subcommand appears in `--help` and that `--dry-run` works on a simple snippet for each binary.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - `api-gql --help` output contains `report-from-cmd`.
  - `api-rest --help` output contains `report-from-cmd`.
  - `report-from-cmd --dry-run` exits 0 and prints `report` plus the derived `--case`.
- **Validation**:
  - `cargo test -p api-gql --test cli_smoke`
  - `cargo test -p api-rest --test cli_smoke`

## Sprint 3: Zsh completions + docs + required checks
**Goal**: Make the feature discoverable (completions/docs) and keep repo checks green.
**Demo/Validation**:
- Command(s): `zsh -f tests/zsh/completion.test.zsh`, `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify: completions source cleanly and mandatory checks pass.

### Task 3.1: Update zsh completions for new subcommands
- **Location**:
  - `completions/zsh/_api-gql`
  - `completions/zsh/_api-rest`
- **Description**: Add `report-from-cmd` to the subcommand list and implement flag completion for `--case/--out/--response/--allow-empty/--dry-run/--stdin`.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - `completions/zsh/_api-gql` offers `report-from-cmd` and its flags.
  - `completions/zsh/_api-rest` offers `report-from-cmd` and its flags.
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 3.2: Document `report-from-cmd` usage (README + usage guide)
- **Location**:
  - `README.md`
  - `crates/api-testing-core/README.md`
- **Description**: Add a short section showing how to pipe a history snippet into `report-from-cmd`, how `--dry-run` works, and how to use `--response` for offline report generation.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 3
- **Acceptance criteria**:
  - README includes at least one `api-gql report-from-cmd` and one `api-rest report-from-cmd` example.
  - Usage guide documents stdin behavior and the `--response -` stdin reservation rule.
- **Validation**:
  - `rg -n \"report-from-cmd\" README.md crates/api-testing-core/README.md`

### Task 3.3: Run mandatory checks (fmt, clippy, tests, zsh completion)
- **Location**:
  - `.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `DEVELOPMENT.md`
- **Description**: Run the repo’s mandatory checks and address any failures caused by this change.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 2
- **Acceptance criteria**:
  - All mandatory checks pass.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit: `api-testing-core` tests for normalization/tokenization/parsing and case derivation.
- Integration: `cli_smoke.rs` for `api-gql` and `api-rest` validates CLI help and `--dry-run` behavior.
- E2E/manual: run on a real repo layout by piping `api-*/history --command-only` into `api-* report-from-cmd`.

## Risks & gotchas
- Tokenization is intentionally best-effort; some shell edge cases may not parse (document as such).
- If users provide both a snippet via stdin and `--response -`, stdin is ambiguous; enforce a clear error.
- Path resolution for relative `setup/...` paths depends on finding a reasonable `project_root` (use config-dir inference + git-root search fallback).

## Rollback plan
- Revert the commits that add `report-from-cmd` and the shared parser module.
- Remove the new completion entries.
- No data migrations are involved; existing history/report behavior remains unchanged.
