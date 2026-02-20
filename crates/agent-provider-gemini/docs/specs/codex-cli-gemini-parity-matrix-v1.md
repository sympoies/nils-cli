# codex-cli -> Gemini parity matrix v1

## Purpose

This matrix defines how `codex-cli` user-facing capabilities map to Gemini-backed execution in the
provider architecture (`agent-provider-gemini` + `agentctl`) and Gemini provider CLI parity
(`gemini-cli`).

Classification:

- `exact`: behavior is expected to be equivalent.
- `semantic`: same user intent, different implementation/details.
- `unsupported`: no Gemini equivalent in this release.

## Capability matrix

| codex-cli surface | Gemini mapping | Class | Deterministic behavior notes |
| --- | --- | --- | --- |
| `help/-V/usage errors` | `gemini-cli` top-level command graph | exact | Help/version exit with `0`; invalid usage exits with `64`. |
| `agent prompt` | `gemini-cli agent prompt` or `agentctl workflow run --provider gemini` | semantic | Prompt intent is preserved; provider output text and envelope fields can differ. |
| `agent advice` | `gemini-cli agent advice` or `agentctl` workflow step with `advice:` template | semantic | Advice template intent is equivalent; model-specific wording is expected. |
| `agent knowledge` | `gemini-cli agent knowledge` or `agentctl` workflow step with `knowledge:` template | semantic | Explanatory intent is equivalent; provider-specific tokenization/formatting can vary. |
| `agent commit` | none | unsupported | Commit workflow is codex/git-specific and remains outside Gemini contracts. |
| `auth login/use/save/remove/refresh/auto-refresh/current/sync` | `gemini-cli auth` family + adapter `auth-state` through `provider-adapter.v1` | semantic | Auth intent parity is required; secret-file layout and runtime auth mechanism may differ. |
| `diag rate-limits` | `gemini-cli diag rate-limits` + adapter error/limits mapping | semantic | Rate-limit intent and category mapping are stable; wire-source details can differ. |
| `config show/set` | `gemini-cli config show/set` with Gemini env namespace | semantic | Same operator intent; environment keys and emitted shell snippets are Gemini-specific. |
| `starship` | none | unsupported | Codex Starship integration remains Codex-specific UI behavior. |
| `completion zsh/bash` | `gemini-cli completion zsh|bash` | exact | Completion export remains deterministic by shell target and command topology. |

## Parity-critical behavior

- Stable `provider-adapter.v1` envelopes for success/error from `agent-provider-gemini`.
- Stable error category/code mapping for `auth`, `rate-limit`, `timeout`, `network`,
  `validation`, and `unavailable`.
- Deterministic health/readiness states in `healthcheck`.
- Deterministic `auth-state` classification (`authenticated`, `unauthenticated`, `unknown`).
- No silent fallback from unsupported surfaces into different command behavior.

## Unsupported behavior policy

Unsupported items must:

1. return a stable provider error code/category when requested through provider execution paths, or
2. be documented with an explicit alternative path (`agentctl` runbooks), without silent fallback.
