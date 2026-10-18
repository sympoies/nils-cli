# Affected-scope CI consumption v1

## Routes

PR code lanes invoke the canonical checks entrypoint with `--ci-merge-plan`.
They consume the existing [trusted-base plan](affected-scope-merge-plan-v1.md),
with no second dependency selector. Documentation-only and proven release-only
PRs retain their substantive audits and provenance checks. Merge groups,
nightly schedules and explicitly dispatched workflows run the full Linux and
macOS stacks, including the absolute 85% workspace line-coverage floor. Queue
candidates cannot select the docs-only, release-only or scoped shortcuts.

Required checks remain `test`, `test_macos`, and `coverage`. Existing advisory
and containment lanes retain their workflow dependencies; their work is
additional to the selected Rust scope. No plan or receipt alone permits merge.

## Inputs and execution

A PR binds M to immutable `github.sha` and H to its reviewed event head. B is
M's first parent, the target snapshot actually checked out and tested; event
base metadata can lag that candidate when main advances and is retained
separately as `event_base` provenance. The platform-generated candidate must
still have exactly ordered parents B/H, and local HEAD must equal M. The
launcher and selector are read from that actual B, with no ancestry tolerance,
alternate checkout, or substitution of a newly queried remote ref.

The consumer also supports merge-group planning for diagnostics and legacy
callers; the hosted merge-group workflow always runs full checks directly.
In that diagnostic route, a merge group binds B to `merge_group.base_sha` and M to `merge_group.head_sha`, which must match the
checkout. A two-parent queue candidate directly supplies H from its second
parent. A single-parent squash queue uses its canonical reserved PR reference
to resolve the exact pull-request head. A local merge-tree proof must equal the
actual checked-out tree before an unreferenced two-parent planning object is
created. That object has ordered B/H parents and the actual tested tree; it
never replaces the checkout or moves a ref. The receipt binds both the planning
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
builds their binaries with all features for sibling fixtures, audits selected
completion-required binaries against committed assets with the existing
freshness audit and no additional build, and runs their nextest tests. Linux
runs selected doctests. macOS runs selected instrumented tests for LCOV,
explicitly excluding unselected workspace packages from the report.

The selected-package aggregate line percentage must not decrease versus the exact
immutable base. Both snapshots run the same selected packages, all features,
nextest profile, and report exclusions on the same runner; no rounded tolerance
is applied. Test identity isolation, cheap workspace audits and formatting
remain. The unchanged policy escalates script, completion,
shared helper, manifest, and sensitive-source changes to the complete stack.

Scoped coverage measures the base freshly in a temporary local clone, with
separate Cargo object/profile directories to avoid duplicate source mappings.
It does not reuse cross-platform or cached report percentages. Missing base,
failed baseline/candidate tests, or empty/invalid LCOV fails closed. The candidate
checkout and refs stay unchanged; the clone is removed on success or failure.
`target/ci-merge-plan/scoped-coverage/` retains the baseline LCOV and a ratchet
receipt with base/candidate commits, identical arguments, counts and verdict.

On Linux PRs, FULL, empty selection and valid planner HOLD use the existing
full entrypoint. The macOS PR lane opts into `NILS_CLI_PR_DEFER_FULL=1`:
when a full stack would be required, it executes no full tests or coverage and
records `DEFERRED_FULL` / `DEFERRED`, with the explicit explanation
`deferred-full: runs in merge group`. The required job names stay `test_macos`
and `coverage`; their step names, annotations and summaries distinguish this
queue prerequisite from a full test pass. A macOS metadata HOLD remains HOLD
in the plan artifact. Invalid protocol or candidate bindings still fail.
The deferral opt-in has no effect for Linux or merge-group callers.

SCOPED PRs still require the instrumented immutable-base coverage ratchet.
`coverage` fails if Linux reports SCOPED but macOS attempts to defer that
measurement. Missing LCOV or a decrease still fails. Merge groups always
execute full tests and the absolute 85% floor, regardless of any PR deferral.
A failed required queue check prevents the candidate from landing; the delivery
owner investigates the original failure before retrying or requeueing. Nightly
full-suite failures retain the existing alert and bisect/revert path.

| Required name | SCOPED PR candidate | FULL/HOLD PR candidate | Merge group / nightly |
| --- | --- | --- | --- |
| `test` | Selected Linux checks and doctests | Full Linux fallback | Full Linux checks and doctests |
| `test_macos` | Selected instrumented tests and base ratchet | Explicit deferred-full status | Full instrumented macOS tests, 85% floor |
| `coverage` | Selected LCOV and containment success | Explicit deferred-full status and containment success | Full LCOV and containment success |

Source review binds the PR candidate and selected evidence. Full macOS and
coverage proof binds the queue candidate that will reach main. No required
check name, ruleset or branch-protection configuration changes with this route.

## Evidence and provenance

Platform artifacts retain `target/ci-merge-plan/plan.json` when available and
`validation.json`. The latter binds planning B/H/M/T, actual checkout/tree,
captured event base, plan digest, effective scope,
packages, fallback, runner, doctest choice, coverage policy/base or absolute
floor, command, exit code, and PASS/FAIL. Existing executed receipts retain
`nils-cli.merge-validation.v1`. Deferred receipts use the additive
`nils-cli.merge-validation.v2` contract, with `status: DEFERRED`,
`validation_scope: DEFERRED_FULL`, an empty command, no coverage policy/floor,
and a `deferred_reason`. Exit 0 acknowledges deferral; it proves no tests ran.
PASS is written only after the actual executor succeeds. Planner HOLD and passing full validation are separate
facts. Executors do not receive `GITHUB_OUTPUT`; the consumer replaces the
step's output with its authoritative scalar scope after success, or empties it
after failure. Executor-written multiline framing cannot change that scope.

Full-validation markers are emitted only for actual FULL validation or
merge-group/nightly/on-demand full routes, so scoped or deferred checks cannot
prove full base CI to the release gate. Executed coverage requires macOS LCOV
and containment success; a deferred PR never downloads or publishes LCOV.

## Validation and measurement

`python3 scripts/ci/tests/merge-plan-ci.test.py` executes real workflow scripts,
the accepted planner on synthetic Git candidates, and the canonical entrypoint
with fake Cargo/admission. It covers event binding, selected command arguments,
identity and launcher isolation, squash queue
equivalence and source races, external-driver/attribute holds, escalation/HOLD,
missing base policy, malformed inputs, dirty source, failed checks, empty scope,
admission reentry, and full-marker provenance. It does not claim live compiler,
macOS, coverage, or hosted acceptance.

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
Missed scopes use the existing nightly alert and maintainer bisect/revert path,
followed by a selector regression. This consumer grants no release authority.

`python3 scripts/ci/tests/scoped-coverage-ratchet.test.py` runs the same offline
consumer fixture with controlled LCOV producers. It covers decreasing, equal
and increasing percentages, immutable base selection, identical package scope,
separate profile directories, baseline/candidate failures, fresh report evidence,
exact fraction comparison and the unchanged full-workspace floor.
