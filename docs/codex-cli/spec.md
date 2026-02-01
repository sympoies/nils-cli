# codex-cli parity spec

## Scope and sources

This spec documents the Rust `codex-cli` contract based on the current Zsh implementation in:

- `~/.config/zsh/scripts/_features/codex/codex-tools.zsh`
- `~/.config/zsh/scripts/_features/codex/codex-secret.zsh`
- `~/.config/zsh/scripts/_features/codex/codex-auto-refresh.zsh`
- `~/.config/zsh/scripts/_features/codex/codex-starship.zsh`
- `~/.config/zsh/scripts/_features/codex/alias.zsh`

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

