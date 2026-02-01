# codex-cli

Rust port of the Zsh Codex helpers (`codex-tools`, `codex-use`, `codex-rate-limits`, `codex-starship`, etc.) for the `nils-cli` workspace.

## Install

- Build the workspace: `cargo build`
- Run help: `cargo run -p codex-cli -- --help`

For a local release install (all workspace binaries), follow `DEVELOPMENT.md`:

- `./.codex/skills/nils-cli-install/scripts/nils-cli-install.sh`

## Quickstart

Agent commands are **dangerous-mode gated** (they call `codex exec --dangerously-bypass-approvals-and-sandbox`).

```sh
export CODEX_ALLOW_DANGEROUS_ENABLED=true
codex-cli agent advice "How do I debug a flaky test?"
```

End-to-end flow (auth -> refresh -> rate-limits -> starship):

```sh
codex-cli auth use work
codex-cli auth refresh
codex-cli diag rate-limits --one-line
export CODEX_STARSHIP_ENABLED=true
codex-cli starship --refresh
```

## Command groups

### `codex-cli agent`

- `codex-cli agent prompt [PROMPT...]`
- `codex-cli agent advice [QUESTION...]`
- `codex-cli agent knowledge [CONCEPT...]`
- `codex-cli agent commit [-p|--push] [-a|--auto-stage] [EXTRA_PROMPT...]`

Notes:

- Requires `CODEX_ALLOW_DANGEROUS_ENABLED=true`.
- Requires the external `codex` CLI on `PATH` (the Rust wrapper shells out to `codex exec`).

### `codex-cli auth`

- `codex-cli auth use <profile|email>`
- `codex-cli auth refresh [secret.json]`
- `codex-cli auth auto-refresh`
- `codex-cli auth current`
- `codex-cli auth sync`

### `codex-cli diag`

- `codex-cli diag rate-limits [OPTIONS] [secret.json]`

Useful flags:

- `--one-line` (single-line output; also used by `--cached`)
- `--all` (table for all secrets)
- `--async` (concurrent all-secrets mode)
- `--cached` (no network; implies `--one-line`)

### `codex-cli config`

Because a child process cannot mutate the parent shell environment, `config set` prints a shell snippet.

- Show effective values:

  ```sh
  codex-cli config show
  ```

- Set a value in your current shell:

  ```sh
  eval "$(codex-cli config set model gpt-5.1-codex-mini)"
  eval "$(codex-cli config set reasoning medium)"
  eval "$(codex-cli config set dangerous true)"
  ```

### `codex-cli starship`

- Enable:

  ```sh
  export CODEX_STARSHIP_ENABLED=true
  ```

- Use from Starship:

  ```toml
  [custom.codex]
  command = "codex-cli starship"
  when = "true"
  ```

## Zsh wrappers and migration

The repo ships thin wrapper scripts under `wrappers/` to preserve legacy command names.

### Wrapper mapping

| Wrapper | Runs |
|---|---|
| `codex-use` | `codex-cli auth use` |
| `codex-refresh-auth` | `codex-cli auth refresh` |
| `codex-auto-refresh` | `codex-cli auth auto-refresh` |
| `codex-rate-limits` | `codex-cli diag rate-limits` |
| `codex-rate-limits-async` | `codex-cli diag rate-limits --async` |
| `codex-starship` | `codex-cli starship` |
| `cx` | `codex-cli` |
| `cxgp` | `codex-cli agent prompt` |
| `cxga` | `codex-cli agent advice` |
| `cxgk` | `codex-cli agent knowledge` |
| `cxgc` | `codex-cli agent commit` |
| `cxau` | `codex-cli auth use` |
| `cxar` | `codex-cli auth refresh` |
| `cxaa` | `codex-cli auth auto-refresh` |
| `cxac` | `codex-cli auth current` |
| `cxas` | `codex-cli auth sync` |
| `cxdr` | `codex-cli diag rate-limits` |
| `cxcs` | `codex-cli config show` |
| `cxct` | `codex-cli config set` |
| `crl` | `codex-cli diag rate-limits` |
| `crla` | `codex-cli diag rate-limits --async` |

### Zsh completion

- Completion file: `completions/zsh/_codex-cli`
- Setup:
  - Add `wrappers/` to `PATH`
  - Add `completions/zsh/` to `fpath` and run `compinit`

# codex-cli parity spec

## Scope and sources

This spec documents the Rust `codex-cli` contract based on the current Zsh implementation in:

- `https://github.com/graysurf/zsh-kit/blob/main/scripts/_features/codex/codex-tools.zsh`
- `https://github.com/graysurf/zsh-kit/blob/main/scripts/_features/codex/codex-secret.zsh`
- `https://github.com/graysurf/zsh-kit/blob/main/scripts/_features/codex/codex-auto-refresh.zsh`
- `https://github.com/graysurf/zsh-kit/blob/main/scripts/_features/codex/codex-starship.zsh`
- `https://github.com/graysurf/zsh-kit/blob/main/scripts/_features/codex/alias.zsh`

Parity goal: match behavior, messages, exit codes, and side effects unless explicitly called out.

**Intentional divergence** (required by plan):

- `codex-cli` prints help and exits 0 when no subcommand is provided.
- No top-level prompt fallback. Unknown top-level commands are treated as usage errors (exit 64) rather than raw prompts.

## Command surface

Top-level:

- `codex-cli agent <command> [args...]`
- `codex-cli auth <command> [args...]`
- `codex-cli diag <command> [args...]`
- `codex-cli config <command> [args...]`
- `codex-cli starship [options]`

Legacy Zsh aliases (wrappers will map to these command paths):

- `cx` -> `codex-cli`
- `cxg*` -> `codex-cli agent ...`
- `cxa*` -> `codex-cli auth ...`
- `cxd*` -> `codex-cli diag ...`
- `cxc*` -> `codex-cli config ...`
- `crl` -> `codex-cli diag rate-limits`
- `crla` -> `codex-cli diag rate-limits --async` (or `rate-limits-async` wrapper)

## Environment variables

Agent / config:

- `CODEX_CLI_MODEL` (default `gpt-5.1-codex-mini`) - passed to `codex exec -m`.
- `CODEX_CLI_REASONING` (default `medium`) - passed to `codex exec -c model_reasoning_effort=...`.
- `CODEX_ALLOW_DANGEROUS_ENABLED` - must be `true` to run agent commands.

Auth / secrets:

- `CODEX_SECRET_DIR` - secrets directory (default: `<feature_dir>/secrets`).
- `CODEX_AUTH_FILE` - active auth file (default: `$HOME/.codex/auth.json`).
- `CODEX_SECRET_CACHE_DIR` - timestamp cache (default: `${ZSH_CACHE_DIR:-${ZDOTDIR:-$HOME/.config/zsh}/cache}/codex/secrets`).
- `CODEX_OAUTH_CLIENT_ID` (default `app_EMoamEEZ73f0CkXaXp7hrann`).
- `CODEX_REFRESH_AUTH_CURL_CONNECT_TIMEOUT_SECONDS` (default `2`).
- `CODEX_REFRESH_AUTH_CURL_MAX_TIME_SECONDS` (default `8`).
- `CODEX_SYNC_AUTH_ON_CHANGE_ENABLED` (default `true`).

Auto-refresh:

- `CODEX_AUTO_REFRESH_ENABLED` (default `false`).
- `CODEX_AUTO_REFRESH_MIN_DAYS` (default `5`).

Rate limits:

- `CODEX_CHATGPT_BASE_URL` (default `https://chatgpt.com/backend-api/`).
- `CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED` (default `false`).
- `CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS` (default `2`).
- `CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS` (default `8`).
- `ZSH_DEBUG` (if `>=2`, enables debug behavior for `rate-limits`).

Starship:

- `CODEX_STARSHIP_ENABLED` (must be `true` to output anything).
- `CODEX_STARSHIP_COLOR_ENABLED` (optional; when set, must be truthy to emit ANSI).
- `CODEX_STARSHIP_SHOW_5H_ENABLED` (optional; defaults to `true`).
- `CODEX_STARSHIP_TTL` (default `5m`).
- `CODEX_STARSHIP_REFRESH_MIN_SECONDS` (default `30`).
- `CODEX_STARSHIP_LOCK_STALE_SECONDS` (default `90`).
- `CODEX_STARSHIP_AUTH_HASH_CACHE_KEEP` (default `5`).
- `CODEX_STARSHIP_NAME_SOURCE` (`secret` | `email`; default `secret`).
- `CODEX_STARSHIP_SHOW_FALLBACK_NAME_ENABLED` (show identity-derived fallback name).
- `CODEX_STARSHIP_SHOW_FULL_EMAIL_ENABLED` (when false, display name uses email local-part only).
- `CODEX_STARSHIP_STALE_SUFFIX` (default `" (stale)"`, appended when cached output is stale).
- `CODEX_STARSHIP_CURL_CONNECT_TIMEOUT_SECONDS` (default `2`).
- `CODEX_STARSHIP_CURL_MAX_TIME_SECONDS` (default `8`).
- `NO_COLOR` (disables ANSI output when set).

Path resolution helpers:

- `ZDOTDIR`, `ZSH_SCRIPT_DIR`, `_ZSH_BOOTSTRAP_PRELOAD_PATH` are used to locate feature files.
- `ZSH_CACHE_DIR` (fallback for cache root when set).
- `STARSHIP_SESSION_KEY`, `STARSHIP_SHELL` influence whether color is enabled when `CODEX_STARSHIP_COLOR_ENABLED` is unset.

## Filesystem paths and side effects

Auth and secrets:

- Reads `CODEX_AUTH_FILE` and secret JSON files under `CODEX_SECRET_DIR`.
- Writes `CODEX_AUTH_FILE` (copy from selected secret) and ensures parent dir exists.
- Writes timestamp sidecars in `CODEX_SECRET_CACHE_DIR/<filename>.timestamp`.
- `codex-sync-auth-to-secrets` copies auth JSON into matching secrets and sets file permissions to `0600`.

Rate limits:

- Writes `codex_rate_limits` metadata back into the target JSON:
  - `.codex_rate_limits.weekly_reset_at`
  - `.codex_rate_limits.weekly_reset_at_epoch`
  - `.codex_rate_limits.weekly_fetched_at`
  - `.codex_rate_limits.non_weekly_reset_at`
  - `.codex_rate_limits.non_weekly_reset_at_epoch`
- Writes/reads starship cache KV files under `$ZSH_CACHE_DIR/codex/starship-rate-limits` (fallback to `$ZDOTDIR/cache`).
- Clears starship cache dir when `-c` is used (refuses to remove unsafe paths).

Starship:

- Cache files: `$ZSH_CACHE_DIR/codex/starship-rate-limits/<key>.kv`.
- Lock dirs: `$ZSH_CACHE_DIR/codex/starship-rate-limits/<key>.refresh.lock`.
- Temporary usage JSON: `$ZSH_CACHE_DIR/codex/starship-rate-limits/wham.usage.*`.
- No output on failures; returns 0 unless `--is-enabled` or invalid `--ttl`.

## Exit codes and error conventions (selected)

General:

- Usage errors return `64` (EX_USAGE) and print usage to stderr.
- Missing files generally return `1` with a `codex-*:` error prefix on stderr.

Auth:

- `auth current`:
  - `1` if `CODEX_AUTH_FILE` missing or hash failure.
  - `2` if no matching secret.
- `auth use`:
  - `64` for invalid args/usage.
  - `1` if secret not found.
  - `2` if email/local-part matches multiple secrets.
- `auth refresh`:
  - `64` for invalid args/usage.
  - `1` if target file missing.
  - `2` if refresh token missing in JSON.
  - `3` for token endpoint failure (network or non-200 response).
  - `4` invalid JSON response.
  - `5` failed JSON merge.
  - `6` sync-to-secrets failure.
- `auth auto-refresh`:
  - `64` if `CODEX_AUTO_REFRESH_MIN_DAYS` invalid or extra args.
  - `1` if any refresh failures occurred.

Diag (rate-limits):

- `2` if access_token missing.
- `3` for request failure or non-200 response.
- `4` for writeback failures.
- `5` for sync-to-secrets failure.
- `64` for invalid option combinations or invalid args.

Starship:

- `--is-enabled` returns 0 when `CODEX_STARSHIP_ENABLED=true`; 1 otherwise.
- Invalid `--ttl` returns 2 and prints usage.

## Command details

### `codex-cli agent`

Subcommands:

- `prompt [prompt...]`
  - If no prompt provided, reads from stdin with `Prompt: `.
  - Error: `codex-tools: missing prompt` (stderr) and exit 1.
- `advice [question]`
  - Reads prompt template `actionable-advice.md` from `$ZDOTDIR/prompts` (or feature fallback).
  - If no question provided, prompts `Question: `.
- `knowledge [concept]`
  - Reads prompt template `actionable-knowledge.md` from prompts dir.
- `commit [-p|--push] [-a|--auto-stage] [extra prompt...]`
  - Requires `git` binary and an in-repo cwd.
  - If `--auto-stage`, runs `git add -A` before committing.
  - If `semantic-commit` is available, runs `codex exec` with:
    - `--dangerously-bypass-approvals-and-sandbox -s workspace-write`
    - `-m CODEX_CLI_MODEL`
    - `-c model_reasoning_effort=CODEX_CLI_REASONING`
  - If `semantic-commit` is missing, falls back to interactive Conventional Commit prompts and runs `git commit -m`.
  - If `--push`, runs `git push` after commit.

Dangerous mode gate:

- All agent commands require `CODEX_ALLOW_DANGEROUS_ENABLED=true`.
- Error message format:
  - `codex: disabled (set CODEX_ALLOW_DANGEROUS_ENABLED=true)` or
  - `<caller>: disabled (set CODEX_ALLOW_DANGEROUS_ENABLED=true)`

### `codex-cli auth`

Subcommands:

- `use <profile|email>`
  - Rejects inputs containing `/` or `..` (exit 64).
  - Resolves by:
    1. `<name>.json` in `CODEX_SECRET_DIR` (if no extension provided).
    2. Email local-part or full email match from token payloads.
  - Ambiguous email match returns 2 and prints:
    - `codex-use: identifier matches multiple secrets: <input>`
    - `codex-use: candidates: a.json, b.json`
  - On success: copies secret to `CODEX_AUTH_FILE`, writes timestamp, prints:
    - `codex: applied <secret.json> to <CODEX_AUTH_FILE>`

- `refresh [secret.json]`
  - Target defaults to `CODEX_AUTH_FILE`.
  - Calls `https://auth.openai.com/oauth/token` with `refresh_token`.
  - On success: merges tokens into `.tokens`, writes `.last_refresh`, updates timestamps, and syncs to secrets if target is the auth file.
  - Success message:
    - `codex: refreshed <target> at <ISO8601>`

- `auto-refresh`
  - Uses `CODEX_AUTO_REFRESH_MIN_DAYS` to determine staleness.
  - Refreshes `CODEX_AUTH_FILE` and all secrets; prints summary line when run as a script or when any refresh/failure occurs:
    - `codex-auto-refresh: refreshed=X skipped=Y failed=Z (min_age_days=N)`

- `current`
  - Matches `CODEX_AUTH_FILE` to secrets via identity key (identity + account_id) or exact SHA-256 hash.
  - Outputs:
    - `codex: <auth file> matches <secret>`
    - `codex: <auth file> matches <secret> (identity; secret differs)`
    - `codex: <auth file> does not match any known secret`

- `sync`
  - Copies `CODEX_AUTH_FILE` content into matching secrets and updates timestamps.
  - No output on success.

### `codex-cli diag rate-limits`

Usage:

- `codex-cli diag rate-limits [-c] [-d] [--cached] [--no-refresh-auth] [--json] [--one-line] [--all] [secret.json]`

Options:

- `-c` Clear starship cache dir before querying.
- `-d, --debug` Keep stderr for per-account errors in `--all` mode.
- `--cached` Read from starship cache only; implies `--one-line`; forbids `--json` and `-c`.
- `--no-refresh-auth` Disable 401 refresh + retry.
- `--json` Raw wham/usage JSON (single account only).
- `--one-line` Single-line summary (single account only; implied by `--all`).
- `--all` Query every `*.json` in `CODEX_SECRET_DIR` and render a table.

Behavior:

- Base URL: `${CODEX_CHATGPT_BASE_URL}/wham/usage` (trailing slash trimmed).
- Adds `Authorization: Bearer <access_token>` and `ChatGPT-Account-Id` (if present).
- On HTTP 401 and refresh enabled, refreshes tokens and retries once.

Output:

- Default (single account):
  - `Rate limits remaining`
  - `<PrimaryLabel> <pct>% • <local reset>`
  - `<SecondaryLabel> <pct>% • <local reset>`
- `--one-line` (single account):
  - `<name> <window>:<pct>% W:<pct>% <weekly_reset_iso>`
- `--all`:
  - Header includes the literal emoji `🚦` and prints a fixed-width table.
  - Table is sorted by weekly reset epoch (missing epochs last).
  - Missing cached entries do not force a non-zero exit in cached mode; non-cached errors set exit code 1.

### `codex-cli config`

Subcommands:

- `show`
  - Prints current values for:
    - `CODEX_CLI_MODEL`
    - `CODEX_CLI_REASONING`
    - `CODEX_ALLOW_DANGEROUS_ENABLED`
  - If secrets are loaded, also prints:
    - `CODEX_SECRET_DIR`, `CODEX_AUTH_FILE`, `CODEX_SECRET_CACHE_DIR`, `CODEX_AUTO_REFRESH_ENABLED`, `CODEX_AUTO_REFRESH_MIN_DAYS`.

- `set <key> <value>`
  - Keys: `model`, `reasoning`, `dangerous` (aliases allowed as in Zsh script).
  - Writes to current shell only; no files are written.
  - For `dangerous`, only `true|false` are accepted.

### `codex-cli starship`

Usage:

- `codex-cli starship [--no-5h] [--ttl <duration>] [--time-format <strftime>] [--refresh] [--is-enabled]`

Behavior:

- If `CODEX_STARSHIP_ENABLED` is falsey, prints nothing and exits 0.
- `--is-enabled` exits 0 if enabled, 1 if disabled.
- `--ttl` supports `s/m/h/d/w` suffixes; invalid values return 2 and print usage.
- In normal mode (no `--refresh`):
  - Prints cached output immediately (even if stale) and appends `CODEX_STARSHIP_STALE_SUFFIX` when stale.
  - Triggers background refresh when cache missing or stale.
- In `--refresh` mode:
  - Runs a blocking refresh with locking and updates cache, then prints fresh output.
- Output format:
  - Default: `<name> <window>:<pct>% W:<pct>% <weekly_reset_time>`
  - `--no-5h`: `<name> W:<pct>% <weekly_reset_time>`

# codex-cli fixtures

This document defines deterministic fixtures and edge-case coverage for codex-cli parity tests.

## Fixture layout (proposed)

```
fixtures/codex-cli/
  auth/
    auth-active.json
    auth-missing-refresh.json
    auth-invalid-json.json
  secrets/
    alpha.json
    beta.json
    alpha-duplicate.json
    gamma-missing-tokens.json
  cache/
    secrets/
      auth.json.timestamp
      alpha.json.timestamp
    starship-rate-limits/
      alpha.kv
      beta.kv
      auth_<hash>.kv
  http/
    oauth-token-200.json
    oauth-token-401.json
    wham-usage-200.json
    wham-usage-401.json
```

## Auth/secrets JSON templates

Base structure used by `auth-active.json` and secret files:

```json
{
  "tokens": {
    "access_token": "hdr.<payload>.sig",
    "refresh_token": "refresh_token_value",
    "id_token": "hdr.<payload>.sig",
    "account_id": "acct_001"
  },
  "last_refresh": "2025-01-20T12:34:56Z"
}
```

Deterministic JWT payloads (base64url; no padding):

- `payload_alpha`:
  - `eyJzdWIiOiJ1c2VyXzEyMyIsImVtYWlsIjoiYWxwaGFAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF91c2VyX2lkIjoidXNlcl8xMjMiLCJlbWFpbCI6ImFscGhhQGV4YW1wbGUuY29tIn19`
- `payload_beta`:
  - `eyJzdWIiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSIsImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X3VzZXJfaWQiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSJ9fQ`

Example tokens (header can be any base64url string):

- `hdr = eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0`
- `access_token = hdr.<payload_alpha>.sig`
- `id_token = hdr.<payload_alpha>.sig`

Profiles:

- `secrets/alpha.json`: payload_alpha, account_id `acct_001`.
- `secrets/beta.json`: payload_beta, account_id `acct_002`.
- `secrets/alpha-duplicate.json`: same payload/email as alpha to trigger ambiguity.
- `auth/auth-active.json`: identical to `secrets/alpha.json` (exact hash match).
- `auth/auth-missing-refresh.json`: no refresh_token (to trigger exit 2 in refresh).
- `auth/auth-invalid-json.json`: invalid JSON (syntax error) for error paths.

## Cache fixtures

### Secrets timestamp cache

- `cache/secrets/<filename>.timestamp` contains ISO8601 `last_refresh` (e.g. `2025-01-20T12:34:56Z`).

### Starship cache KV

KV format (one per line):

```
fetched_at=1700000000
non_weekly_label=5h
non_weekly_remaining=94
non_weekly_reset_epoch=1700003600
weekly_remaining=88
weekly_reset_epoch=1700600000
```

Fixtures:

- `cache/starship-rate-limits/alpha.kv` - valid cache for alpha.
- `cache/starship-rate-limits/beta.kv` - valid cache for beta.
- `cache/starship-rate-limits/auth_<hash>.kv` - valid cache for auth file hash key.
- Invalid cache variants (missing weekly or non-weekly fields) to trigger errors.

## HTTP stubs

### OAuth token success (200)

`http/oauth-token-200.json`:

```json
{
  "access_token": "new_access",
  "refresh_token": "new_refresh",
  "id_token": "new_id",
  "token_type": "Bearer",
  "expires_in": 3600
}
```

### OAuth token error (401)

`http/oauth-token-401.json`:

```json
{
  "error": "invalid_grant",
  "error_description": "Refresh token expired"
}
```

### wham/usage success (200)

`http/wham-usage-200.json`:

```json
{
  "rate_limit": {
    "primary_window": {
      "limit_window_seconds": 18000,
      "used_percent": 12,
      "reset_at": 1700003600
    },
    "secondary_window": {
      "limit_window_seconds": 604800,
      "used_percent": 25,
      "reset_at": 1700600000
    }
  }
}
```

### wham/usage unauthorized (401)

`http/wham-usage-401.json`:

```json
{
  "error": "Unauthorized"
}
```

## Edge-case matrix

| Scenario | Inputs | Expected behavior | Exit code |
|---|---|---|---|
| Missing `codex` binary | `agent prompt` with dangerous enabled | stderr `missing binary: codex` (if wrapper checks); no exec | 1 |
| Missing `git` | `agent commit` | stderr `codex-commit-with-scope: missing binary: git` | 1 |
| Not a git repo | `agent commit` | stderr `codex-commit-with-scope: not a git repository` | 1 |
| No staged changes | `agent commit` (no auto-stage) | stderr `no staged changes` | 1 |
| Invalid `auth use` arg | `auth use ../x` | stderr `invalid secret name` | 64 |
| Ambiguous profile | `auth use alpha` with `alpha.json` and `alpha-duplicate.json` | stderr includes `identifier matches multiple secrets` + candidates | 2 |
| Missing secret | `auth use missing` | stderr `secret not found` | 1 |
| Missing refresh token | `auth refresh` on `auth-missing-refresh.json` | stderr `failed to read refresh token` | 2 |
| Refresh 401 then success | `auth refresh` with first 401 then 200 | refresh+retry, success message, timestamps updated | 0 |
| Refresh non-200 | `auth refresh` with 401/500 | stderr with error summary (if present) | 3 |
| Rate limits 401 refresh retry | `diag rate-limits` | refresh tokens, retry once, success | 0 |
| Rate limits 401 no-refresh | `diag rate-limits --no-refresh-auth` | no retry, stderr non-200 | 3 |
| `--cached` without cache | `diag rate-limits --cached` | stderr `cache not found` | 1 |
| `--cached` invalid cache | missing weekly/non-weekly data | stderr `invalid cache` | 1 |
| `--json` + `--cached` | invalid combo | usage error | 64 |
| `--all` + `--json` | invalid combo | usage error | 64 |
| `--all` empty secret dir | no secrets | stderr `no secrets found` | 1 |
| `--async` jobs invalid | `--jobs 0` or non-numeric | default to 5 | 0 |
| Async debug | `--async --debug` | prints captured per-account stderr after table | 0 or 1 |
| Starship stale output | cached but expired | prints cached output + `CODEX_STARSHIP_STALE_SUFFIX` | 0 |
| Starship disabled | `CODEX_STARSHIP_ENABLED=false` | prints nothing | 0 |
| `NO_COLOR` set | rate-limits table | no ANSI color output | 0 |

