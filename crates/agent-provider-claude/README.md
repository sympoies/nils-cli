# agent-provider-claude

## Overview

`agent-provider-claude` is the Claude adapter implementation for `provider-adapter.v1`.

It is designed for provider-neutral orchestration through `agentctl`.

## Runtime requirements

- `ANTHROPIC_API_KEY` for execute/authenticated state.
- Network access to Anthropic API endpoint (default `https://api.anthropic.com`).
- Optional local `claude` CLI for characterization workflows only.

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

## References

- `../agent-runtime-core/README.md`
- `../../docs/runbooks/provider-onboarding.md`
- `docs/specs/claude-provider-contract-v1.md`
- `docs/specs/codex-cli-claude-parity-matrix-v1.md`
- `docs/runbooks/verification-oracles.md`
- `../agentctl/docs/runbooks/codex-to-claude-mapping.md`

## Docs

- [Docs index](docs/README.md)
