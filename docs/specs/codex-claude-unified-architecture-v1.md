# codex/claude unified architecture v1

## Purpose
Define one canonical ownership model for Codex and Claude execution surfaces in this workspace.
This contract keeps `codex-cli` stable, adds first-class `claude-cli` support, and keeps
`agentctl` as the provider-neutral orchestration plane.

## Ownership Model

| Layer | Codex responsibilities | Claude responsibilities | Must not own |
| --- | --- | --- | --- |
| `*-core` runtime crates (`codex-core`, `claude-core`) | Runtime primitives for auth/config/path/exec/error mapping. | Runtime primitives for env/config/prompt/client/exec/error mapping. | CLI argument parsing, help text, provider-neutral orchestration, `provider-adapter.v1` envelopes. |
| `*-cli` provider CLIs (`codex-cli`, `claude-cli`) | Provider-specific user workflows and output contracts for Codex operations. | Provider-specific user workflows and output contracts for Claude operations. | Provider-neutral orchestration (`provider`, `debug`, `workflow`, `automation`), adapter internals. |
| Provider adapters (`agent-provider-codex`, `agent-provider-claude`) | Map runtime outcomes to `provider-adapter.v1` for `agentctl`. | Map runtime outcomes to `provider-adapter.v1` for `agentctl`. | Direct user-facing CLI UX and feature-specific command contracts. |
| `agentctl` | Provider-neutral registry, health, diagnostics, debug bundles, workflow orchestration, automation entrypoints. | Same. | Provider-specific command UX that belongs in `codex-cli` or `claude-cli`. |

## CLI Selection Contract

| Operator need | Canonical surface |
| --- | --- |
| Codex-only commands (`agent`, `auth`, `diag rate-limits`, `config`, `starship`) | `codex-cli` |
| Claude-only commands (`agent`, `auth-state`, `diag`, `config`) | `claude-cli` |
| Provider-neutral operations (`provider`, `diag doctor/capabilities`, `debug`, `workflow`, `automation`) | `agentctl` |
| Migrated `codex-cli` wrapper commands (`provider`, `debug`, `workflow`, `automation`) | `agentctl` (forwarded by `wrappers/codex-cli`) |

## Compatibility Commitments

1. `codex-cli` command behavior, warning style, and exit semantics remain contract-stable.
2. `codex-cli` JSON schema versions remain stable:
   - `codex-cli.diag.rate-limits.v1`
   - `codex-cli.auth.v1`
3. `provider-adapter.v1` envelopes remain stable for `agentctl` consumers.
4. Claude support is available through both `agentctl` and `claude-cli` when `claude-cli` is part
   of the shipped release set.

## Anti-goals

- Do not move provider-neutral workflows back into `codex-cli` or `claude-cli`.
- Do not embed runtime-heavy client/config logic directly in `agent-provider-*` crates.
- Do not force codex-only UX surfaces (`agent commit`, `starship`) into Claude contracts.
- Do not couple `codex-core` and `claude-core` to each other's provider-specific policies.

## Dependency Direction

- `codex-cli` -> `codex-core`
- `claude-cli` -> `claude-core`
- `agent-provider-codex` -> `codex-core` + `agent-runtime-core`
- `agent-provider-claude` -> `claude-core` + `agent-runtime-core`
- `agentctl` -> `agent-provider-*` + `agent-runtime-core`

Any reverse edge that pulls provider-neutral orchestration into a provider CLI, or CLI UX into a
core crate, violates this contract.
