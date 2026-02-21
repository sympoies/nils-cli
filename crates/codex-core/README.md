# codex-core

## Overview

`codex-core` owns reusable Codex runtime primitives used across crates.

- Runtime scope: `auth`, `jwt`, `paths`, `config`, `exec`, typed runtime errors
- Non-goals: CLI argument parsing, help text, shell completion rendering, user-facing command routing
- Consumers: `codex-cli` (UX layer)

## Ownership boundary

| Concern | Owner |
|---|---|
| Runtime primitives and policies | `codex-core` |
| Command UX, parsing, user-facing messaging | `codex-cli` |

## Docs

- [Docs index](docs/README.md)
