# codex-cli

## Overview
codex-cli is a provider-specific Rust CLI for OpenAI/Codex workflows: Codex execution wrappers,
auth/secret management, Codex diagnostics, config output, and Starship rendering.
Core runtime primitives are sourced from `codex-core`; this crate owns only command UX and
provider-specific user-facing behavior.

## Usage
```text
Usage:
  codex-cli <group> <command> [args]
  codex-cli starship [options]

Groups:
  agent    prompt | advice | knowledge | commit
  auth     login | use | save | remove | refresh | auto-refresh | current | sync
  diag     rate-limits
  config   show | set
  starship (options)

Help:
  codex-cli help
  codex-cli <group> help
```

## Scope boundary
| Job | Primary owner |
|---|---|
| Shared Codex runtime layer (`auth/path/config/exec/error`) | `codex-core` |
| OpenAI/Codex auth, Codex prompt wrappers, Codex rate-limit diagnostics, Starship | `codex-cli` |
| Legacy top-level groups (`provider|debug|workflow|automation`) | unsupported (`64`) |

- `codex-cli` owns only provider-specific OpenAI/Codex operations (`agent`, `auth`, `diag rate-limits`, `config`, `starship`).
- Compatibility behavior: existing `codex-cli` commands stay stable for provider-specific workflows.
- Legacy top-level groups `provider|debug|workflow|automation` are retained only as deterministic usage errors (`64`).

## Commands

### agent
- `prompt [PROMPT...]`: Run a raw prompt through `codex exec`.
- `advice [QUESTION...]`: Request actionable engineering advice.
- `knowledge [CONCEPT...]`: Request a concept explanation.
- `commit [-p|--push] [-a|--auto-stage] [EXTRA...]`: Run the semantic-commit workflow.

### auth
- `login [--api-key|--device-code]`: Login via ChatGPT browser flow (`chatgpt-browser`, default), ChatGPT device-code flow (`chatgpt-device-code`), or API key flow (`api-key`). `--api-key` and `--device-code` are mutually exclusive (`64` on invalid usage).
- `use <name|email>`: Switch to a secret by name or email.
- `save [--yes] <secret.json>`: Save active `CODEX_AUTH_FILE` into `CODEX_SECRET_DIR` with an explicit file name. If target exists, interactive mode prompts for overwrite; non-interactive and JSON mode require `--yes` to overwrite.
- `remove [--yes] <secret.json>`: Remove a secret file from `CODEX_SECRET_DIR`. Interactive mode prompts for confirmation; non-interactive and JSON mode require `--yes`.
- `refresh [secret.json]`: Refresh OAuth tokens.
- `auto-refresh`: Refresh stale tokens across auth + secrets.
- `current`: Show which secret matches `CODEX_AUTH_FILE`.
- `sync`: Sync `CODEX_AUTH_FILE` back into matching secrets.

Auth examples:
- `codex-cli auth login`: ChatGPT browser login.
- `codex-cli auth login --device-code`: ChatGPT device-code login.
- `codex-cli auth login --api-key`: OpenAI API key login.
- `codex-cli auth save team-alpha.json`: Save and prompt before overwrite when applicable.
- `codex-cli auth save --yes team-alpha.json`: Force overwrite without prompt.
- `codex-cli auth remove --yes team-alpha.json`: Remove a saved secret file.

### diag
- `rate-limits [options] [secret.json]`: Rate-limit diagnostics. Options: `-c/--clear-cache`, `-d/--debug`, `--cached`, `--no-refresh-auth`, `--json`, `--one-line`, `--all`, `--async`, `--jobs <n>`.

### config
- `show`: Print effective configuration values.
- `set <key> <value>`: Emit a shell snippet for the current shell.

### starship
- `starship [--no-5h] [--ttl <duration>] [--time-format <strftime>] [--show-timezone] [--refresh] [--is-enabled]`: Render or refresh the Starship line. Default reset time uses local time without timezone; `--show-timezone` adds the local offset.

## JSON contract (service consumers)
- Human-readable text is the default output mode.
- Machine-readable JSON mode is explicit: use `--format json` (preferred) or `--json` where
  supported for compatibility.
- Contract spec: `docs/specs/codex-cli-diag-auth-json-contract-v1.md`
- Consumer runbook: `docs/runbooks/json-consumers.md`
- Covered surfaces: `diag rate-limits` (single/all/async) and
  `auth login|use|save|remove|refresh|auto-refresh|current|sync`.

## Environment
- `CODEX_ALLOW_DANGEROUS_ENABLED=true` is required for `agent` commands.
- `CODEX_CLI_MODEL` and `CODEX_CLI_REASONING` set `codex exec` defaults.
- `CODEX_SECRET_DIR` controls the secret directory path. When unset, it defaults to
  `~/.config/codex_secrets`.
- `CODEX_AUTH_FILE` controls the active auth file path. When unset, it defaults to
  `~/.agents/auth.json`.
- `CODEX_SECRET_CACHE_DIR` controls secret cache timestamps.
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

## Compatibility sign-off checklist

- [ ] `cargo test -p nils-codex-cli --test main_entrypoint --test dispatch`
- [ ] `rg -n "codex-cli\\.diag\\.rate-limits\\.v1|codex-cli\\.auth\\.v1" crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`
- [ ] `NILS_WRAPPER_MODE=debug ./wrappers/codex-cli provider list` exits `64` with a stable unsupported-command hint.

## Docs

- [Docs index](docs/README.md)
- [JSON consumers runbook](docs/runbooks/json-consumers.md)
