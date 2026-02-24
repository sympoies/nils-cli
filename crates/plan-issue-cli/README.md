# plan-issue-cli

## Overview
`plan-issue-cli` is the contract-first workspace crate for the Rust replacement of
`plan-issue-delivery-loop.sh`.

Sprint 1 establishes the v1 command contract, gate semantics, and deterministic artifacts needed
to preserve orchestration compatibility before implementation cutover.

## Status
- Current scope is documentation + parity fixtures.
- Runtime command implementation is intentionally deferred to later tasks.

## Specifications
- [CLI contract v1](docs/specs/plan-issue-cli-contract-v1.md)
- [State machine and gate invariants v1](docs/specs/plan-issue-state-machine-v1.md)
- [Gate matrix v1](docs/specs/plan-issue-gate-matrix-v1.md)

## Fixtures
- Shell parity fixtures live under `tests/fixtures/shell_parity/`.
- Use `tests/fixtures/shell_parity/regenerate.sh` to refresh fixture snapshots when shell source
  behavior intentionally changes.
