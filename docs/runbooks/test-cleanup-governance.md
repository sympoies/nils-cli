# Test Cleanup Governance

## Purpose

This runbook defines the stale test lifecycle for workspace cleanup and ongoing maintenance.
Use it to decide whether a candidate should be `remove`, `keep`, `rewrite`, or `defer`, and to keep CI guardrails deterministic.

## Stale Test Lifecycle

1. Discover candidates with `bash scripts/dev/workspace-test-stale-audit.sh`.
2. Classify each candidate using one decision mode:
   - `remove`: deterministic stale artifact with replacement coverage already present.
   - `keep`: still protects active behavior, parity, JSON schema, warning text, color handling, or exit semantics.
   - `rewrite`: behavior is still needed but the test/helper/fixture implementation is obsolete.
   - `defer`: evidence is ambiguous (for example macro indirection or reflection risk) and requires manual review.
3. Validate contract safety before merge.
4. Update CI baseline only after reviewed cleanup PRs merge.

## Evidence Rules

Before marking a candidate `remove`, include all of the following:

- Candidate path and symbol evidence from stale-test inventory output.
- Confirmation that `contract-allowlist.tsv` does not protect the candidate path.
- Replacement test evidence when user-visible behavior could change.
- Explicit validation command outputs in the PR (`test-stale-audit`, required checks, and coverage gate).

For `rewrite`, document:

- Why the old test/helper is obsolete.
- Which test now guards the behavior.
- Which command(s) prove parity/contract behavior still pass.

## CI Guardrails

- `bash scripts/ci/test-stale-audit.sh --strict`
  - Fails on new orphaned helper regressions relative to `scripts/ci/test-stale-audit-baseline.tsv`.
  - Fails on deprecated-path leftovers (`deprecated_path_marker`) in the current inventory.
- `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - Must pass before delivery.
- Coverage gate (non-docs changes):
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Reviewer Checklist

- Decision mode selected: `remove`, `keep`, `rewrite`, or `defer`.
- Evidence links include crate/file/symbol and replacement assertions when required.
- `bash scripts/ci/test-stale-audit.sh --strict` output is clean.
- Required checks entrypoint passes.
- Coverage gate result is attached for non-doc changes.
- Baseline updates are justified and limited to reviewed stale-helper removals.
