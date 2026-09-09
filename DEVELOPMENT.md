# Development Guide

Principles and routine workflow for maintaining the `nils-cli` workspace.
Detailed setup, command inventories, validation lanes, coverage, generated
artifacts, and release/publish procedures live in the
[`workspace maintenance reference`](docs/runbooks/workspace-maintenance-reference.md).

## Maintenance principles

- Keep reusable behavior in the owning shared crate and preserve documented
  crate boundaries; do not duplicate cross-workspace policy in individual
  binaries.
- Treat agents and system automation as first-class CLI consumers. Preserve
  versioned schemas, stable error codes, deterministic exit status, bounded
  redacted diagnostics, and typed recovery metadata on new or touched paths.
- Keep work authorization distinct from collision awareness in
  `agent-session`: advisory coordination does not deny work, while strict
  admission belongs only to an explicit enforce launch.
- Keep source, completions, generated third-party artifacts, release metadata,
  and public documentation synchronized with their canonical owners.
- Every user-facing CLI exposes root `-V, --version`; clap parsers use
  `#[command(version)]` and show the flag in `--help`.
- In Rust tests, prefer `pretty_assertions::{assert_eq, assert_ne}` for useful
  diffs.
- Use changed-scope validation for the local loop. Full workspace, coverage,
  supply-chain, release, and live acceptance gates remain task-specific or
  CI-owned.

## Change workflow

1. Classify the change by crate, shared workspace surface, completion asset,
   generated artifact, documentation owner, coordination contract, or release
   boundary.
2. Inspect affected callers, tests, schemas, manifests, scripts, generated
   outputs, and canonical docs before editing.
3. Capture a meaningful regression failure for testable behavior when
   practical. Documentation-only work validates ownership, links, lint, and
   routing instead.
4. Make the smallest observable change and keep every affected package,
   completion, generated artifact, and contract test aligned.
5. Run focused checks while iterating, then the default local finish-line:

   ```bash
   bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast
   ```

6. Use the docs-only lane only when every changed path is documentation:

   ```bash
   bash scripts/ci/nils-cli-checks-entrypoint.sh --docs-only
   ```

7. Run full CI parity or coverage locally only for release-quality evidence,
   coverage maintenance, CI debugging, or an explicit request. GitHub required
   checks `test`, `test_macos`, and `coverage` remain the merge gate.

## Documentation routing

| Need | Canonical document |
| --- | --- |
| Setup, builds, validation details, CI inventory, coverage, and publishing | [`Workspace maintenance reference`](docs/runbooks/workspace-maintenance-reference.md) |
| Runtime dependencies and degradation | [`BINARY_DEPENDENCIES.md`](BINARY_DEPENDENCIES.md) |
| CLI completion and alias policy | [`CLI completion standard`](docs/runbooks/cli-completion-development-standard.md) |
| New or redesigned CLI crates | [`New CLI crate standard`](docs/runbooks/new-cli-crate-development-standard.md) |
| Documentation ownership and placement | [`Crate docs placement policy`](docs/specs/crate-docs-placement-policy.md) |
| `agent-session` coordination | [`agent-session` docs](crates/agent-session/docs/README.md) |
| Temporary-directory cleanup | [`Test temp-directory policy`](docs/specs/test-temp-directory-policy.md) |

`AGENTS.md` owns agent-specific repository rules. Plans, discussions, and
reports are retained evidence; they do not override current source, schemas,
policy, or canonical runbooks.
