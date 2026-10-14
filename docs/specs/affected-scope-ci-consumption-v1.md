# Affected-scope CI consumption v1

## Routes

The code lanes for `pull_request` and `merge_group` invoke the canonical checks
entrypoint with `--ci-merge-plan`. They consume the existing
[trusted-base plan](affected-scope-merge-plan-v1.md), with no second dependency
selector. Established documentation-only and proven release-only routes retain
their substantive audits and provenance checks. Scheduled and explicitly
dispatched workflows retain full validation.

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

A merge group binds
B to `merge_group.base_sha` and M to `merge_group.head_sha`, which must match the
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
planning uses caller-provided tools/cache without fetching or compilation.

Docs and release change detection use the same candidate first parent for
their diff and trusted release policy. Wrong reviewed-head or checkout binding
holds detection before a reduced lane can be selected.

Nonempty `READY/VALIDATE/SCOPED` runs strict Clippy for the selected packages,
builds their binaries with all features for sibling fixtures, audits selected
completion-required binaries against committed assets with the existing
freshness audit and no additional build, and runs their nextest tests. Linux
runs selected doctests. macOS runs selected instrumented tests for LCOV,
explicitly excluding unselected workspace packages from the report. The
configured 85% floor and test identity isolation remain. Cheap workspace audits
and formatting still run. The unchanged policy escalates script, completion,
shared helper, manifest, and sensitive-source changes to the complete stack.

FULL uses the existing full entrypoint. Empty selection also runs full checks
rather than claiming an empty test PASS. Valid planner HOLD evidence remains
HOLD while actual full validation runs as fallback; it never becomes a scoped
plan PASS. A base without the launcher uses full checks without inventing READY.
Invalid protocol or bindings fail before selected commands.

## Evidence and provenance

Platform artifacts retain `target/ci-merge-plan/plan.json` when available and
`validation.json`. The latter binds planning B/H/M/T, actual checkout/tree,
captured event base, plan digest, effective scope,
packages, fallback, runner, doctest choice, coverage floor, command, exit code,
and PASS/FAIL. PASS and job output `validation_scope` are written only after the
actual executor succeeds. Planner HOLD and passing full validation are separate
facts. Executors do not receive `GITHUB_OUTPUT`; the consumer replaces the
step's output with its authoritative scalar scope after success, or empties it
after failure. Executor-written multiline framing cannot change that scope.
Full-validation markers are emitted only for actual FULL validation or
nightly/on-demand full routes, so scoped checks cannot prove full base CI to
the release gate. Coverage still requires macOS LCOV and containment success.

## Validation and measurement

`python3 scripts/ci/tests/merge-plan-ci.test.py` executes real workflow scripts,
the accepted planner on synthetic Git candidates, and the canonical entrypoint
with fake Cargo/admission. It covers event binding, selected command arguments,
coverage exclusions/floor, identity and launcher isolation, squash queue
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
