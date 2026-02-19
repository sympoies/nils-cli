# codex-cli -> Claude parity matrix v1

## Purpose

This matrix defines how `codex-cli` user-facing capabilities map to Claude-backed execution in the
provider architecture (`agent-provider-claude` + `agentctl`).

Classification:

- `exact`: behavior is expected to be equivalent.
- `semantic`: same user intent, different implementation/details.
- `unsupported`: no Claude adapter equivalent in this release.

## Capability matrix

| codex-cli surface | Claude mapping | Class | Notes |
| --- | --- | --- | --- |
| `agent prompt` | `agentctl workflow run` provider step with `provider=claude` and prompt input | semantic | Output text intent is equivalent; provider envelope differs. |
| `agent advice` | same as above, `advice:` prompt intent template | semantic | Uses template expansion in `agent-provider-claude`. |
| `agent knowledge` | same as above, `knowledge:` prompt intent template | semantic | Uses explanatory template expansion. |
| `agent commit` | none | unsupported | Commit workflow is codex/git-specific. |
| `auth login/use/save/remove/refresh/auto-refresh/current/sync` | `auth-state` only (`provider-adapter.v1`) | semantic | Claude adapter does not manage Codex secret files. |
| `diag rate-limits` | provider-level health + error categories (`rate-limit`) | semantic | Claude adapter surfaces rate-limit errors in execute envelopes. |
| `config show/set` | environment-based adapter config (`ANTHROPIC_*`, `CLAUDE_*`) | semantic | No shell snippet emitter in provider adapter. |
| `starship` | none | unsupported | Starship integration is Codex-specific UI logic. |

## Parity-critical behavior

- Stable `provider-adapter.v1` envelopes for success/error.
- Stable error category and code mapping for `auth`, `rate-limit`, `timeout`, `network`,
  `validation`, and `unavailable`.
- Deterministic health/readiness states in `healthcheck`.
- Deterministic `auth-state` classification (`authenticated` vs `unauthenticated`).

## Unsupported behavior policy

Unsupported items must:

1. return a stable provider error code/category when requested through provider execution paths, or
2. be documented with an explicit alternative path (`agentctl` runbooks), without silent fallback.
