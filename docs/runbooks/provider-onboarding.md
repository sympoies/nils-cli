# Provider Onboarding Runbook (`agent-provider-*`)

This runbook defines the minimum required deliverables for onboarding a new provider adapter into the provider-neutral control plane.

## Scope

- Runtime contract: `crates/agent-runtime-core`
- Provider adapter crates: `crates/agent-provider-<provider>`
- Registry integration: `crates/agentctl/src/provider/registry.rs`
- Operator docs/completions: `README.md`, `crates/agentctl/README.md`, shell completions

## Required Files

1. `crates/agent-provider-<provider>/Cargo.toml`
2. `crates/agent-provider-<provider>/src/lib.rs`
3. `crates/agent-provider-<provider>/src/adapter.rs`
4. `crates/agent-provider-<provider>/tests/adapter_contract.rs`
5. Workspace registration in `/Users/terry/Project/graysurf/nils-cli/Cargo.toml`
6. Registry wiring in `/Users/terry/Project/graysurf/nils-cli/crates/agentctl/src/provider/registry.rs`

## Contract Checklist

- Adapter implements `ProviderAdapterV1`.
- `metadata().id` matches CLI selector (`--provider <id>`).
- `metadata().contract_version == provider-adapter.v1`.
- `metadata().maturity` is set explicitly (`stable` or `stub`).
- `healthcheck` returns deterministic status and summary.
- `execute` returns a normalized `ProviderError` if unsupported.
- `limits` / `auth_state` return valid schema payloads (even for stub adapters).

## Stub Provider Policy

Use `ProviderMaturity::Stub` when the adapter is compile-only and not production-ready.

- `capabilities.execute.available` must be `false`.
- `healthcheck.status` should be `degraded` (or `unknown`) with an explicit "stub" summary.
- `execute` should fail with `ProviderErrorCategory::Unavailable` and a stable error code (`not-implemented`).

## Registry Integration Checklist

1. Add crate dependency to `crates/agentctl/Cargo.toml`.
2. Import adapter in `crates/agentctl/src/provider/registry.rs`.
3. Register adapter in `ProviderRegistry::with_builtins()`.
4. Ensure `agentctl provider list --format json` surfaces:
   - `id`
   - `contract_version`
   - `maturity`
   - `status`

## Validation Commands

```bash
cargo test -p agent-provider-<provider>
cargo run -p agentctl -- provider list --format json
cargo run -p agentctl -- provider healthcheck --provider <provider> --format json
```

For full repository gates, run:

```bash
./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh
```

## Release Readiness Notes

Promote a provider from `stub` to `stable` only after:

- Auth/state behavior is implemented and tested.
- Execute pathway is deterministic and covered by integration tests.
- Docs and dependency/runtime notes are updated.
- CI passes on Linux and macOS targets.

## Stable Promotion Checklist (Gemini reference)

`agent-provider-gemini` is the reference stable lane for this checklist:

1. Runtime dependency edge is `agent-provider-gemini -> gemini-core` only (no `gemini-cli` import).
2. `metadata().maturity` is `stable` and surfaced as stable in `agentctl provider list`.
3. `capabilities`, `healthcheck`, `execute`, `limits`, and `auth-state` are contract-tested.
4. Execute/auth failures map to stable category/code taxonomy.
5. Provider docs include contract spec + verification oracle runbook.
