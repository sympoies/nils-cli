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
| `agent prompt` | `agentctl workflow run` provider step with `provider=claude` and prompt input | semantic | Output intent is equivalent; provider envelope and CLI wording differ. |
| `agent advice` | same as above, `advice:` prompt intent template | semantic | Uses template expansion in `agent-provider-claude`. |
| `agent knowledge` | same as above, `knowledge:` prompt intent template | semantic | Uses explanatory template expansion. |
| `agent commit` | none | unsupported | Commit workflow is codex/git-specific. |
| `auth current` | provider `auth-state` read (`provider-adapter.v1`) | exact | Read-only authentication state intent maps directly. |
| `auth login` | environment-based key provisioning (`ANTHROPIC_API_KEY`) + readiness checks | semantic | Equivalent goal, different flow (no Codex secret login flow). |
| `auth use` | none | unsupported | No profile/secret store selector in Claude adapter. |
| `auth save` | none | unsupported | No Codex-compatible secret persistence in adapter contract. |
| `auth remove` | none | unsupported | No Codex-compatible secret removal API. |
| `auth refresh` | none | unsupported | No token refresh API in Claude adapter contract. |
| `auth auto-refresh` | none | unsupported | No managed refresh daemon/control surface. |
| `auth sync` | none | unsupported | No Codex secret sync surface. |
| `diag doctor` | `agentctl diag doctor --provider claude` | semantic | Readiness diagnostics are equivalent intent with different field/format details. |
| `diag rate-limits` | provider execute errors with `rate-limit` category + `diag doctor` context | semantic | Claude adapter surfaces rate-limit via execute envelope semantics. |
| `config show` | inspect effective `ANTHROPIC_*` / `CLAUDE_*` environment config | semantic | Same operator intent, different output contract. |
| `config set` | set environment variables consumed by adapter | semantic | Same operator intent, no codex-cli shell snippet emitter. |
| `starship` | none | unsupported | Starship integration is Codex-specific UI logic. |

## Classification completeness guard

All codex-oriented surfaces used in migration docs are classified exactly once:

- `agent`: `prompt`, `advice`, `knowledge`, `commit`
- `auth`: `login`, `use`, `save`, `remove`, `refresh`, `auto-refresh`, `current`, `sync`
- `diag`: `doctor`, `rate-limits`
- `config`: `show`, `set`
- `starship`

Class coverage in this matrix:

- `exact`: present
- `semantic`: present
- `unsupported`: present

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
