# Workspace Maintenance Reference

Detailed command and operational reference for contributors maintaining
`nils-cli`. Start with the principles and routine workflow in
[`DEVELOPMENT.md`](../../DEVELOPMENT.md); use this runbook for:

- environment setup
- local test/check execution
- local development validation and CI delivery gates

For runtime dependency details and degradation behavior, see
[`BINARY_DEPENDENCIES.md`](../../BINARY_DEPENDENCIES.md).

## 1. Environment setup

### 1.1 Recommended bootstrap

Run once on a new machine:

```bash
scripts/setup-rust-tooling.sh
```

This installs/updates:

- rustup + cargo
- Rust components: `rustfmt`, `clippy`, `llvm-tools-preview`
- `cargo-nextest`
- `cargo-llvm-cov`

### 1.2 Minimum tools required for local checks

All local check flows assume `bash` is available to run repo scripts.

Docs-only checks require:

- `git`
- `npx`
- `python3`

Local fast changed-scope checks also require:

- `python3`
- `cargo` for non-document changes

CI/full parity checks also require:

- `cargo`
- `python3`
- `zsh`
- `rg`
- `cargo-nextest` when `NILS_CLI_TEST_RUNNER=nextest`

Coverage checks also require:

- `cargo-llvm-cov`
- `cargo-nextest`

For optional runtime tools used by individual CLIs, see `BINARY_DEPENDENCIES.md`.

## 2. Build and quick smoke checks

- Build workspace: `cargo build`
- Example CLI help checks:
  - `cargo run -p nils-cli-template -- --help`
  - `cargo run -p nils-git-scope -- --help`

### 2.1 Codex skill-surface shape checks

`agent-runtime doctor --class skill-surface --product codex` validates
install-map shape only. A passing shape check is not Codex Desktop acceptance;
live acceptance still requires `codex debug prompt-input` in a fresh Codex
Desktop session with `$HOME/.agents` absent and retired skill-related
environment variables unset. Rationale: see the archived plan bundle
`agent-plan-archive:plans/github.com/sympoies/nils-cli/2026-05-23-codex-skill-surface-primitives/`.

### 2.2 `agent-session` coordination changes

Before changing coordination behavior, read the crate-local
[`agent-session` documentation index](../../crates/agent-session/docs/README.md)
and the
[Session Coordination V1 contract](../../crates/agent-session/docs/specs/session-coordination-v1.md).
This repository owns the CLI, automatic presence, work-context, and protocol
contracts. `sympoies/agent-runtime-kit` owns the global agent policy and hook
consumer behavior.

Keep work authorization distinct from collision awareness: default `advisory`
coordination and unmanaged sessions require no claim and never deny work;
`work-context set` only improves overlap metadata; strict claim/admission
semantics apply only to an explicit `enforce` launch. Changes to that boundary
require coupled nils-cli/runtime-kit validation.

### 2.3 Agent- and system-facing CLI UX

Treat agents and system automation as first-class CLI consumers while keeping
interactive text mode clear for humans. For new or touched automation-facing
failures, contributors must:

- preserve the exact versioned schema and evolve it additively within a schema
  version;
- publish a stable machine error code with deterministic exit status;
- expose typed `error.details.retryable`, `error.details.next_action`, and a
  bounded `error.details.recovery` object on recoverable touched paths;
- derive clear text output from the same typed failure model;
- bound and redact diagnostic fields, excluding secrets, raw session IDs,
  absolute local state or project paths, private content, and command dumps;
- require automation to branch on structured fields, never `message` or
  `hint`;
- cover representative failures, field types, exit status, and leakage in
  contract tests.

This contract applies to new or touched automation-facing failures. Existing
CLIs are audited incrementally; the first bounded implementation covers
`agent-docs session` errors rather than claiming a workspace-wide migration is
already complete.

## 3. Canonical validation flows

Local development defaults to changed-scope validation. The full workspace test
stack and coverage gate are CI responsibilities for normal PRs; run them
locally only when you need CI parity, release-quality verification, coverage
maintenance, or explicit debugging evidence.

Primary local entrypoint for day-to-day implementation work:

```bash
bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast
```

This runs changed-scope validation against `origin/main` by default:

- documentation-only changes use the docs-only lane
- non-shared crate changes run package-scoped `fmt`, `clippy`, and tests
- shared crates and workspace-level files escalate to the workspace Rust gate
- the workspace lane runs its tests through `scripts/ci/tempdir-leak-probe.sh`,
  which fails on temp directories the suite leaves behind; see
  `docs/specs/test-temp-directory-policy.md`
- a package-scoped lane does not build binaries another package owns, so a test
  that needs one skips with a reason naming the build command; see
  `bin::sibling_or_skip` in `crates/nils-test-support`. Build the sibling — for
  example `cargo build -p nils-claude-cli --bins` — to run those tests locally.
  A *stale* sibling fails instead of skipping, because an artifact from an
  earlier release is an operator error rather than a property of the run. The
  workspace lane and CI build every default-feature binary, and CI additionally
  sets `NILS_TEST_REQUIRE_SIBLING_BINS=1` so a skip there is a hard failure
  rather than a silent loss of coverage

Override the base ref when needed:

```bash
bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast --base main
```

Use `--plan-only` to inspect the selected validation scope without running it:

```bash
bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast --plan-only
```

The canonical CI/full-check entrypoint remains:

```bash
bash scripts/ci/nils-cli-checks-entrypoint.sh
```

This delegates to `./.agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh`.
It is what CI uses for the full `test` and `test_macos` jobs; it is not the
default local development loop.

### Integration batches on `next`

The maintainer may approve a small batch of low-risk changes on the persistent
`next` branch to share complete local validation. Changed-scope checks still
belong to each member. Lifecycle, mailbox, credential, and release changes use
individual PRs to `main`; keep other changes with comparable risk out of a
batch. Required hosted checks and branch protections continue to apply.

1. Open one tracking issue per batch. It lists each member with its exact
   head, the reconciliation PR, the frozen head, the gate result, and the
   integration PR; whoever routes the batch keeps it current.
   Select the batch members and record their exact PR head SHAs, with both
   changed-scope validation PASS and designated review PASS on those heads. Resolve review findings
   and check that each member applies to the current integration base before
   admitting it. Changed heads require fresh review and validation.
2. Assign validation to one platform lane per item at a time. Keep Linux
   systemd, cgroup, and containment checks on Linux; use macOS for applicable
   platform checks. A platform result covers only its recorded head and
   command. Follow the host's resource limits and admission policy.
3. Retarget approved member PRs to `next` and merge them through `forge-cli`,
   pinning the expected head and base with `--expected-head` and
   `--expected-base next` and the explicit `--allow-non-default-base` opt-in.
   Recheck review and required checks after retargeting;
   stop if either expectation changes. Record each resulting merge commit and
   its member evidence. Do not bypass branch protection.
4. Freeze the batch at one exact `next` head after the selected members land.
   Stop adding members while it is being validated. Run one canonical complete
   local gate from a clean managed worktree at that head:

   ```bash
   NILS_CLI_TEST_RUNNER=nextest bash .agents/scripts/pre-pr.sh --full
   ```

   Use the shared gate semaphore described below and the host's configured limits.
   A package check, partial run, or a passing member does not establish a
   complete batch PASS.
5. Retain the original terminal outcome, logs, selected scope, counts, and
   source identity. A resource abort or cancelled run is incomplete. On a
   failure, stop promotion and isolate the cause with separately admitted,
   bounded checks or a bisect of batch members. Do not infer a culprit from
   merge order or erase a failed result with a retry.
6. Repair or eject a confirmed culprit through a reviewed successor or revert
   PR to `next`. Never reset or force-push the integration branch. Freeze the
   resulting head again and run its complete gate; earlier results do not
   transfer to that head.
7. Open the integration PR to `main` only at the validated frozen head; do not
   keep a persistent integration PR open between batches. `forge-cli pr create`
   accepts only typed head branches (`chore/<slug>` and the other kinds), so
   cut a `chore/` branch at the exact frozen `next` SHA and open the PR from it.
   The head SHA, not the branch name, is what the gate result and
   `--expected-head` bind. Require review and green hosted `test`,
   `test_macos`, and `coverage` checks before an expected-head merge with
   `--expected-base main --keep-branch`.

   If `main` advances and reconciliation is needed, open and review a PR into
   `next` from a `chore/` branch that carries the merge of `main` into `next`,
   with any conflicts resolved there. Merge it using the guarded form in step 3
   with `--expected-head`, `--expected-base next`, `--allow-non-default-base`,
   and `--keep-branch`. Then freeze and validate the new integration head.
   Keep the `next` branch for subsequent batches.
8. Record the promoted integration head and member outcomes in the tracking
   issue and the delivery handoff. A batch merge does not authorize tagging,
   publishing, installing, or deployment; follow the existing release workflow
   when requested.

### Gate resource budgets

Executing code gates, including local-fast and direct required-checks script
calls, share a FIFO semaphore under
`${XDG_STATE_HOME:-$HOME/.local/state}/nils-cli/resources`. The semaphore is
shared across worktrees and repositories using this entrypoint for the same
user on a host. Kernel `flock` ownership and recorded unit quiescence govern admission:
crashed holders and waiters do not require deleting lock files. A crashed
supervisor slot remains unavailable while its scope/services are draining. Waiting callers print the holder
PID and UTC start time every five seconds; admission times out after one hour.
Help, plan-only inspection, and the explicit docs-only lane do not take a slot.

| Knob | Default | Purpose |
| --- | --- | --- |
| `NILS_CLI_GATE_SLOTS` | `1` | Maximum concurrent code gates for the user on this host. |
| `NILS_CLI_GATE_TIMEOUT_SECONDS` | `3600` | Bounded gate and contained-runner queue wait. |
| `NILS_CLI_RUNNER_MAX` | `max(1, min(2, CPU count / 4, MemAvailable / 4 GiB))` | Host contained-runner slots and upper bound on nextest/libtest concurrency; integer division. |
| `CARGO_BUILD_JOBS` | `1` | Cargo compilation concurrency, including coverage compilation; clamped to the runner maximum. |
| `NILS_CLI_GATE_MEMORY_MAX_GIB` | `16` | Linux aggregate hard memory cap per gate. |
| `NILS_CLI_GATE_MEMORY_HIGH_GIB` | `75%` of the hard cap, rounded down, minimum `1` | Linux memory throttle threshold (`12` GiB with defaults). |
| `NILS_CLI_GATE_MIN_AVAILABLE_GIB` | `20` | Linux admission requires at least this much `MemAvailable`; waits within the same timeout. A floor above `MemTotal` fails immediately. |

These defaults intentionally serialize ordinary `agent-hook` finish-line
workloads across checkouts, including small commands. The 16 GiB no-swap cap
and bounded one-hour admission wait are the default resource policy. Configure
the shared runner and gate limits together to permit additional concurrency.
If the admission floor exceeds the host's total memory, the wrapper rejects it
before joining FIFO; lower the floor and memory caps together on a smaller
dedicated host.

The default expected peak **budget** for a complete gate is 12–16 GiB,
including compilation, linking, tests, and their contained services. This is a
capacity planning allowance, not a measured guarantee for every toolchain or
coverage build: two runners receive an allowance of about 4 GiB each, with the
remaining space for compilation and filesystem cache. Linux caps aggregate
cgroup-accounted memory at 16 GiB and disables swap for the gate. A workload
that cannot fit fails its own gate. Observe the gate slice's `MemoryPeak` with
`systemctl --user show <slice> -p MemoryPeak` while it runs before increasing
budgets; a failed/collected slice may no longer retain that measurement.

Linux requires cgroup v2 and a working systemd user manager. Each gate runs in a
user scope; contained services join that scope's unique parent slice so user
manager launches are included in the aggregate cap. Both the scope and slice
receive `MemoryHigh`, `MemoryMax`, and `MemorySwapMax=0`; the scope and
contained services also use `OOMPolicy=kill`.
Missing containment fails closed. The outer wrapper stops the slice when the
gate exits, including after a memory failure. On macOS the memory scope and
`MemAvailable` admission floor are documented no-ops; semaphore and Cargo/test
limits still apply. Interrupting the macOS supervisor terminates its workload
process group before releasing admission. This fallback does not change
`agent-hook`'s Linux-only contained-execution support.

Use consistent semaphore and runner settings across callers on the same host.
For example, on a host with capacity reserved for two gates:

```bash
NILS_CLI_GATE_SLOTS=2 NILS_CLI_RUNNER_MAX=2 \
  NILS_CLI_GATE_MEMORY_MAX_GIB=16 CARGO_BUILD_JOBS=1 \
  bash scripts/ci/nils-cli-checks-entrypoint.sh
```

Budget at least `gate slots × memory cap`, plus capacity for interactive work.
The runner cap is shared across gates; raising gate slots does not multiply it.
With a cap of at least two, one slot is reserved for admitted-gate children;
ordinary contained commands use the remaining slots. This prevents queued outer
gates from consuming all capacity needed by an admitted gate. Gate children
prefer the reserved slot and may also use any free ordinary slot, preserving
the total cap. A complete gate already inside a contained runner requires
`NILS_CLI_RUNNER_MAX` of at least two and fails immediately otherwise. Use
`NEXTEST_TEST_THREADS=1` with a runner cap of two for a nested gate with one
test workload at a time.
On a host with fewer than eight CPUs, the derived runner maximum is one;
set `NILS_CLI_RUNNER_MAX=2` explicitly when a finish-line contained command
must run a complete gate, while retaining a lower test-thread setting.
For two gates that themselves run as contained commands, a runner cap of four
allows both outer units and their child workloads; keep
`NEXTEST_TEST_THREADS=2`, `RUST_TEST_THREADS=2`, and `CARGO_BUILD_JOBS=1`
to preserve each gate's conservative workload concurrency.
Caller `NEXTEST_TEST_THREADS` and `RUST_TEST_THREADS` values can lower test
concurrency; the wrapper clamps higher values to the runner maximum.
`NILS_CLI_RESOURCE_STATE_DIR` is the resolved host lock directory propagated to
contained runners, so test fixtures changing `HOME` or `XDG_STATE_HOME` cannot
create independent pools. Keep that directory consistent across callers.
`NILS_CLI_GATE_ACTIVE`, `NILS_CLI_GATE_SLICE`, and
`NILS_CLI_CONTAINED_RUNNER_ACTIVE` are internal wrapper state;
ordinary callers should leave them unset.
`NILS_CLI_LOCAL_FAST_PLAN` is an internal one-use planning handoff, removed
before checks run; local-fast resolves docs-only, no-change and usage-error
paths before entering the complete-gate queue.

The dedicated hosted CI lanes explicitly use two runners. Linux uses a smaller
reservation (2 GiB available at admission, 12 GiB hard cap), leaving room for
the operating system within the [standard hosted runner resources](https://docs.github.com/en/actions/reference/runners/github-hosted-runners). Adjust the
memory floor and cap together for smaller dedicated runners, rather than
turning off containment. Raw Cargo invocations are outside gate admission;
use the entrypoint for complete validation.

### 3.1 Docs-only changes fast path

If all changed files are documentation-only (`*.md`, `docs/**`, `crates/*/docs/**`, root docs like `README.md` and `DEVELOPMENT.md`):

```bash
bash scripts/ci/nils-cli-checks-entrypoint.sh --docs-only
```

The docs-only entrypoint additionally runs the CLI output contract lint
(`docs/specs/cli-output-contract-v1.md`) so envelope and exit-code drift gets
caught even on PRs that only touch documentation:

```bash
bash scripts/ci/cli-output-contract-lint.sh --strict
```

The lint has a self-test under
`scripts/ci/tests/cli-output-contract-lint.test.sh` that exercises every
regression class against synthetic fixtures.

In CI, the `changes` job runs `scripts/ci/detect-docs-only.sh` (sharing the
`scripts/ci/lib/doc_classify.py` classifier with the local-fast planner). When
every changed file is documentation, the `test` and `test_macos` jobs run
`--docs-only` and the `coverage` job skips its steps, so docs-only PRs avoid the
full Rust build/test/coverage cost. The skips are step-level, so all three
release-gated checks still report success.

### 3.2 Local fast changed-scope checks

For most local implementation loops:

```bash
bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast
```

Set `NILS_CLI_TEST_RUNNER=nextest` to require `cargo-nextest`; otherwise the
local fast gate auto-selects `cargo nextest` when available and falls back to
`cargo test`.

The local fast gate is conservative. Changes to `nils-common`, `nils-term`,
`nils-test-support`, root manifests, CI scripts, completions, `.agents/`,
`.github/`, or other workspace-level paths use a workspace Rust gate because
package-scoped checks can miss reverse-dependency breakage.

For non-shared crate changes, the selected set includes changed packages and
all transitive workspace reverse dependents from offline, locked Cargo metadata
with all features enabled. Normal, dev, build and target-specific dependencies
participate; traversal can pass through external packages, which are excluded
from the selected set. Binary coupling also expands this
closure. The plan prints each selected package and its inclusion reasons.
Any Cargo manifest or lockfile, toolchain file, `.cargo/` configuration, or
build-script change uses the workspace lane; documentation-only routing stays
the same. The full-check entrypoint and merge gates are unchanged.

### 3.3 Full checks (CI gate / optional local parity)

```bash
NILS_CLI_TEST_RUNNER=nextest bash scripts/ci/nils-cli-checks-entrypoint.sh
```

Notes:

- `nextest` mode runs `cargo nextest run --profile ci --workspace`.
- The `ci` profile terminates a test that is still running after 10 minutes
  (`slow-timeout` in `.config/nextest.toml`) and reports it as `TIMEOUT`, so a
  hung test fails the run instead of holding it. The same bound applies to the
  `llvm-cov` runner and to the local fast gate when it runs nextest. Give a
  test that legitimately needs longer its own override there.
- Because doctests are not included in nextest, the entrypoint also runs
  `cargo test --workspace --doc` when `NILS_CLI_TEST_RUNNER=nextest`.
- `NILS_CLI_SKIP_DOCTESTS=1` skips that doc run in both the `nextest` and
  `llvm-cov` runner modes. CI sets it only on `test_macos` (llvm-cov runner),
  because the Linux `test` job already runs the same doc tests. It does not
  affect the trailing doc run of `--with-coverage`.
- `NILS_CLI_SKIP_OS_INDEPENDENT_AUDITS=1` skips the stale-test and completion
  freshness/parity audits, whose result does not depend on the OS. CI sets it
  only on `test_macos`; the Linux `test` job runs them.

### 3.4 Full coverage flow (CI gate / explicit local parity)

Coverage gate is mandatory in CI for non-doc changes and in explicit
release-quality verification (total line coverage must stay `>= 85.00%`).
In CI, the `test_macos` job runs the workspace tests once under
`NILS_CLI_TEST_RUNNER=llvm-cov`, which enforces the floor, and the `coverage`
job publishes the summary from that run's LCOV artifact.
The Linux cgroup containment and provider-stop canaries run in their own
`test_containment` job beside `test`; `coverage` requires it to succeed.
Normal local development does not need to run coverage before opening a PR:

```bash
NILS_CLI_TEST_RUNNER=nextest \
  bash scripts/ci/nils-cli-checks-entrypoint.sh --with-coverage
```

`--with-coverage` runs, after the full check stack:

```bash
mkdir -p target/coverage
cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85
bash scripts/ci/coverage-summary.sh target/coverage/lcov.info
cargo test --workspace --doc
```

Use the default threshold for CI parity. To run a stricter local check, override
the threshold:

```bash
NILS_CLI_COVERAGE_FAIL_UNDER_LINES=90 bash scripts/ci/nils-cli-checks-entrypoint.sh --with-coverage
```

## 4. Full checks included by the CI entrypoint

`bash scripts/ci/nils-cli-checks-entrypoint.sh` includes:

- `bash scripts/ci/docs-placement-audit.sh --strict`
- `bash scripts/ci/docs-hygiene-audit.sh --strict`
- `bash scripts/ci/docs-prose-audit.sh --strict`
- `bash scripts/ci/markdownlint-audit.sh --strict`
- `bash scripts/ci/cli-output-contract-lint.sh --strict`
- `bash scripts/ci/forge-cli-fixture-lint.sh --strict`
- `bash scripts/ci/tests/install-local-release-binaries.test.sh`
- `bash scripts/ci/tests/completion-freshness-audit.test.sh`
- `bash scripts/ci/tests/completion-flag-parity-audit.test.sh`
- `bash scripts/ci/tests/local-fast-checks.test.sh`
- `bash scripts/ci/tests/detect-docs-only.test.sh`
- `bash scripts/ci/tests/detect-release-only.test.sh`
- `node scripts/ci/tests/release-ci-gate.test.cjs`
- `bash scripts/ci/tests/dependabot-third-party-workflow-contract.test.sh`
- `bash scripts/ci/tests/release-workflow-contract.test.sh`
- `bash scripts/ci/tests/shared-helper-adoption-audit.test.sh`
- `bash scripts/ci/tests/publish-order-audit.test.sh`
- `bash scripts/ci/tests/docs-hygiene-audit.test.sh`
- `bash scripts/ci/tests/workspace-test-stale-audit.test.sh`
- `bash scripts/ci/tests/file-size-audit.test.sh`
- `bash scripts/ci/tests/docs-prose-audit.test.sh`
- `bash scripts/ci/tests/prepare-private-release-workflow.test.sh`
- `bash scripts/ci/skill-shell-suites.sh` (runs every
  `.agents/skills/*/tests/test_*.sh` smoke suite)
- `bash scripts/ci/test-stale-audit.sh --strict`
- `bash scripts/ci/file-size-audit.sh --strict` (see the file-size ratchet below)
- `bash scripts/ci/docs-prose-audit.sh --strict`
- `bash scripts/ci/workspace-version-lockstep.sh --strict`
- `bash scripts/ci/crate-naming-audit.sh`
- `bash scripts/ci/publish-order-audit.sh --strict`
- `bash scripts/ci/third-party-artifacts-audit.sh --strict`
- `bash scripts/ci/completion-asset-audit.sh --strict`
- `bash scripts/ci/completion-freshness-audit.sh --strict`
- `bash scripts/ci/completion-flag-parity-audit.sh --strict`
- `zsh -f tests/zsh/completion.test.zsh`
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --workspace` (or `cargo nextest run --profile ci --workspace`
  plus `cargo test --workspace --doc` when `NILS_CLI_TEST_RUNNER=nextest`)

### File-size ratchet

`scripts/ci/file-size-audit.sh` limits each tracked `crates/**/*.rs` file to
2,000 implementation lines and 3,000 test lines. Test lines are the lines of
top-level `#[cfg(test)]` items and `#[cfg(all(..))]` items with `test` as one
of the top-level `all` arguments (in any position), plus whole files
under `tests/` or `benches/` and files reached through an out-of-line
`#[cfg(test)] mod x;`. An item ends at its first `;` or `{` outside `(..)` and
`[..]`. Comments and literals are skipped when matching braces.

Files already over a limit are recorded in
`scripts/ci/file-size-baseline.tsv` (`path<TAB>kind<TAB>lines`). Strict mode
fails when:

- a file without a baseline row is over a limit;
- a baselined file grew;
- a baseline row is stale: the file is gone, is now within its limit, or shrank.
- the per-kind sum of baseline values rose against the merge base with
  `origin/main` (override with `--base <ref>`). A rename may move its row to the
  new path, but cannot raise the total. When the change set modifies
  `scripts/ci/lib/rust_file_size.py`, the combined implementation-plus-test sum
  is compared instead, so a measurement-rule change may move lines between
  kinds but still cannot raise the total.

When a PR shrinks or removes an over-limit file, refresh the baseline in the same
PR with `bash scripts/ci/file-size-audit.sh --update-baseline` and commit it.

Put new code in new modules, not in baselined files. A baselined file cannot
grow by even one line, so each of its rows must end the PR at or below its
baseline:

- **New module file.** Put the new implementation and its unit tests in a new
  file. A new file has no baseline row and can grow up to the limits. The `mod`
  declaration and the call sites still count against the baselined file, so
  offset them with removals there, or declare the module from a parent file
  without a baseline row.
- **Equal move-out.** Move at least as many lines out of the same file as the
  change adds, then refresh the baseline. Rows only go down.

Run `bash scripts/ci/file-size-audit.sh --strict` before pushing; it needs no
compile. CI audits the PR's merge result each time it runs, so a PR opened
before the ratchet merged fails on its next push or re-run.

### Markdown prose-run ratchet

`scripts/ci/docs-prose-audit.sh` checks tracked Markdown outside `docs/devlog/`
and the generated third-party notices. A prose run is consecutive nonblank
lines outside fenced code; table rows, headings, and HTML comment lines end a
run, and list markers begin a new run. Runs longer than 12 lines are recorded
per file in `scripts/ci/docs-prose-baseline.tsv`
(`path<TAB>runs_over_limit<TAB>longest_run`). Strict mode rejects new or grown
rows, stale rows, and an increase in summed baseline runs from the merge base.
Refresh improvements with `bash scripts/ci/docs-prose-audit.sh
--update-baseline` and commit the baseline in the same PR.

## 4.1 Supply-chain audit (cargo-deny)

A dedicated `cargo-deny` GitHub Actions job (`.github/workflows/ci.yml`) runs on
every push and PR, independent of the Rust test jobs. Run the same gate locally
with:

```bash
bash scripts/ci/cargo-deny-audit.sh   # cargo deny check advisories bans
```

It enforces two policies from the root `deny.toml`:

- **advisories** — any RUSTSEC vulnerability / unsound advisory fails the build.
- **bans** — `multiple-versions = "deny"`: a *new* duplicate crate version fails
  the build. Duplicates that exist today only because of in-progress upstream
  ecosystem transitions are recorded in the `deny.toml` `skip` list (a ratchet).

Requires `cargo-deny` (`cargo install cargo-deny --locked` or `brew install
cargo-deny`). When a new duplicate is unavoidable, add a `skip` entry with a
`reason`; to temporarily accept an advisory, add an `ignore` entry with a
`reason`.

## 5. Additional checks when completion assets change

When completion/alias assets are changed, also run:

- `zsh -n completions/zsh/_<cli>`
- `bash -n completions/bash/<cli>`

Canonical completion policy and validation workflow:

- `docs/runbooks/cli-completion-development-standard.md`

## 6. Generated artifacts

Regenerate third-party license/notice artifacts after dependency or metadata
changes:

```bash
bash scripts/generate-third-party-artifacts.sh --write
```

Verify the generated artifacts are current before delivery:

```bash
bash scripts/generate-third-party-artifacts.sh --check
```

The artifact contract is documented in
`docs/specs/third-party-artifacts-contract-v1.md`.

### 6.1 Dependabot bumps

Every Dependabot cargo bump rewrites `Cargo.lock`, which drifts both artifacts
and fails the strict third-party audit. Two workflows handle that
automatically, split so that the privileged half never touches pull-request
content:

- `.github/workflows/dependabot-third-party-artifacts.yml` runs unprivileged on
  `pull_request` — no secrets, read-only token. It is the only place that
  executes anything from the bump, and it publishes the regenerated files as
  the `third-party-refresh` artifact.
- `.github/workflows/dependabot-third-party-apply.yml` runs privileged on
  `workflow_run`. It checks out nothing, commits the two artifact paths onto the
  Dependabot branch through the Git Data API, and squash merges
  `dependabot/cargo/cargo-minor-patch-*` once the `CI` workflow concludes
  success on that exact commit. Major-version and security bumps are refreshed
  but never auto-merged.

The artifact is untrusted input, so the applying workflow never executes it: it
writes only the two known paths, only onto the branch the run belongs to, and
only while that branch head still matches the commit the refresh was generated
for. CI then re-runs the strict audit on the resulting commit, and the merge
gate requires that run to pass.

The applying workflow requires two repository secrets, `BOT_APP_ID` and
`BOT_APP_PRIVATE_KEY`, for a GitHub App installed on this repository with
`Contents: read and write`, `Pull requests: read and write`, and
`Actions: read`. The App token is mandatory rather than a convenience: a commit
made with the default `GITHUB_TOKEN` does not start a new workflow run, so the
refreshed commit would carry no CI results and could never be shown green.
Without the secrets the applying workflow fails fast with that instruction, and
`.agents/skills/project-deliver-dependabot-bump-pr` remains the manual path.

## 7. Test conventions

- In Rust tests, prefer `pretty_assertions::{assert_eq, assert_ne}` for readable diffs.

## 8. CLI version policy

- Every user-facing CLI must expose root `-V, --version`.
- For clap-based CLIs, set `#[command(version)]` on the root `Parser`.
- `--help` output should show `-V, --version`.

## 9. Local install, release, and publishing

### 9.1 Local release install helper

Build and install workspace binaries into `~/.local/nils-cli/bin` by default:

```bash
./scripts/install-local-release-binaries.sh
```

Install only one binary when you are smoke-checking a focused change:

```bash
./scripts/install-local-release-binaries.sh --bin git-scope
```

Add the install directory to `PATH` when needed:

```bash
export PATH="$HOME/.local/nils-cli/bin:$PATH"
```

### 9.2 GitHub release packaging

Release tags matching `v*` trigger `.github/workflows/release.yml`. The workflow
first verifies the tagged commit has green `test`, `test_macos`, and `coverage`
checks, then builds release tarballs for Linux and macOS on x86_64 and aarch64.

Release tarballs include:

- release-default workspace binaries from `scripts/workspace-bins.sh`
- `completions/zsh/` and `completions/bash/`
- `README.md`, `LICENSE`, `THIRD_PARTY_LICENSES.md`, and `THIRD_PARTY_NOTICES.md`

Releases run through the private sympoies-infra release broker: tagging, the
GitHub Release, the tap update, and fleet convergence are not done by hand from
this repository. Use the `project-release-nils-cli` skill in
`serenvia/sympoies-infra` to release. The repo-owned
`.agents/skills/project-bump-version-tag-release` skill documents only the
version-bump transform the broker consumes.

`.github/workflows/prepare-private-release.yml` is a narrower preparation-only
entrypoint for the private infrastructure orchestrator. It runs the same
canonical version preparation and locked workspace check on a GitHub-hosted
runner, and uploads only a patch plus a checksum-bound
manifest. It has read-only repository permissions, accepts no credentials, and
does not create a branch, PR, tag, or release. The private orchestrator must
bind the artifact to the workflow run's immutable `headSha`, independently
validate the patch semantics, and own all delivery mutations.

### 9.3 crates.io publishing

Local crate publish dry-runs and direct publishes use `scripts/publish-crates.sh`.
The default crate order is `release/crates-io-publish-order.txt`.

```bash
scripts/publish-crates.sh --dry-run
scripts/publish-crates.sh --publish
scripts/publish-crates.sh --crates "nils-term nils-common" --dry-run
```

GitHub workflow dispatch is available through `.github/workflows/publish-crates.yml`.
In `publish` mode the workflow requires the repository secret
`CARGO_REGISTRY_TOKEN`.

Use the repo-owned dispatch helper when you want workflow dispatch with run
reporting and post-run crates.io status snapshots:

```bash
.agents/skills/project-dispatch-crates-io-publish/scripts/publish-crates-io.sh --all --wait
```

To query crates.io publish status locally, use:

```bash
scripts/crates-io-status.sh --all --format text
```

Detailed status-script semantics live in
`docs/runbooks/crates-io-status-script-runbook.md`.

## 10. Development log

`docs/devlog/README.md` owns the development log's conventions, entry template,
and month index. This section owns only the command contract and the validation
lane.

The `devlog` CLI maintains and queries the log:

```bash
devlog new --title "<title>" --result "<bullet>" --why "<bullet>" \
  --evidence "<bullet>" --link "<bullet>"
devlog search <term> [--month YYYY-MM]
devlog check
devlog fix
devlog index
```

`new` inserts newest-first and refreshes the month index in the same operation.
`check` reports structural problems the Markdown lint cannot see: mis-named
month files, index drift, missing or unknown entry sections, date/month
mismatches, and entries that are not newest-first. `fix` repairs the subset of
those that have one correct repair and reports the rest; it is for adopting a
log written before the contract existed, not part of the write path. Exit codes
and the JSON envelope names are documented in `crates/devlog/README.md`.

Devlog-only changes are documentation, so they validate through the docs-only
fast path in section 3.1.
