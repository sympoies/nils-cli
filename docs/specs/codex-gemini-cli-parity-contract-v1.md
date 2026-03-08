# codex/gemini CLI parity contract v1

## Purpose

This document is the canonical parity contract for `nils-codex-cli` and `nils-gemini-cli` after core-crate consolidation into CLI adapters
plus `nils-common::provider_runtime`.

## Topology parity

Both binaries must expose the same top-level command topology:

- `agent`
- `auth`
- `diag`
- `config`
- `starship`
- `completion`

Shared help behavior:

- `--help`/`help` exit code remains `0`.
- Unknown groups/subcommands return deterministic usage errors (`64`) with lane-specific binary prefixes.

## Invalid-command parity

Non-canonical command invocations must preserve equal exit semantics between lanes:

- Unknown top-level groups
- Unknown subcommands under canonical groups

Parity requirement: for equivalent invocation shapes above, `codex-cli` and `gemini-cli` must return identical exit code classes.

## JSON contract parity

Structure parity is required while schema ids remain provider-specific.

- Auth command family:
  - Codex schema id prefix: `codex-cli.auth.v1`
  - Gemini schema id prefix: `gemini-cli.auth.v1`
- Diag command family:
  - Codex schema id prefix: `codex-cli.diag.rate-limits.v1`
  - Gemini schema id prefix: `gemini-cli.diag.rate-limits.v1`

Compatibility rules:

1. Required fields and envelope shape remain aligned across both lanes.
2. Provider labels and schema ids remain lane-specific.
3. No secret-bearing fields are emitted in error envelopes.

## Runtime adapter invariants

1. Provider-specific env key names, default model values, path precedence, and dangerous-exec command shape are configured via provider
   profiles.
2. Shared runtime primitives (`auth/json/jwt/error/path/config/exec` logic) stay in `nils-common::provider_runtime`.
3. Human output text and exit semantics stay stable for existing commands.

## Concurrency guardrails

### Gemini starship background-refresh parity

- `--refresh` remains the only blocking refresh path in both lanes. The default prompt path must keep printing the cached line first and
  keep appending the lane-specific stale suffix when the cache is stale.
- Current gap: `gemini-cli` still calls `refresh_blocking()` from the default stale/missing-cache path, while `codex-cli` enqueues a
  best-effort background refresh and returns immediately.
- Current Gemini missing-cache behavior is part of the same gap: no cache entry falls through to the same inline `refresh_blocking()`
  path instead of a detached enqueue.
- When porting Codex semantics to Gemini, preserve these guardrails:
  - Best-effort background spawn via the current executable; `current_exe` or spawn failures stay silent no-ops.
  - Minimum refresh interval throttling via `*_STARSHIP_REFRESH_MIN_SECONDS`.
  - Last-attempt markers are written before `spawn()` so prompt storms still throttle after child-launch failure.
  - Lock contention stays a no-op for prompt rendering, with stale-lock recovery via `*_STARSHIP_LOCK_STALE_SECONDS`.
  - Lock and throttle files stay cache-adjacent (`<cache-stem>.refresh.lock` and `<cache-stem>.refresh.at`).
  - Cache format, stale suffix rendering, and exit codes stay unchanged.
- Existing Gemini tests that must change when the default path stops refreshing inline:
  - `crates/gemini-cli/tests/starship_refresh.rs`: `starship_stale_cached_entry_refreshes_on_run`
  - `crates/gemini-cli/tests/starship_cached.rs`: `starship_stale_cache_with_failed_refresh_returns_0`
  - `crates/gemini-cli/tests/starship_cached.rs`: `starship_missing_cache_root_is_treated_as_no_cache`
- Coverage still missing after the port:
  - A default missing-cache test that asserts background enqueue instead of inline fetch.
  - Lock/min-interval/stale-lock recovery coverage that matches the existing `codex-cli` starship contract.
- `crates/gemini-cli/tests/starship_refresh.rs`: `starship_refresh_updates_cache` remains the blocking-path anchor and should stay
  unchanged by the background-refresh rewrite.

### Async rate-limits guardrails

- Shared async invariants:
  - `--async` still rejects `--one-line` and positional secret args with exit `64`.
  - `--async --cached -c` remains invalid; clear-cache failures remain exit `1`.
  - Async JSON keeps `schema_version`, `command="diag rate-limits"`, `mode="async"`, top-level `ok`, and a full `results` array.
  - Collection results stay deterministically sorted by `name`, never by completion order.
  - Command-level secret discovery failures remain top-level `error.code="secret-discovery-failed"`.
  - Per-secret failures in async JSON keep the full `results` array and return exit `1`.
  - `--jobs` remains non-fatal on zero/invalid input; later concurrency work must not introduce a new `invalid --jobs` usage error.
  - Missing-access-token fallback remains explicit: successful fallback emits `source="cache-fallback"` with `ok=true`; otherwise the
    result stays an error.
- JSON error-path invariants:
  - `codex-cli` command failures emit `error { code, message, details? }`; per-result failures use the same nested `error` envelope and
    may carry `details`.
  - `gemini-cli` command failures emit the same top-level `error { code, message, details? }`, but per-result failures currently
    serialize a nested `error { code, message }` object without per-result `details`.
  - Stable per-secret async JSON error codes are `missing-access-token`, `request-failed`, `invalid-usage-payload`, and
    `cache-read-failed`.
- Crate-specific differences to preserve during concurrency refactors:
  - `codex-cli` text async already uses bounded worker threads and honors `--jobs`, defaulting zero/invalid values to `5`; its watch mode
    is codex-only and still requires `--async`.
  - `codex-cli` async JSON is still sequential today; Sprint 4 should change only the execution strategy, not envelopes, ordering,
    fallback, or return codes.
  - `codex-cli` async JSON already honors `--cached`.
  - `gemini-cli` text async is sequential today because it delegates to `run_all_mode(...)`; `jobs` exists in `RateLimitsOptions` but is
    unused.
  - `gemini-cli` async JSON is sequential and keeps a `missing-access-token -> cache-fallback` special case.
  - `gemini-cli` text async honors `--cached`, but async JSON currently does not branch on `args.cached` and still fetches from the
    network. Treat that difference as part of the current observable contract until an intentional contract update says otherwise.

### Focused concurrency validation

- `cargo test -p nils-gemini-cli --test starship_cached --test starship_refresh`
- `cargo test -p nils-gemini-cli --test rate_limits_async --test rate_limits_network`
- `cargo test -p nils-codex-cli --test starship_cached --test starship_refresh`
- `cargo test -p nils-codex-cli --test rate_limits_async`

## Validation anchors

- `cargo test -p nils-codex-cli --test parity_oracle`
- `cargo test -p nils-gemini-cli --test parity_oracle`
- `cargo test -p nils-codex-cli --test runtime_auth_contract --test runtime_error_contract --test runtime_exec_contract --test runtime_paths_config_contract`
- `cargo test -p nils-gemini-cli --test runtime_auth_contract --test runtime_error_contract --test runtime_exec_contract --test runtime_paths_config_contract`
