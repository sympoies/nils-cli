# codex-core

## Overview

`codex-core` owns reusable Codex runtime primitives used across crates.

- Runtime scope: `auth`, `jwt`, `paths`, `config`, `exec`, typed runtime errors
- Non-goals: CLI argument parsing, help text, shell completion rendering, user-facing command routing
- Consumers: `codex-cli` (UX layer), `agent-provider-codex` (provider contract mapping)

## Ownership boundary

| Concern | Owner |
|---|---|
| Runtime primitives and policies | `codex-core` |
| Command UX, parsing, user-facing messaging | `codex-cli` |
| Provider contract mapping (`provider-adapter.v1`) | `agent-provider-codex` |

See [`docs/specs/codex-core-boundary-v1.md`](docs/specs/codex-core-boundary-v1.md).

## Docs

- [Docs index](docs/README.md)
- [Migration runbook](../../docs/runbooks/codex-core-migration.md)
