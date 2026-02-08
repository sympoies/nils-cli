---
name: nils-cli-refactor-foundations
description: Find and implement high-value test/stability/shared-foundation refactors across crates, then deliver via create-feature-pr.
---

# Nils CLI Refactor Foundations

## Contract

Prereqs:

- Run inside the `nils-cli` git work tree.
- Rust toolchain available on `PATH` (`cargo`, `rustfmt`, `clippy`).
- `git`, `gh`, `semantic-commit`, and `git-scope` available when creating the delivery PR.
- Use this skill together with:
  - `$create-feature-pr` for branch/commit/PR delivery.

Inputs:

- Optional scope hints:
  - target crate(s) to prioritize
  - constraints (time, risk tolerance, out-of-scope areas)
- Optional quality priorities:
  - `coverage-first`, `stability-first`, `shared-extraction-first`

Outputs:

- One of two outcomes:
  - `Implement`: at least one high-value refactor is implemented with tests and validation evidence.
  - `No Action`: no high-value target found; return concrete recommendations and potential issue list.
- Standardized report formatted from:
  - `.codex/skills/nils-cli-refactor-foundations/references/IMPLEMENTATION_RESPONSE_TEMPLATE.md`
  - `.codex/skills/nils-cli-refactor-foundations/references/NO_ACTION_RESPONSE_TEMPLATE.md`
- Delivery via `$create-feature-pr` when code changes are implemented.

Exit codes:

- `0`: completed workflow (implemented changes or no-action report)
- `1`: command/runtime failure while executing workflow
- `2`: usage/scope ambiguity that blocks safe execution

Failure modes:

- No candidate passes the value gate (avoid refactor-for-refactor).
- Candidate requires behavior changes that break parity expectations.
- Shared extraction crosses crate boundaries with unclear ownership or high regression risk.
- Unable to run required validation commands in the current environment.

## Scripts (only entrypoints)

- `.codex/skills/nils-cli-refactor-foundations/scripts/render-refactor-response-template.sh`

## Workflow

1. Build candidate inventory (all crates, evidence-first)

- Review each crate for:
  - missing tests around observable behavior, edge cases, and error paths
  - flaky or brittle logic (implicit assumptions, weak error handling, unstable output contracts)
  - duplicated domain-neutral helpers that could move into shared foundations crates:
    - `crates/nils-common`
    - `crates/nils-term`
    - `crates/nils-test-support`
- Capture each candidate with concrete evidence (file path + why it matters).

2. Apply the value gate (must pass before any refactor)

- A candidate is implementable only if it satisfies at least one:
  - improves correctness/stability for user-visible behavior
  - adds meaningful coverage for uncovered critical paths
  - removes duplicated logic used by 2+ crates via shared foundations extraction
- Reject candidates that are style-only, cosmetic-only, or low-impact churn.

3. Decide branch

- If one or more candidates pass:
  - choose smallest high-value slice
  - implement with behavior parity preserved
  - add/expand tests first or alongside code changes
- If none pass:
  - do not refactor
  - produce a no-action recommendations report using the no-action template

4. Implementation rules (when branch is `Implement`)

- Prefer characterization tests before moving logic.
- Keep crate-local adapters for user-facing messages/exit-code policy when extracting shared helpers.
- Extract only domain-neutral primitives into shared foundations crates.
- Avoid bundling unrelated cleanup in the same change set.

5. Validation

- Run targeted tests for touched crates first.
- If scope is broad or cross-crate, run:
  - `./.codex/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Report exact commands and pass/fail status.

6. Delivery (required for implemented changes)

- Use `$create-feature-pr` to:
  - create feature branch
  - commit with semantic commit policy
  - push and open PR with summary, changes, testing, and risk notes

7. Response contract (always required)

- `Implement` path: use the implementation template.
- `No Action` path: use the no-action template with concrete recommendation list and potential issues.
- Render helpers:
  - `./.codex/skills/nils-cli-refactor-foundations/scripts/render-refactor-response-template.sh --mode implement`
  - `./.codex/skills/nils-cli-refactor-foundations/scripts/render-refactor-response-template.sh --mode no-action`
