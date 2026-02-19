# agent-provider-claude

## Overview

`agent-provider-claude` is the stable Claude adapter implementation for
`provider-adapter.v1` (`provider=claude`, `maturity=stable`).

It is designed for provider-neutral orchestration through `agentctl`.

## Runtime requirements

Required runtime requirements:

- `ANTHROPIC_API_KEY` for execute/authenticated state.
- Network access to Anthropic API endpoint (default `https://api.anthropic.com`).

Optional dependency:

- Local `claude` CLI for characterization workflows only.
  Runtime execute/authenticated flows do not depend on this binary.

## Environment

- `ANTHROPIC_API_KEY`: required for execute.
- `ANTHROPIC_BASE_URL`: override API base URL.
- `CLAUDE_MODEL` / `ANTHROPIC_MODEL`: model override.
- `CLAUDE_TIMEOUT_MS`: request timeout.
- `CLAUDE_MAX_TOKENS`: max output tokens.
- `CLAUDE_RETRY_MAX`: retry attempts for retryable failures.
- `CLAUDE_MAX_CONCURRENCY`: limits surface override.
- `ANTHROPIC_AUTH_SUBJECT`: explicit auth subject.
- `ANTHROPIC_AUTH_SCOPES`: comma-separated scopes.

## Role in ownership boundary

- Adapter implementation lives in `agent-provider-*` crates.
- Adapter contract and shared schema live in `../agent-runtime-core`.
- Provider-neutral orchestration and dispatch live in `../agentctl`.
- CLI migration classifications (`exact`/`semantic`/`unsupported`) and fallback guidance live in `../agentctl/docs/runbooks/codex-to-claude-mapping.md`, aligned to `docs/specs/codex-cli-claude-parity-matrix-v1.md`.
- Provider core runtime behavior must stay free of CLI coupling: no `clap` parsing, no `nils-agentctl` command flow logic, and no CLI rendering code in `src/`.
- Boundary regression checks in `tests/adapter_contract.rs` enforce these provider core constraints.

## References

- `../agent-runtime-core/README.md`
- `../../docs/runbooks/provider-onboarding.md`
- `docs/specs/claude-provider-contract-v1.md`
- `docs/specs/codex-cli-claude-parity-matrix-v1.md`
- `docs/runbooks/verification-oracles.md`
- `../agentctl/docs/runbooks/codex-to-claude-mapping.md`

## Docs

- [Docs index](docs/README.md)
