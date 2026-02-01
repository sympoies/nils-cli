# Plan: Rust codex-cli parity (CLI + docs + tests)

## Overview
This plan ports the Zsh Codex feature set from `~/.config/zsh/scripts/_features/codex/` into a single Rust CLI crate in this workspace, named `codex-cli`. The goal is behavioral parity for the underlying features (auth/profile management, token refresh, rate-limit diagnostics, starship prompt output, and agent wrappers) while intentionally tightening the top-level interface: when no subcommand is provided, `codex-cli` prints help (no implicit “raw prompt mode” fallback). Zsh wrappers and completion will be provided for backwards-compatible entrypoints like `codex-use`, `codex-rate-limits`, and `cx*` aliases.

## Scope
- In scope: A new `codex-cli` Rust binary implementing all existing feature behaviors: agent prompt wrappers, auth/profile management, rate-limits diagnostics (single/all/async/cached), starship prompt output, config show/set behavior, and compatibility wrappers + Zsh completion. Comprehensive tests (unit + integration + Zsh completion) anchored to docs fixtures. Delivery requirement: `codex-cli` crate line coverage **>= 80.00%** (measured by `cargo llvm-cov nextest --profile ci -p codex-cli --lcov --output-path target/coverage/codex-cli.lcov.info --fail-under-lines 80`).
- Out of scope: Implementing OpenAI's `codex` CLI itself, changing network APIs or authentication semantics, encrypting secrets at rest, and making persistent changes to user shell state without an explicit wrapper/eval/export contract.

## Assumptions (if any)
1. The external `codex` CLI is available on PATH for agent commands (prompt/advice/knowledge/commit); when missing, the Rust CLI will surface a clear error consistent with the existing Zsh behavior.
2. Secret/profile files remain JSON and follow the observed schema patterns used by the Zsh scripts (`tokens.*`, optional `account_id`, `last_refresh`, and optional `.codex_rate_limits.*` writeback fields).
3. `CODEX_CHATGPT_BASE_URL` remains supported for directing `wham/usage` to a test server (used heavily by the test suite); the default remains `https://chatgpt.com/backend-api/`.

## Subcommands (proposed for review)

Binary name: `codex-cli`

- `codex-cli agent prompt [PROMPT...]`: Run a raw prompt via `codex exec` (dangerous mode; gated by `CODEX_ALLOW_DANGEROUS_ENABLED=true`).
- `codex-cli agent advice [QUESTION]`: Run the `actionable-advice` prompt template with `$ARGUMENTS` substitution.
- `codex-cli agent knowledge [CONCEPT]`: Run the `actionable-knowledge` prompt template with `$ARGUMENTS` substitution.
- `codex-cli agent commit [-p|--push] [-a|--auto-stage] [EXTRA_PROMPT...]`: Run the semantic-commit workflow prompt (staged vs autostage templates); falls back to a local Conventional Commit flow if `semantic-commit` is not available.

- `codex-cli auth use PROFILE_OR_EMAIL`: Switch `CODEX_AUTH_FILE` by applying a secret under `CODEX_SECRET_DIR` (supports resolving by email local-part).
- `codex-cli auth refresh [SECRET_JSON]`: Refresh OAuth tokens via `refresh_token` (default: active `CODEX_AUTH_FILE`).
- `codex-cli auth auto-refresh`: Refresh stale tokens across auth + secrets (timestamp-based, configurable min age).
- `codex-cli auth current`: Print which secret matches the active `CODEX_AUTH_FILE` (identity/hash matching).
- `codex-cli auth sync`: Sync `CODEX_AUTH_FILE` back into matching secrets under `CODEX_SECRET_DIR`.

- `codex-cli diag rate-limits [OPTIONS] [SECRET_JSON]`: Fetch and render Codex rate limits; supports `--json`, `--one-line`, `--all`, `--async`, `--cached`, `--jobs`, `-c`, `-d`, and `--no-refresh-auth`.

- `codex-cli config show`: Print effective configuration and detected paths (no mutation).
- `codex-cli config set KEY VALUE`: Print shell export(s) needed to update config in the current shell (no direct parent-shell mutation).

- `codex-cli starship [--no-5h] [--ttl DURATION] [--time-format STRFTIME] [--refresh] [--is-enabled]`: Print Starship-ready rate-limit summary (stale-while-revalidate caching, silent failure when disabled).

## Sprint 1: Inventory, parity spec, and fixtures
**Goal**: Make the current Zsh behaviors explicit and create a fixtures/edge-case matrix that tests can anchor to.
**Demo/Validation**:
- Command(s): `rg -n \"codex-tools\\(\\)|codex-use\\(|codex-refresh-auth\\(|codex-rate-limits\\(|codex-starship\\(\" ~/.config/zsh/scripts/_features/codex/*.zsh`
- Verify: `docs/codex-cli/spec.md` and `docs/codex-cli/fixtures.md` fully enumerate commands, flags, outputs, and edge cases.

### Task 1.1: Write codex-cli parity spec
- **Location**:
  - `docs/codex-cli/spec.md`
- **Description**: Read `~/.config/zsh/scripts/_features/codex/` scripts and document the Rust `codex-cli` CLI contract: subcommands, flag parsing, help text, exit codes, output strings, env var behavior, cache/writeback semantics, and shell-integration limitations (notably `config set`).
- **Dependencies**:
  - none
- **Complexity**: 6
- **Acceptance criteria**:
  - Spec lists all subcommands and documents that `codex-cli` prints help when no subcommand is provided (no top-level prompt fallback).
  - Spec documents all env vars referenced by the Zsh implementation (agent/auth/diag/starship).
  - Spec documents on-disk side effects: which files are read/written, permissions, and cache paths.
  - Spec captures exit codes and stderr/stdout behavior for usage errors, missing files, and ambiguous secret selection.
- **Validation**:
  - `rg -n \"^# codex-cli parity spec\" docs/codex-cli/spec.md`

### Task 1.2: Define codex-cli fixtures and edge-case matrix
- **Location**:
  - `docs/codex-cli/fixtures.md`
- **Description**: Define deterministic fixture scenarios (auth.json, secrets/*.json, cache kv files, stub HTTP responses) and a full edge-case matrix covering: missing tools (`codex`, `git`), missing/invalid JSON, ambiguous profile resolution, 401 refresh-and-retry, `--cached` behavior, `--all` table rendering, `--async` concurrency, starship stale suffix, and `NO_COLOR` output.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Fixtures cover every `codex-cli` subcommand at least once.
  - Fixtures include at least one scenario each for `--all`, `--async`, and `--cached` rate limit modes.
  - Fixtures include at least one scenario for starship cached output and starship refresh behavior.
- **Validation**:
  - `rg -n \"^# codex-cli fixtures\" docs/codex-cli/fixtures.md`

## Sprint 2: Crate scaffold and CLI surface (dispatch + exit codes)
**Goal**: Add a `codex-cli` crate with stable CLI parsing and parity for the dispatcher behaviors (help, legacy guidance, and exit codes).
**Demo/Validation**:
- Command(s): `cargo run -p codex-cli -- --help`
- Verify: help output lists command groups; `cargo test -p codex-cli` runs.

### Task 2.1: Create codex-cli crate and workspace wiring
- **Location**:
  - `Cargo.toml`
  - `crates/codex-cli/Cargo.toml`
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/src/cli.rs`
- **Description**: Add a new Rust binary crate named `codex-cli`, register it as a workspace member, and implement a clap CLI skeleton that mirrors the Zsh command groups.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `cargo run -p codex-cli -- --help` succeeds and lists `agent`, `auth`, `diag`, `config`, and `starship`.
  - `codex-cli` (no args) prints help and exits 0.
  - Unknown commands are treated as usage errors (no implicit raw prompt behavior).
- **Validation**:
  - `cargo run -p codex-cli -- --help | rg \"codex-cli\"`
  - `cargo metadata --no-deps | rg '\"name\": \"codex-cli\"'`

### Task 2.2: Implement legacy guidance messages (redirects) and `list` behavior
- **Location**:
  - `crates/codex-cli/src/main.rs`
  - `crates/codex-cli/tests/dispatch.rs`
- **Description**: Implement parity-friendly guardrails for legacy command words that previously existed at the top-level of `codex-tools`: `list` guidance (exit 64) and explicit redirects for `prompt`, `advice`, `knowledge`, `commit`, `auto-refresh`, and `rate-limits` that print the recommended subcommand path and exit 64.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - `codex-cli list` exits 64 and prints the guidance line to stderr.
  - `codex-cli prompt` exits 64 and points to `codex-cli agent prompt`.
  - `codex-cli rate-limits` exits 64 and points to `codex-cli diag rate-limits`.
- **Validation**:
  - `cargo test -p codex-cli dispatch`

## Sprint 3: Auth + secrets (use/refresh/auto-refresh/current/sync)
**Goal**: Port secret management and token refresh behaviors with deterministic tests.
**Demo/Validation**:
- Command(s): `cargo test -p codex-cli auth_`
- Verify: auth subcommands match spec for output, exit codes, timestamps, and file permissions.

### Task 3.1: Implement path discovery and JSON/JWT helpers
- **Location**:
  - `crates/codex-cli/src/paths.rs`
  - `crates/codex-cli/src/json.rs`
  - `crates/codex-cli/src/jwt.rs`
  - `crates/codex-cli/src/fs.rs`
  - `crates/codex-cli/src/auth/mod.rs`
- **Description**: Implement shared helpers used by auth/rate-limits/starship: resolving `CODEX_SECRET_DIR`, `CODEX_AUTH_FILE`, and `CODEX_SECRET_CACHE_DIR`; computing SHA-256 file hashes; decoding JWT payloads; extracting identity/email/account_id; and performing safe atomic file writes with 0600 permissions where applicable.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Helpers replicate Zsh resolution precedence for auth file and secrets directory.
  - JWT decoding supports base64url tokens and extracts identity/email fields used by the Zsh scripts.
  - File writes are atomic and permissions are set to 0600 on secret JSON outputs where the Zsh scripts do so.
- **Validation**:
  - `cargo test -p codex-cli jwt_`

### Task 3.2: Implement auth current and auth sync
- **Location**:
  - `crates/codex-cli/src/auth/current.rs`
  - `crates/codex-cli/src/auth/sync.rs`
  - `crates/codex-cli/tests/auth_current_sync.rs`
- **Description**: Implement `codex-cli auth current` (match active auth file to a secret by identity key and/or exact hash) and `codex-cli auth sync` (sync auth file back to all matching secrets). Mirror Zsh stdout/stderr messages and exit codes, and write timestamp sidecar files in `CODEX_SECRET_CACHE_DIR`.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 7
- **Acceptance criteria**:
  - When `CODEX_AUTH_FILE` matches a secret hash exactly, output matches the Zsh `exact` message.
  - When identity matches but hash differs, output includes the `identity; secret differs` message and exits 0.
  - `auth current` exits 2 when no secret matches, and exits 1 for missing auth file.
  - `auth sync` updates matching secret JSON content and timestamp files, and leaves non-matching secrets untouched.
- **Validation**:
  - `cargo test -p codex-cli auth_current_`
  - `cargo test -p codex-cli auth_sync_`

### Task 3.3: Implement auth use (profile switching)
- **Location**:
  - `crates/codex-cli/src/auth/use_secret.rs`
  - `crates/codex-cli/tests/auth_use.rs`
- **Description**: Implement `codex-cli auth use PROFILE_OR_EMAIL` with the same validation rules as `codex-use`: reject path traversal, resolve by `name.json` or email local-part/full email, handle ambiguity as exit 2, and apply the chosen secret to `CODEX_AUTH_FILE` (syncing current auth first when present).
- **Dependencies**:
  - Task 3.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Invalid input (empty, extra args, path traversal) exits 64 with the Zsh usage/error strings.
  - Resolving by email local-part works and ambiguous resolution exits 2 with the candidates line.
  - Applying a secret updates `CODEX_AUTH_FILE` and writes/updates the timestamp sidecar.
- **Validation**:
  - `cargo test -p codex-cli auth_use_`

### Task 3.4: Implement auth refresh (OAuth refresh_token flow)
- **Location**:
  - `crates/codex-cli/src/auth/refresh.rs`
  - `crates/codex-cli/tests/auth_refresh.rs`
- **Description**: Implement `codex-cli auth refresh` by calling the OAuth token endpoint using the refresh token from the target JSON, merging returned token fields into `.tokens`, updating `.last_refresh`, updating timestamp files, and syncing back to secrets when the refreshed target is `CODEX_AUTH_FILE`. Support the Zsh timeout env vars for connect/max-time.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Missing refresh token exits 2 with the same error prefix as the Zsh script.
  - Non-200 responses include an error summary when present and exit 3.
  - Successful refresh prints `codex: refreshed ... at ...` and updates timestamps.
  - Refreshing `CODEX_AUTH_FILE` triggers a sync to matching secrets.
- **Validation**:
  - `cargo test -p codex-cli auth_refresh_`

### Task 3.5: Implement auth auto-refresh (stale token refresh across auth + secrets)
- **Location**:
  - `crates/codex-cli/src/auth/auto_refresh.rs`
  - `crates/codex-cli/tests/auth_auto_refresh.rs`
- **Description**: Implement `codex-cli auth auto-refresh` parity with `codex-auto-refresh`: determine staleness using timestamp files and/or `.last_refresh`, refresh stale targets (auth file plus secrets), and print the summary line when invoked directly or when work was performed. Respect `CODEX_AUTO_REFRESH_ENABLED` and `CODEX_AUTO_REFRESH_MIN_DAYS` behavior.
- **Dependencies**:
  - Task 3.4
- **Complexity**: 8
- **Acceptance criteria**:
  - Invalid `CODEX_AUTO_REFRESH_MIN_DAYS` exits 64 with the Zsh error string.
  - Stale detection backfills timestamp files from `.last_refresh` when needed.
  - Summary output matches the Zsh format `refreshed=X skipped=Y failed=Z (min_age_days=N)`.
- **Validation**:
  - `cargo test -p codex-cli auth_auto_refresh_`

## Sprint 4: Rate-limits diagnostics (single/all/async/cached)
**Goal**: Port `codex-rate-limits` and `codex-rate-limits-async` behaviors under `codex-cli diag rate-limits`.
**Demo/Validation**:
- Command(s): `cargo test -p codex-cli rate_limits_`
- Verify: output formats, cache usage, writeback behavior, and concurrency modes match the fixtures.

### Task 4.1: Implement wham usage client (HTTP + retry on 401)
- **Location**:
  - `crates/codex-cli/src/rate_limits/client.rs`
  - `crates/codex-cli/src/rate_limits/mod.rs`
  - `crates/codex-cli/tests/rate_limits_client.rs`
- **Description**: Implement the `wham/usage` fetch logic using `access_token` (and optional `account_id`) from the target JSON. Support `CODEX_CHATGPT_BASE_URL`, timeouts, and `--no-refresh-auth`. On 401, refresh tokens and retry once when refresh is enabled.
- **Dependencies**:
  - Task 3.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Requests include Authorization and optional ChatGPT-Account-Id header (when present).
  - 401 triggers refresh+retry exactly once when enabled; when disabled, 401 exits 3 without retry.
  - Non-200 responses include a short body preview line (first 200 bytes) similar to the Zsh output.
- **Validation**:
  - `cargo test -p codex-cli rate_limits_client_`

### Task 4.2: Implement single-account render modes, cache, and writeback
- **Location**:
  - `crates/codex-cli/src/rate_limits/render.rs`
  - `crates/codex-cli/src/rate_limits/writeback.rs`
  - `crates/codex-cli/src/rate_limits/cache.rs`
  - `crates/codex-cli/src/ansi.rs`
  - `crates/codex-cli/tests/rate_limits_single.rs`
- **Description**: Implement output modes for single targets: human summary, one-line summary, and raw JSON. Implement starship cache write/read (kv format) and `--cached` mode semantics. Implement weekly reset metadata writeback into `.codex_rate_limits` fields and the `-c` cache-clear safety checks. Re-implement the Night Owl percent coloring used by `ansi_theme_night_owl::format_percent_cell/token` in Rust.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 8
- **Acceptance criteria**:
  - `--json` and `--one-line` are mutually exclusive with the same exit code behavior as the Zsh script.
  - Successful fetch writes `.codex_rate_limits.weekly_reset_at_epoch` and related fields into the target JSON.
  - `--cached` reads from the starship cache and performs no network calls.
  - `NO_COLOR=1` disables ANSI output consistently across rate-limits renderers.
- **Validation**:
  - `cargo test -p codex-cli rate_limits_single_`

### Task 4.3: Implement all-accounts table rendering (sequential)
- **Location**:
  - `crates/codex-cli/src/rate_limits/all.rs`
  - `crates/codex-cli/tests/rate_limits_all.rs`
- **Description**: Implement `--all` behavior: iterate all secrets under `CODEX_SECRET_DIR`, compute per-account results (using cached mode when requested), aggregate into a sorted table, compute the `Left` cells from reset epochs, and match the Zsh fixed-width layout and sorting rules.
- **Dependencies**:
  - Task 4.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Output includes the `🚦 Codex rate limits for all accounts` header and the fixed-width table columns.
  - Table sorts by weekly reset epoch when available and places missing epochs at the end.
  - In cached mode, missing cache entries do not force a non-zero exit code.
- **Validation**:
  - `cargo test -p codex-cli rate_limits_all_`

### Task 4.4: Implement async all-accounts mode (jobs-limited concurrency)
- **Location**:
  - `crates/codex-cli/src/rate_limits/async_mode.rs`
  - `crates/codex-cli/tests/rate_limits_async.rs`
- **Description**: Implement `--async` behavior: run per-secret fetches concurrently (jobs-limited), capture per-account stderr for `--debug`, and render the same table output as sequential `--all`. Implement `--jobs` parsing and validation rules, and mirror the Zsh exit code semantics.
- **Dependencies**:
  - Task 4.3
- **Complexity**: 9
- **Acceptance criteria**:
  - `--jobs` accepts positive integers and rejects invalid values with exit 64.
  - `--debug` emits the captured per-account stderr blocks after the table.
  - Exit code is 1 when any non-cached per-account call fails; cached mode stays 0 when data is missing.
- **Validation**:
  - `cargo test -p codex-cli rate_limits_async_`

## Sprint 5: Starship prompt output (stale-while-revalidate)
**Goal**: Port `codex-starship` behaviors into `codex-cli starship` and ensure cache compatibility with rate-limits.
**Demo/Validation**:
- Command(s): `cargo test -p codex-cli starship_`
- Verify: cached output formatting, TTL parsing, and refresh locking behaviors match fixtures.

### Task 5.1: Implement starship output rendering and flag parsing
- **Location**:
  - `crates/codex-cli/src/starship/mod.rs`
  - `crates/codex-cli/src/starship/render.rs`
  - `crates/codex-cli/tests/starship_cached.rs`
- **Description**: Implement `codex-cli starship` with parity flags (`--no-5h`, `--ttl`, `--time-format`, `--is-enabled`). Render cached output immediately (even stale), append stale suffix when appropriate, and respect `CODEX_STARSHIP_ENABLED` and `NO_COLOR` behavior. Implement name selection rules (`secret` vs `email` sources and fallback name behavior).
- **Dependencies**:
  - Task 3.1
  - Task 4.2
- **Complexity**: 7
- **Acceptance criteria**:
  - When `CODEX_STARSHIP_ENABLED` is false, the command prints nothing and exits 0.
  - `--is-enabled` exits 0 only when enabled, otherwise exits 1.
  - `--ttl` accepts numeric seconds and `s/m/h/d/w` suffix durations; invalid values exit 2 and print usage.
  - Output matches the documented starship line formats for `--no-5h` and default mode.
- **Validation**:
  - `cargo test -p codex-cli starship_cached_`

### Task 5.2: Implement starship refresh, locks, and background enqueue
- **Location**:
  - `crates/codex-cli/src/starship/refresh.rs`
  - `crates/codex-cli/src/starship/lock.rs`
  - `crates/codex-cli/tests/starship_refresh.rs`
- **Description**: Implement stale-while-revalidate: on stale cache (or missing cache), enqueue a detached refresh when outside the configured min interval, while still returning cached output immediately. Implement `--refresh` for blocking refresh, lock acquisition with stale lock cleanup, and best-effort cleanup of stale temp usage files.
- **Dependencies**:
  - Task 5.1
  - Task 4.1
- **Complexity**: 9
- **Acceptance criteria**:
  - In normal mode, stale cache prints immediately with stale suffix and triggers a background refresh attempt.
  - In `--refresh` mode, cache is updated and the fresh output is printed (or the command stays silent on failures, matching Zsh semantics).
  - Locking prevents concurrent refreshes for the same cache key and recovers from stale locks.
- **Validation**:
  - `cargo test -p codex-cli starship_refresh_`

## Sprint 6: Agent commands (codex exec wrappers + prompt templates)
**Goal**: Port `codex-cli agent` behaviors, prompt templates, and the semantic commit workflow.
**Demo/Validation**:
- Command(s): `cargo test -p codex-cli agent_`
- Verify: dangerous-mode gating and `codex exec` invocation match the Zsh implementation.

### Task 6.1: Implement agent prompt and dangerous-mode gating
- **Location**:
  - `crates/codex-cli/src/agent/mod.rs`
  - `crates/codex-cli/src/agent/exec.rs`
  - `crates/codex-cli/tests/agent_prompt.rs`
- **Description**: Implement `codex-cli agent prompt` and the shared `codex exec` wrapper. Enforce `CODEX_ALLOW_DANGEROUS_ENABLED=true` gating with the same stderr messages as the Zsh helper, and pass the configured model/reasoning flags (`CODEX_CLI_MODEL`, `CODEX_CLI_REASONING`) to `codex exec` using the same argument structure.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - When dangerous mode is disabled, agent commands exit 1 and print the `disabled (set CODEX_ALLOW_DANGEROUS_ENABLED=true)` message.
  - When enabled, the wrapper executes `codex exec` with `--dangerously-bypass-approvals-and-sandbox -s workspace-write`.
  - `CODEX_CLI_MODEL` and `CODEX_CLI_REASONING` are applied to the `codex exec` invocation.
- **Validation**:
  - `cargo test -p codex-cli agent_prompt_`

### Task 6.2: Implement prompt template lookup and advice/knowledge commands
- **Location**:
  - `crates/codex-cli/src/prompts.rs`
  - `crates/codex-cli/tests/agent_templates.rs`
- **Description**: Implement prompt template resolution matching the Zsh lookup rules (ZDOTDIR prompts directory with feature-dir fallback). Implement `$ARGUMENTS` substitution and `codex-cli agent advice` / `codex-cli agent knowledge` command behavior.
- **Dependencies**:
  - Task 6.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Missing templates produce the same error prefix as the Zsh scripts and exit non-zero.
  - `$ARGUMENTS` substitution behaves identically for single and multi-word inputs.
  - Empty input triggers the interactive prompt behavior or a clear error consistent with the spec.
- **Validation**:
  - `cargo test -p codex-cli agent_templates_`

### Task 6.3: Implement agent commit workflow and fallback mode
- **Location**:
  - `crates/codex-cli/src/agent/commit.rs`
  - `crates/codex-cli/tests/agent_commit.rs`
- **Description**: Implement `codex-cli agent commit` with `--push` and `--auto-stage` flags. Match the Zsh behavior: verify git repo, enforce staged changes (or autostage), use the semantic-commit prompt template when `semantic-commit` is available, and fall back to an interactive Conventional Commit flow when it is not.
- **Dependencies**:
  - Task 6.2
- **Complexity**: 9
- **Acceptance criteria**:
  - In fallback mode, the command prompts for type/scope/subject and commits via `git commit -m`.
  - With `--push`, the command pushes after committing in both modes.
  - When `git-scope` is present, the command prints scope context similar to the Zsh fallback; otherwise it prints staged files.
- **Validation**:
  - `cargo test -p codex-cli agent_commit_`

## Sprint 7: Zsh integration (wrappers + completion) and delivery checks
**Goal**: Provide backwards-compatible entrypoints, Zsh completion, and ensure all required checks pass.
**Demo/Validation**:
- Command(s): `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify: fmt, clippy, tests, and Zsh completion tests all pass.

### Task 7.1: Add compatibility wrappers for legacy commands and aliases
- **Location**:
  - `wrappers/codex-cli`
  - `wrappers/codex-use`
  - `wrappers/codex-refresh-auth`
  - `wrappers/codex-auto-refresh`
  - `wrappers/codex-rate-limits`
  - `wrappers/codex-rate-limits-async`
  - `wrappers/codex-starship`
  - `wrappers/cx`
  - `wrappers/cxgp`
  - `wrappers/cxga`
  - `wrappers/cxgk`
  - `wrappers/cxgc`
  - `wrappers/cxau`
  - `wrappers/cxar`
  - `wrappers/cxaa`
  - `wrappers/cxac`
  - `wrappers/cxas`
  - `wrappers/cxdr`
  - `wrappers/cxcs`
  - `wrappers/cxct`
  - `wrappers/crl`
  - `wrappers/crla`
- **Description**: Add thin wrapper scripts that `exec` `codex-cli` with the correct subcommand mapping, preserving the alias behaviors in `~/.config/zsh/scripts/_features/codex/alias.zsh` while keeping a single underlying Rust binary.
- **Dependencies**:
  - Task 6.3
  - Task 5.2
  - Task 4.4
  - Task 3.5
- **Complexity**: 4
- **Acceptance criteria**:
  - Wrapper names map to the same command paths as the Zsh aliases (`cx*` and `crl/crla`).
  - Wrappers preserve argv passthrough and exit codes from the underlying binary.
  - Wrapper scripts are POSIX-safe and use `exec` to avoid extra processes.
- **Validation**:
  - `rg -n \"exec .*codex-cli\" wrappers/cx`

### Task 7.2: Add Zsh completion for codex-cli and update the completion test
- **Location**:
  - `completions/zsh/_codex-cli`
  - `tests/zsh/completion.test.zsh`
- **Description**: Add a Zsh completion file defining `_codex-cli` and registering `compdef` for `codex-cli` and key wrapper names. Update `tests/zsh/completion.test.zsh` to source the file and assert the function exists and includes key subcommands in the completion list.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - `tests/zsh/completion.test.zsh` passes and asserts `agent`, `auth`, and `diag` subcommands appear in completion.
  - Completion includes `diag rate-limits` option flags (at least `--all`, `--async`, and `--cached`).
- **Validation**:
  - `zsh -f tests/zsh/completion.test.zsh`

### Task 7.3: Add codex-cli docs for installation and migration
- **Location**:
  - `docs/codex-cli/README.md`
- **Description**: Document how to install and use the Rust `codex-cli` binary, how wrappers map from legacy Zsh commands, recommended Zsh/Starship integration, and how to apply `config set` output in a parent shell.
- **Dependencies**:
  - Task 1.1
  - Task 7.1
  - Task 7.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Docs include example invocations for each command group and at least one full end-to-end flow (use -> refresh -> rate-limits -> starship).
  - Docs include the complete wrapper mapping table for `cx*` and `crl/crla`.
  - Docs call out limitations where parent-shell mutation is required and document the eval/export contract.
- **Validation**:
  - `rg -n \"^# codex-cli\" docs/codex-cli/README.md`

### Task 7.4: Run required formatting, lint, and test gates
- **Location**:
  - `DEVELOPMENT.md`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- **Description**: Run the repo-required gates (fmt, clippy, workspace tests, Zsh completion tests, and coverage) and fix any failures until all checks pass. In addition to the repo coverage policy (>= 70% workspace), require `codex-cli` crate coverage >= 80%.
- **Dependencies**:
  - Task 7.3
- **Complexity**: 3
- **Acceptance criteria**:
  - `cargo fmt --all -- --check` passes.
  - `cargo clippy --all-targets --all-features -- -D warnings` passes.
  - `cargo test --workspace` passes.
  - `zsh -f tests/zsh/completion.test.zsh` passes.
  - `cargo llvm-cov nextest --profile ci -p codex-cli --lcov --output-path target/coverage/codex-cli.lcov.info --fail-under-lines 80` passes.
  - `scripts/ci/coverage-summary.sh target/coverage/codex-cli.lcov.info` reports total line coverage >= 80.00%.
- **Validation**:
  - `cargo llvm-cov nextest --profile ci -p codex-cli --lcov --output-path target/coverage/codex-cli.lcov.info --fail-under-lines 80`
  - `scripts/ci/coverage-summary.sh target/coverage/codex-cli.lcov.info`
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Testing Strategy
- Unit: Parsing helpers (JWT decoding, TTL parsing, cache key normalization), render formatting (percent color mapping), and path resolution precedence.
- Integration: End-to-end CLI tests using temp directories for auth/secrets/cache, stub HTTP servers for `wham/usage` and token refresh, and PATH-stubbed external binaries for `codex`, `git`, and `semantic-commit`.
- E2E/manual: Starship integration in a real shell, verifying stale-while-revalidate behavior and that `NO_COLOR` and `CODEX_STARSHIP_ENABLED` gating behave as expected.
- Coverage: enforce `codex-cli` crate line coverage >= 80.00% via `cargo llvm-cov nextest --profile ci -p codex-cli ... --fail-under-lines 80`.

## Risks & gotchas
- Auth/token endpoints and JSON schemas may evolve; keep endpoint URLs and request formats configurable for tests while defaulting to current production endpoints.
- Parent-shell mutation: `config set` cannot mutate the caller environment directly; this requires a documented eval/export contract and wrapper guidance.
- Concurrency and caches: async rate limit queries and starship background refresh need robust locking to avoid corrupt caches or excessive requests.
- Output parity: fixed-width tables and percent coloring must match the Zsh scripts (including whitespace, sorting, and stale suffix formatting).

## Rollback plan
- Keep the existing Zsh feature scripts installed and sourceable as the immediate fallback.
- Ship wrappers in a way that allows switching back to the Zsh implementations by adjusting PATH precedence (for example, prioritize the old scripts directory) without deleting any files.
- If regressions are found, remove the `codex-cli` crate from the workspace and delete the wrappers/completions added in this plan; no repository data migrations are required.
