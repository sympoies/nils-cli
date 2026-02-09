# ADR: `codex-cli` / `agentctl` provider boundary

- Status: Accepted
- Date: 2026-02-08
- Owners: `codex-cli` maintainers, `agentctl` maintainers

## Context

The workspace is moving to a multi-provider architecture. We need a strict boundary so
provider-specific logic does not leak into provider-neutral orchestration.

## Decision

- `codex-cli` owns OpenAI/Codex provider-specific operations only.
- `agentctl` owns provider-neutral orchestration and local automation integration.
- Cross-provider control-plane features must live in `agentctl`, not `codex-cli`.

## Scope and ownership

### `codex-cli` (provider-specific)

Allow list:
- OpenAI/Codex auth and secret lifecycle (`use`, `refresh`, `auto-refresh`, `current`, `sync`).
- OpenAI/Codex execution wrappers and prompt tooling (`agent prompt/advice/knowledge/commit`).
- OpenAI/Codex diagnostics (for example, rate-limit and provider health signals).
- OpenAI/Codex-specific config and UX helpers (for example, `config`, `starship`) that do not
  orchestrate other providers.

Deny list:
- Provider-neutral provider registry, default-provider selection, or cross-provider routing.
- Provider-neutral workflow orchestration (`workflow run`, retries, multi-step execution).
- Provider-neutral diagnostics that aggregate multiple providers or local automation tools.
- Local automation integration contracts (`macos-agent`, `screen-record`, `image-processing`,
  `fzf-cli`) as first-class command ownership.

### `agentctl` (provider-neutral)

Allow list:
- Provider-neutral orchestration entrypoints (`provider`, `diag`, `debug`, `workflow`,
  `automation`).
- Provider registry and adapter dispatch across Codex and future providers.
- Unified diagnostics and debug bundles that combine provider + local automation readiness.
- Stable machine-readable contracts for local automation CLIs.

Deny list:
- OpenAI/Codex-only auth/secret mutation commands.
- OpenAI/Codex-only prompt convenience wrappers.
- Provider-specific policy flags or defaults that should remain in provider CLIs.

## Migration principles

1. Boundary-first: new commands must be placed by ownership first, then implemented.
2. Adapter-first: `agentctl` consumes provider adapters; it does not re-implement provider internals.
3. No dual ownership: a command family cannot be simultaneously owned by both CLIs.
4. Parity-preserving: existing `codex-cli` behavior remains stable until explicit migration is done.
5. Migration by redirection: when provider-neutral behavior exists in `codex-cli`, move ownership
   to `agentctl` and leave compatibility messaging in `codex-cli`.

## Compatibility policy

- Backward compatibility: existing `codex-cli` commands remain available during migration unless a
  documented breaking change is approved.
- Deprecation style: migrated provider-neutral flows should keep a compatibility shim period with
  explicit guidance to `agentctl`.
- Messaging rule: help/docs must always state that `codex-cli` is provider-specific and `agentctl`
  is provider-neutral.
- No silent behavior shifts: command ownership changes require migration notes in docs and release
  notes.

## Consequences

- Future providers can be added via `agentctl` adapter onboarding without expanding `codex-cli`
  scope.
- Local automation integrations remain centralized in one provider-neutral control plane.
