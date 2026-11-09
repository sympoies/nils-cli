# Delivery and releases

Pull requests and the merge queue protect development on `main`. Every merge
group also compiles and lints affected packages on macOS, including test targets.
Every published release must separately pass full Linux and macOS validation:
a fresh run on its exact tagged commit, or verified reuse of a complete
successful release-candidate merge-group full validation at the same commit or
an identical tree. Ordinary PR or scoped queue checks do not authorize a release.

## What runs

| Lane | Work | Failure blocks |
| --- | --- | --- |
| Pull request | Linux affected-package Clippy, binary builds, tests and doctests; workspace audits; dependency policy and containment checks | Merge |
| Merge queue | The same Linux scope planner on the actual combined merge-group tree, including reverse dependencies; affected-package macOS compile/Clippy with all targets and features; workspace fallback when planning cannot safely select scope; release-candidate groups also run full validation on the combined candidate | Merge |
| Nightly / manual full CI | Full Linux tests and doctests, Linux containment, dependency policy, full instrumented macOS tests and workspace line coverage of at least 85% | Full CI; opens or updates the nightly tracker |
| Release | Authorize the invocation, then the same full Linux/macOS/coverage workflow on the exact tag SHA (skipped only when complete merge-group evidence is verified for reuse), then build and package four platform targets, publish assets, and dispatch the tap update | Release builds, assets and tap update |
| Push to `main` | Lightweight status only | No additional merge or release guarantee |

[`ci.yml`](../../.github/workflows/ci.yml) runs on PRs, merge groups and main pushes. Its full
validation caller runs daily at **05:23 UTC**, or through `workflow_dispatch`
on `main`, using [`full-validation.yml`](../../.github/workflows/full-validation.yml).
Scheduled and manual full runs share one concurrency group; newer PR pushes
cancel their superseded run, while merge-group runs remain independent.

The daily full run is the automatic post-merge macOS regression check: both
test partitions run on the scheduled `main` commit, and a failed partition
fails full validation and opens or reopens the nightly tracker. The target is
the next daily run after a merge, subject to GitHub schedule and runner delays;
the schedule is not a hard deadline. Check the tracker and scheduled run when
the expected run is missing or delayed.

Budget 120 hosted macOS runner minutes per day for the scheduled partitions
(two jobs, each limited to 60 minutes), excluding optional manual and release
runs. The successful scheduled run 41835167531 used about 55 runner minutes
across its two partitions. Running full partitions on every affected merge
group would multiply that cost with queue traffic; manual PR dispatch alone
would not provide automatic detection. The existing nightly path adds no
runner jobs and covers every workspace crate.

The planner loads accepted policy from the immutable base, binds the source
head, combined candidate and tested tree, and records selected packages,
fallback reasons and results in `merge-validation-linux`. Unavailable planning
uses full Linux; invalid checkout/event bindings fail before validation.
Workspace manifests, CI, shared infrastructure and sensitive paths can
intentionally select FULL. Crates with no workspace dependents, generated
completion assets of a known binary, and the three audit baseline files stay
scoped; each run's job summary lists the scope, packages and reasons.

A canonical release version bump takes the reduced release-only lane (version
lockstep, third-party artifacts, publish order and a locked `cargo check`, no
test suites) without waiting for a base-commit receipt; any other dependency
change stays on the planner. The release workflow still requires full validation
of the tag, fresh or by verified reuse. Docs-only PRs run substantive
documentation audits.

Every merge group runs `macos_clippy` on `macos-latest`, using the same
trusted-base planner and exact candidate binding. It runs
`cargo clippy --locked --all-targets --all-features -p <package> -- -D warnings`
for affected packages and reverse dependencies, including integration test
crates. FULL, empty, or unavailable scope uses `--workspace`. It executes no
tests or coverage.

Scope and results are retained in
`merge-validation-macos-clippy`. PRs skip this job successfully; ordinary and
release queue groups both require its success through the required `test`
check. Full macOS tests and coverage remain nightly/release gates.
The merge queue still invokes the planner for docs and release changes.
See the [scope consumption contract](../specs/affected-scope-ci-consumption-v1.md).

## Required checks and migration

The maintainer manages the ruleset; workflow changes never edit it automatically.
After migration, both PRs and merge groups require these exact check names:

- `test`
- `test_containment`
- `cargo-deny (advisories + bans)`

The previous required names are `test`, `test_macos`, and `coverage`.
`test_macos` and `coverage` now exist only inside full nightly/release validation;
there is no deferred-full labeler. `macos_clippy` is a separate compile/lint
job: ordinary PRs are exempt, and every merge group executes it. The required
`test` job fails closed unless its macOS queue result is `success`; failed,
cancelled, skipped, or missing results block merge.

Retain the three Linux required contexts above. Per the
[maintainer decision in #2809](https://github.com/sympoies/nils-cli/issues/2809#issuecomment-6356110500),
no separate `macos_clippy` ruleset context is needed while required `test`
enforces that dependency. Add it only if that wiring is loosened. Verify an
ordinary queue entry and a release entry both report macOS Clippy success.
Full macOS tests and coverage remain nightly/release gates. The following
order documents the earlier full-validation migration:

Apply this order at the migration merge:

1. Review the workflow change and its hosted Linux results. Pause new queue
   admissions and drain any old groups before changing required contexts.
2. Add `test_containment` and `cargo-deny (advisories + bans)` to the ruleset
   while retaining `test`, `test_macos`, and `coverage`. Both added jobs already
   exist in the old workflow. Confirm their current PR results are successful.
3. Remove `test_macos` and `coverage` from required contexts immediately before
   admitting the migration PR. Keep all three Linux contexts required.
4. Enqueue the reviewed migration PR; require all three Linux checks on its
   actual group commit. Inspect the retained selected scope or full fallback.
5. Verify the merged workflow and start a full manual CI run on `main`, then
   resume queue admissions. Retain its result and the first nightly result.

This avoids a missing required macOS/coverage context blocking the migration
and preserves Linux containment and dependency failures as merge blockers.
For rollback, restore the prior workflow under the Linux ruleset, prove its
macOS/coverage jobs report again, then restore the previous required contexts.
Do not require removed checks before their producers are restored.

## Release flow

Before opening a version-bump PR, the release owner must:

1. Record the current `main` commit as the intended release base.
2. Confirm a successful scheduled full CI run on that exact commit, or dispatch
   the `CI` workflow on `main` and wait for full validation to pass. Check that
   both macOS partitions and the full-validation gate succeeded for that commit.
3. Recheck `main` before opening the PR. If it advanced, repeat the validation
   for the new base. Investigate a failed run before preparing the release.

The additional full run costs about 55 hosted macOS runner minutes per release
based on scheduled run 41835167531, with a 120-minute maximum from the two job
timeouts. A matching green nightly avoids that additional run. This is an owner
checklist: the release broker opens the PR outside this repository, so the
preparation workflow cannot enforce the check at PR creation. The release queue
still validates its combined candidate independently.

Version preparation makes a reviewed version-bump PR. The authorized release
broker owns stable `vMAJOR.MINOR.PATCH` tags and release publication; maintainers
must not tag or publish from an ordinary delivery session.
[`release.yml`](../../.github/workflows/release.yml) also supports authorized recovery dispatch
on an existing stable tag. Invocation and rerun identity checks still apply.

The release runs full validation itself, unless it verifies complete reusable
evidence from a release-candidate merge group (the `verify_ci` job in
[workspace-ci-entrypoint-inventory-v1](../specs/workspace-ci-entrypoint-inventory-v1.md)).

Every validation and build checkout
pins the caller's immutable `github.sha`; the expected commit input is checked
at the final gate and cannot select a different checkout. Its final gate rejects failed, cancelled,
skipped or missing Linux, macOS, coverage, containment or dependency jobs.
Only then can release builds start. Coverage uses the full-workspace 85% floor,
with an LCOV report required; source-PR checks and ordinary merge-group checks
cannot substitute for this gate. Releases build native Linux x86_64/aarch64 and macOS aarch64
archives, audit third-party notices, upload assets, publish the GitHub Release,
and confirm that a tap update workflow actually started. Crates.io publication
is a separate authorized [`publish-crates.yml`](../../.github/workflows/publish-crates.yml) flow.

## Claude Code release check

[`claude-release-check.yml`](../../.github/workflows/claude-release-check.yml)
runs hourly on `main`. A `plan` job with read-only repository, issue and
pull-request permissions runs first. When the newest stable version is above
the reviewed range and a candidate PR (any state, head `ci/claude-range-<version>`)
or a failure issue (any state) already exists for it, the scheduled run skips the
`check` and `publish` jobs: no Rust setup, cargo build, isolated install or
provider write, and the run stays green. A new version, an in-range version, a
failed lookup and every manual dispatch run the full check.

Cost while a candidate waits for native acceptance, per day: before, up to 24
full runs (Rust toolchain, `cargo test --no-run -p nils-agent-session` and the
isolated install; a run that reached those steps took about 4 minutes); after,
up to 24 plan-only runs of roughly half a minute each, plus one full run per new
version.

A candidate failure is reported once: the first failing run fails red and opens
the version-keyed issue, which keeps its first evidence. Later scheduled runs
skip, so they no longer fail every hour. To retry after a fix, dispatch the
workflow manually; it does not skip.

### Orphan candidate branch

If PR creation fails after `ci/claude-range-<version>`
was pushed, a rerun on the unchanged `main` commit recovers it. After `main`
moves, the run stops with `candidate branch occupied; maintainer reconciliation
required` and does not recover automatically. Reconcile as a maintainer:

1. Confirm there is no PR for the branch, open or closed. Inspect its commits
   and changes against its base: every commit must be workflow-generated, and
   only the generated range files may change. If either check fails, stop and
   investigate; do not delete the branch.
2. Record the exact branch head on the tracking issue.
3. The maintainer, or a session authorized by the maintainer for this branch,
   deletes exactly that branch using its recorded head as the lease. Never use
   a wildcard; run one command per branch:

   ```bash
   git push --force-with-lease=refs/heads/ci/claude-range-<version>:<recorded-head-sha> origin --delete ci/claude-range-<version>
   ```

4. Dispatch the workflow manually on `main`; it opens the draft candidate from
   the current `main`.

## Failure recovery

For a PR or queue failure, inspect the earliest failing step and its plan,
validation receipt and JUnit artifacts. Repair through the PR, validate the
new head, and requeue after review. A full fallback is valid evidence of full
Linux execution, not evidence that scope planning succeeded.

A failed or cancelled full nightly run opens or updates one bot-owned tracking
issue. It includes the failed commit, run URL, and comparison from the last
earlier successful scheduled nightly. With no green baseline, it says so.
Updates preserve maintainer notes outside the generated section. Repeated
failures reuse the issue; a later successful scheduled or manual full run
closes it, and a new failure reopens it. Duplicate markers, API failures or
invalid ownership fail the reporting job visibly rather than creating another
issue. Reporting runs with issue-write permission only on trusted `main`.

Inspect the first failure and bisect the recorded range. Repair or revert the
culprit through a reviewed PR and run full CI on `main` again. A missed scope
also needs a regression in the selector. Nightly failure does not automatically
change rulesets, revert code, or waive the release gate.

If a release gate fails, no release assets or tap dispatch can occur. Retain
its logs and artifacts, fix the defect through a PR, and let the broker prepare
a new release commit/tag. For infrastructure-only failures, the authorized
broker may rerun the existing tag; full validation must succeed before builds.
If publication or tap dispatch fails after successful builds, inspect the
existing release/run first and use the broker's recovery flow to avoid
publishing a conflicting release. Never bypass a failed validation gate.

## Time limits

| Job | Limit |
| --- | --- |
| Change detection | 60 minutes |
| Linux validation | 50 minutes |
| Linux containment | 25 minutes |
| Dependency policy | 15 minutes |
| Full macOS test partition (two run in parallel) | 60 minutes |
| Full macOS coverage merge and floor | 15 minutes |
| Coverage report / nightly issue update | 10 minutes each |
| Final full-validation gate | 5 minutes |
| Release invocation authorization | 35 minutes |

A release queue entry runs the Linux and the two macOS partition jobs in
parallel before the required `test` job. In the [v1.40.8 version PR merge-group
run](https://github.com/sympoies/nils-cli/actions/runs/41797597406), the entry
took 31.4 minutes end to end. The Linux full-validation `test` job was the
critical path at 28.5 minutes; the macOS partitions took 26.1 and 17.7 minutes,
and the required `test` job added about 2 minutes. The Linux job's 50-minute
timeout left about 21.5 minutes of margin; the release PR's 60-minute check
wait left about 28.6 minutes. Before the split, the single macOS job took
about 60 minutes and set the entry's length.

The nextest CI profile terminates an individual test after its configured
10-minute slow-test bound; job limits also catch hung builds or tool installs.
Build, packaging and publication jobs retain GitHub Actions' default job limit.
Timeouts and cancellations cannot satisfy the full-validation gate.

For local commands and prerequisites, see [DEVELOPMENT.md](../../DEVELOPMENT.md)
and the [workspace maintenance reference](workspace-maintenance-reference.md).
