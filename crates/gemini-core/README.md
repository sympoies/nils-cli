# gemini-core

## Overview

`gemini-core` owns reusable Gemini runtime primitives used across crates.

- Runtime scope: `auth`, `jwt`, `paths`, `config`, `exec`, typed runtime errors
- Non-goals: CLI argument parsing, help text, shell completion rendering, user-facing command routing
- Consumers: `gemini-cli` (UX layer)

## Ownership boundary

| Concern | Owner |
|---|---|
| Runtime primitives and policies | `gemini-core` |
| Command UX, parsing, user-facing messaging | `gemini-cli` |

## Docs

- [Docs index](docs/README.md)
