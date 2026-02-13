# agent-runtime-core

## Overview

`agent-runtime-core` defines the provider-neutral runtime contract used by `agentctl` and
provider adapter crates.

- Contract identifier: `provider-adapter.v1`
- Primary surfaces: `capabilities`, `healthcheck`, `execute`, `limits`, `auth-state`
- Shared envelopes: normalized success/error schema and compatibility defaults

## Ownership boundary

| Job | Primary owner |
|---|---|
| Provider adapter implementation against `provider-adapter.v1` | `agent-provider-*` crates + `agent-runtime-core` |

## Who uses this crate

- `agentctl`: provider registry, diagnostics, and workflow execution against adapter contracts
- `agent-provider-codex`: stable Codex adapter implementation
- `agent-provider-claude`: compile-only onboarding stub (`maturity=stub`)
- `agent-provider-gemini`: compile-only onboarding stub (`maturity=stub`)

## Onboarding references

- `../../docs/runbooks/provider-onboarding.md`
- `../agentctl/README.md`

## Docs

- [Docs index](docs/README.md)
