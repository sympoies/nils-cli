# agent-provider-gemini

## Overview

`agent-provider-gemini` is the stable Gemini adapter implementation (`maturity=stable`) for
`provider-adapter.v1`.

The adapter is runtime-backed via `gemini-core` and provides deterministic mappings for:

- `metadata`
- `capabilities`
- `healthcheck`
- `execute`
- `limits`
- `auth-state`

## Role in ownership boundary

- Adapter implementation lives in `agent-provider-*` crates.
- Adapter contract and shared schema live in `../agent-runtime-core`.
- Provider-neutral orchestration and dispatch live in `../agentctl`.
- Runtime execution/auth-state primitives are owned by `../gemini-core`.

## Validation focus

- Keep `agent-provider-gemini -> gemini-core` as the only runtime dependency edge.
- Preserve stable error category/code mappings for execute/auth-state failures.
- Keep fixture-backed tests deterministic in CI; optional live checks are drift detectors only.

## References

- `../agent-runtime-core/README.md`
- `../../docs/runbooks/provider-onboarding.md`

## Docs

- [Docs index](docs/README.md)
