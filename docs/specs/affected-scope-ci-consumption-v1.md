# Affected-scope CI consumption v1

## Routes

PR and merge-group Linux lanes invoke the canonical checks entrypoint with
`--ci-merge-plan` and consume the existing
[trusted-base plan](affected-scope-merge-plan-v1.md). PR docs-only and proven
release-only lanes retain their audits and provenance checks. The queue always
invokes the planner on the combined candidate, with full Linux fallback.

After the maintainer's ruleset migration, required names are `test`,
`test_containment`, and `cargo-deny (advisories + bans)`. PRs and queue groups
have no macOS, coverage or deferred-full labeler jobs. The required Linux test
job runs a failure guard even when change detection fails, preventing skipped
needs from admitting a candidate. No plan or receipt alone permits merge.

Full Linux/macOS validation and the 85% workspace coverage floor run nightly,
on manual full CI on main, and before release builds on the exact tagged SHA.
See the [delivery flow](../runbooks/delivery-and-releases.md) for migration and recovery.

## Inputs and execution

A PR binds M to immutable `github.sha` and H to its reviewed event head. B is
M's first parent, the target snapshot actually checked out and tested; event
base metadata can lag that candidate when main advances and is retained
separately as `event_base` provenance. The platform-generated candidate must
still have exactly ordered parents B/H, and local HEAD must equal M. The
launcher and selector are read from that actual B, with no ancestry tolerance,
alternate checkout, or substitution of a newly queried remote ref.

The hosted merge-group lane consumes the same planner. A merge group binds B
to `merge_group.base_sha` and M to `merge_group.head_sha`, which must match the
checkout. A two-parent queue candidate directly supplies H from its second
parent. A single-parent squash queue uses its canonical reserved PR reference
to resolve the exact pull-request head. A local merge-tree proof must equal the
actual checked-out tree before an unreferenced two-parent planning object is
created. That object has ordered B/H parents and the actual tested tree; it
never replaces the checkout or moves a ref.

The receipt binds both the planning
object and actual checkout. Source races, unavailable provenance, attributes,
or configured external merge drivers use explicit full fallback. Unknown base
or event/checkout mismatch holds validation. T is read from the candidate.
Plan input, digest, or package mismatch also holds. Tracked files must be clean.

The launcher comes from B and reads the accepted selector, policy, registry,
schema, and exact trees. Candidate launcher bytes never select a plan. Offline
planning consumes locked metadata offline, without compilation. CI first
prepares locked all-feature metadata for both exact snapshots using trusted
base tooling, so a cold restored Cargo cache does not force a metadata HOLD.
Preparation never replaces the final offline metadata evidence.

Docs and release change detection use the same candidate first parent for
their diff and trusted release policy. Wrong reviewed-head or checkout binding
holds detection before a reduced lane can be selected.

Nonempty `READY/VALIDATE/SCOPED` runs strict Clippy for the selected packages,
builds their binaries with all features, plus the binaries of unselected packages
that their tests resolve (found by `scripts/ci/lib/sibling_bins.py`; built, never
tested), audits selected completion-required binaries against committed assets
with the existing freshness and flag-parity audits (`--skip-build --bin <name>`,
no workspace build), and runs their nextest tests. Linux runs selected doctests. Formatting
and test identity isolation remain.

The scoped lane also runs every cheap, non-compiling workspace audit that a
crate-only change can break, before compiling: documentation placement,
hygiene, prose and Markdown lint, CLI output contract, forge-cli fixtures, file
size, test-stale, tempdir leaks, completion assets and the zsh completion suite.
The three audit baselines are audit-only paths, so they never force FULL.

Accepted policy escalates other scripts, unmapped completions, shared helpers,
manifests and sensitive sources to the complete Linux stack. Generated
`completions/zsh/_<bin>` and `completions/bash/<bin>` assets follow the package
that owns the binary. Unknown squash provenance or a mismatch between
reconstructed source and the actual combined tree uses full fallback rather
than granting scoped credit.

Nightly and release full validation use the canonical full entrypoint directly;
there is no docs, release-only, preview or affected-scope shortcut. Their final
failure guard requires Linux, macOS, coverage, containment and dependency policy
success, including nonempty LCOV from the instrumented macOS run. It rejects
failed, cancelled, skipped and missing lanes. Release build checkouts and all
full-validation checkouts pin the same immutable SHA.

## Evidence and provenance

Every PR and merge-group run also appends the effective scope, selected
packages, fallback and a per-code reason table to the job summary, so a
reviewer sees why a run was FULL without opening the artifact. Changed paths
are rewritten to inert characters and examples are bounded.

The Linux artifact retains `target/ci-merge-plan/plan.json` when available
and `validation.json`. Executed `nils-cli.merge-validation.v1` receipts bind
planning B/H/M/T, actual checkout/tree, captured event base, plan digest,
effective scope, packages, fallback, runner, doctest choice, command, exit code
and PASS/FAIL. A diagnostic `--plan-only` invocation remains available for PRs;
it has no validation receipt and proves no tests ran. Hosted jobs no longer
use it to schedule or label macOS work. Deferred receipts are no longer emitted.

PASS is written only after the actual executor succeeds. Planner HOLD and passing full validation are separate
facts. Executors do not receive `GITHUB_OUTPUT`; the consumer replaces the
step's output with its authoritative scalar scope after success, or empties it
after failure. Executor-written multiline framing cannot change that scope.

Full-validation markers in the Linux affected lane require actual FULL
execution. They do not authorize releases. Releases execute the separate shared
full workflow on their tagged SHA rather than reusing PR or queue receipts.

## Validation and measurement

`python3 scripts/ci/tests/merge-plan-ci.test.py` executes real workflow scripts,
the accepted planner on synthetic Git candidates, and the canonical entrypoint
with fake Cargo/admission. It covers event binding, selected command arguments,
identity and launcher isolation, squash queue
equivalence and source races, external-driver/attribute holds, escalation/HOLD,
missing base policy, malformed inputs, dirty source, failed checks, empty scope,
admission reentry, and full-marker provenance. It does not claim live compiler,
macOS, coverage, or hosted acceptance.

`python3 scripts/ci/tests/merge-plan-policy.test.py` runs the shipped policy and
registry over a synthetic workspace graph with real package and binary names.
It covers audit-only baselines, completion owners, unknown or ambiguous owners,
and the forge-cli, agent-session and semantic-commit scopes. The merge-plan-ci
suite covers the scoped audits and job summary.

`python3 scripts/ci/tests/merge-plan-ci-repairs.test.py` retains stale/regenerated
selected completion controls, unavailable metadata failure, and fake executor
output-framing regressions using the same offline Git/Cargo fixture.

`python3 scripts/ci/tests/merge-plan-ci-snapshot.test.py` captures main advancing
between event capture and checkout. It verifies actual-base selector loading,
SCOPED and FULL routing, preserved event provenance, and strict reviewed-head,
checkout and merge-group binding controls. The actual workflow detection script
also verifies docs/release routing against that snapshot and rejects wrong
reviewed-head and checkout inputs before policy reads.

Rollout compares required-check duration, ready-to-merged time, and merges per
day against preceding full runs, retaining exact inputs, scope, fallback, and
real check results. Synthetic package counts are not throughput measurements.
Missed scopes use the automatic nightly tracking issue and maintainer bisect/revert path,
followed by a selector regression. This consumer grants no release authority.

The standalone scoped-coverage helper remains available for explicit callers;
it is not a PR or queue lane.
`python3 scripts/ci/tests/scoped-coverage-ratchet.test.py` runs the same offline
consumer fixture with controlled LCOV producers. It covers decreasing, equal
and increasing percentages, immutable base selection, identical package scope,
separate profile directories, baseline/candidate failures, fresh report evidence,
exact fraction comparison and the unchanged full-workspace floor.
