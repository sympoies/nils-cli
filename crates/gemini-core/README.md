# gemini-core

## Overview

`gemini-core` owns reusable Gemini runtime primitives used across crates.

- Runtime scope: `auth`, `jwt`, `paths`, `config`, `exec`, typed runtime errors
- Non-goals: CLI argument parsing, help text, shell completion rendering, user-facing command routing
- Consumers: `gemini-cli` (UX layer), `agent-provider-gemini` (provider contract mapping)

## Ownership boundary

| Concern | Owner |
|---|---|
| Runtime primitives and policies | `gemini-core` |
| Command UX, parsing, user-facing messaging | `gemini-cli` |
| Provider contract mapping (`provider-adapter.v1`) | `agent-provider-gemini` |

## Runtime viability direction

- Target runtime path is `agent-provider-gemini -> gemini-core` for execution and auth-state logic.
- Deterministic CI behavior is fixture-backed; no live runtime dependency is required for default contract tests.
- When a runtime capability is unavailable or unsupported, callers must receive explicit, stable fallback errors.

## Docs

- [Docs index](docs/README.md)
