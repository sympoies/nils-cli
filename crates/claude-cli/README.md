# claude-cli

## Overview

`claude-cli` is a provider-specific Rust CLI for Claude workflows.
It uses `claude-core` runtime primitives and owns user-facing command UX for:
- `agent` (`prompt|advice|knowledge`)
- `auth-state` (`show`)
- `diag` (`healthcheck`, with explicit unsupported guidance for codex-only `rate-limits`)
- `config` (`show|set`)

## Usage

```text
Usage:
  claude-cli <group> <command> [args]

Groups:
  agent       prompt | advice | knowledge
  auth-state  show
  diag        healthcheck | rate-limits (unsupported guidance)
  config      show | set

Help:
  claude-cli help
  claude-cli <group> help
```

## Scope boundary

| Job | Primary owner |
|---|---|
| Shared Claude runtime layer (`config/prompts/client/exec`) | `claude-core` |
| Claude provider-specific user workflows (`agent/auth-state/diag/config`) | `claude-cli` |
| Provider-neutral orchestration (`provider`, `diag doctor`, `workflow`, `automation`) | `agentctl` |
| OpenAI/Codex-specific auth/rate-limit/starship surfaces | `codex-cli` |

- `claude-cli` is first-class for Claude-specific user workflows.
- Provider-neutral orchestration remains in `agentctl`.
- Codex-only features (`agent commit`, `starship`, codex auth secret lifecycle, codex rate-limit table) stay in `codex-cli` and return stable unsupported guidance in `claude-cli` when invoked.

## JSON contracts

- Contract spec: `docs/specs/claude-cli-json-contract-v1.md`
- Migration runbook: `docs/runbooks/codex-to-claude-cli-migration.md`

## Exit codes

- `0`: success and help output.
- `64`: usage errors and unsupported codex-only command guidance.
- `1`: operational runtime errors.

## Docs

- [Docs index](docs/README.md)
