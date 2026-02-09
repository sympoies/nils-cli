# agent-provider-codex

## Overview

`agent-provider-codex` is the stable OpenAI/Codex provider adapter implementation for
`provider-adapter.v1`.

## Role in ownership boundary

- Adapter implementation lives in `agent-provider-*` crates.
- Adapter contract and shared schema live in `../agent-runtime-core`.
- Provider-neutral orchestration and dispatch live in `../agentctl`.

## References

- `../agent-runtime-core/README.md`
- `../../docs/runbooks/provider-onboarding.md`
