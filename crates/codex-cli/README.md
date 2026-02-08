# codex-cli

## Overview
codex-cli is a provider-specific Rust CLI for OpenAI/Codex workflows: Codex execution wrappers,
auth/secret management, Codex diagnostics, config output, and Starship rendering.

## Usage
```text
Usage:
  codex-cli <group> <command> [args]
  codex-cli starship [options]

Groups:
  agent    prompt | advice | knowledge | commit
  auth     use | refresh | auto-refresh | current | sync
  diag     rate-limits
  config   show | set
  starship (options)

Help:
  codex-cli help
  codex-cli <group> help
```

## Scope boundary
- `codex-cli` owns provider-specific OpenAI/Codex commands only (`agent`, `auth`, `diag rate-limits`, `config`, `starship`).
- `codex-cli` does not own provider-neutral orchestration concerns (multi-provider registry/selection, provider-neutral doctor/debug/workflow, or local automation integration).
- `agentctl` owns those provider-neutral concerns and integration contracts during migration.
- Compatibility note: existing `codex-cli` command behavior remains stable during migration, while provider-neutral ownership moves to `agentctl`.

## Commands

### agent
- `prompt [PROMPT...]`: Run a raw prompt through `codex exec`.
- `advice [QUESTION...]`: Request actionable engineering advice.
- `knowledge [CONCEPT...]`: Request a concept explanation.
- `commit [-p|--push] [-a|--auto-stage] [EXTRA...]`: Run the semantic-commit workflow.

### auth
- `use <name|email>`: Switch to a secret by name or email.
- `refresh [secret.json]`: Refresh OAuth tokens.
- `auto-refresh`: Refresh stale tokens across auth + secrets.
- `current`: Show which secret matches `CODEX_AUTH_FILE`.
- `sync`: Sync `CODEX_AUTH_FILE` back into matching secrets.

### diag
- `rate-limits [options] [secret.json]`: Rate-limit diagnostics. Options: `-c`, `-d/--debug`, `--cached`, `--no-refresh-auth`, `--json`, `--one-line`, `--all`, `--async`, `--jobs <n>`.

### config
- `show`: Print effective configuration values.
- `set <key> <value>`: Emit a shell snippet for the current shell.

### starship
- `starship [--no-5h] [--ttl <duration>] [--time-format <strftime>] [--show-timezone] [--refresh] [--is-enabled]`: Render or refresh the Starship line. Default reset time uses local time without timezone; `--show-timezone` adds the local offset.

## Environment
- `CODEX_ALLOW_DANGEROUS_ENABLED=true` is required for `agent` commands.
- `CODEX_CLI_MODEL` and `CODEX_CLI_REASONING` set `codex exec` defaults.
- `CODEX_SECRET_DIR`, `CODEX_AUTH_FILE`, `CODEX_SECRET_CACHE_DIR` control auth/secret paths.
- `CODEX_STARSHIP_ENABLED=true` enables Starship output.
- `CODEX_STARSHIP_TTL` overrides the cache TTL.

## Dependencies
- `codex` is required for `agent` commands.
- `git` is required for `agent commit`.
- `semantic-commit` and `git-scope` are optional for `agent commit` (fallbacks apply).

## Exit codes
- `0`: success and help output.
- `64`: usage or argument errors.
- `1`: operational errors.
