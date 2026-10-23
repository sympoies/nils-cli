# forge-cli docs

Crate-local documentation for the `forge-cli` binary.

## Specs

- [PR lifecycle metadata](specs/pr-lifecycle-v1.md) — provider timestamps, shared queue-read object, and current merge-group check timing.
- [`specs/checks-scope-and-deadline-v1.md`](specs/checks-scope-and-deadline-v1.md) — additive check scope fields and the whole-operation wait budget.
- [Shared forge identity policy](../../nils-common/docs/specs/forge-identity-policy-v1.md) — identity explain/doctor, enforcement, and rollout interface.
- [`specs/forge-cli-spec-v1.md`](specs/forge-cli-spec-v1.md) — v1 contract,
  parity matrix, lock-down rules, exit-code map.

### Operation contracts by task

- [Publication and administration](specs/forge-publication-admin-v1.md) — release assets, workflow dispatch, comments, labels, and issue trackers.
- [Default-branch publication](specs/forge-default-push-v1.md) — the guarded single-commit default-branch push contract.
- [Checks and review reads](specs/forge-checks-review-reads-v1.md) — wait for checks and read review threads or native reviews.
- [Review transaction and handoff state](specs/forge-review-state-v1.md) — review receipts, handoff ownership, convergence state, and recovery.
- [Review publication and thread writes](specs/forge-review-publication-v1.md) — publish and validate reviews, then reply to or resolve threads.
- [Pull request merge](specs/forge-merge-v1.md) — merge admission, provider compare-and-swap, and branch cleanup.
- [Workflow run reruns](specs/forge-run-rerun-v1.md) — fenced GitHub Actions run or job reruns at an expected head.

### Related specifications

- [`specs/forge-cli-ops-v1.yaml`](specs/forge-cli-ops-v1.yaml) —
  machine-readable op catalog.
- [`specs/local-provider-contract-v1.md`](specs/local-provider-contract-v1.md) —
  file-backed issue and seeded PR store contract.
- [`specs/search-completions-v1.md`](specs/search-completions-v1.md) — raw
  completion facts, exact completion windows, and resumable coverage.

## Runbooks

- [`runbooks/pr-head-repair-loop.md`](runbooks/pr-head-repair-loop.md) —
  keeping one PR and one review timeline across a CI repair loop: amend,
  force-with-lease, re-bind evidence, re-deliver, and the check-registration
  window after a new head.

Workspace-level docs that govern this crate (the envelope contract, the
crate docs placement policy, and the dispatch plan) live under the
repository root `docs/` tree; consult that tree from the workspace
root.

## Links

- Back to crate README: [`../README.md`](../README.md)
