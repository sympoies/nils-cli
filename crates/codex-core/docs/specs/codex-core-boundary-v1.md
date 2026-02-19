# codex-core boundary v1

## Purpose

Define stable ownership boundaries for Codex runtime extraction.

## Module ownership

`codex-core` owns:
- `auth`: auth-file parsing and identity extraction helpers
- `path`: runtime path discovery (`CODEX_AUTH_FILE`, secret/cache paths, zsh feature paths)
- `config`: runtime config defaults and resolved environment snapshots
- `exec`: dangerous execution policy gate and codex execution wrapper
- `typed error`: runtime categories + mapping helpers for consumers

`codex-cli` owns:
- Clap command parsing and command routing
- User-facing output text/help/usage and shell completion output
- Compatibility redirects and CLI-specific exit handling

`agent-provider-codex` owns:
- Provider contract mapping (`provider-adapter.v1`)
- Mapping core errors/results into provider schema categories/codes
- Provider metadata/capabilities wiring

## Explicit anti-goals

`codex-core` must not:
- Depend on Clap or parse CLI argv
- Render CLI help text or command table output
- Own provider-neutral registry/orchestration logic
- Emit product UX strings that are specific to one CLI frontend

## Allowed dependency direction

- `codex-cli -> codex-core`
- `agent-provider-codex -> codex-core`
- `agent-provider-codex` must not import `codex-cli` runtime internals
