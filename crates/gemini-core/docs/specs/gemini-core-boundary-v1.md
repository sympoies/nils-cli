# gemini-core boundary v1

## Purpose

Define stable ownership boundaries for Gemini runtime extraction.

## Module ownership

`gemini-core` owns:
- `auth`: auth-file parsing and identity extraction helpers
- `jwt`: token decoding and claims parsing helpers
- `path`: runtime path discovery (`GEMINI_AUTH_FILE`, secret/cache paths, zsh feature paths)
- `config`: runtime config defaults and resolved environment snapshots
- `exec`: dangerous execution policy gate and gemini execution wrapper
- `typed error`: runtime categories + mapping helpers for consumers

`gemini-cli` owns:
- Clap command parsing and command routing
- User-facing output text/help/usage and shell completion output
- Compatibility redirects and CLI-specific exit handling

`agent-provider-gemini` owns:
- Provider contract mapping (`provider-adapter.v1`)
- Mapping core errors/results into provider schema categories/codes
- Provider metadata/capabilities wiring

## Explicit anti-goals

`gemini-core` must not:
- Depend on Clap or parse CLI argv
- Render CLI help text or command table output
- Own provider-neutral registry/orchestration logic
- Emit product UX strings that are specific to one CLI frontend

## Allowed dependency direction

- `gemini-cli -> gemini-core`
- `agent-provider-gemini -> gemini-core`
- `agent-provider-gemini` must not import `gemini-cli` runtime internals
- `agent-provider-gemini` must not depend on the `nils-gemini-cli` package
