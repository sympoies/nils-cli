# Delivery and releases

Pull requests and the merge queue protect Linux development on `main`.
Every published release must separately pass full Linux and macOS validation
on its exact tagged commit. Green PR or queue checks do not authorize a release.

## What runs

| Lane | Work | Failure blocks |
| --- | --- | --- |
| Pull request | Linux affected-package Clippy, binary builds, tests and doctests; workspace audits; dependency policy and containment checks | Merge |
| Merge queue | The same Linux scope planner on the actual combined merge-group tree, including reverse dependencies; full Linux fallback when planning cannot safely select scope | Merge |
| Nightly / manual full CI | Full Linux tests and doctests, Linux containment, dependency policy, full instrumented macOS tests and workspace line coverage of at least 85% | Full CI; opens or updates the nightly tracker |
| Release | Authorize the invocation, then the same full Linux/macOS/coverage workflow on the exact tag SHA, then build and package four platform targets, publish assets, and dispatch the tap update | Release builds, assets and tap update |
| Push to `main` | Lightweight status only | No additional merge or release guarantee |

[`ci.yml`](../../.github/workflows/ci.yml) runs on PRs, merge groups and main pushes. Its full
validation caller runs daily at **05:23 UTC**, or through `workflow_dispatch`
on `main`, using [`full-validation.yml`](../../.github/workflows/full-validation.yml).
Scheduled and manual full runs share one concurrency group; newer PR pushes
cancel their superseded run, while merge-group runs remain independent.

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
change stays on the planner. The release workflow still validates the tag in full. Docs-only PRs run substantive documentation audits.
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
there is no deferred-full labeler or skipped macOS check in the PR/queue lane.

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

Version preparation makes a reviewed version-bump PR. The authorized release
broker owns stable `vMAJOR.MINOR.PATCH` tags and release publication; maintainers
must not tag or publish from an ordinary delivery session.
[`release.yml`](../../.github/workflows/release.yml) also supports authorized recovery dispatch
on an existing stable tag. Invocation and rerun identity checks still apply.

The release runs full validation itself. Every validation and build checkout
pins the caller's immutable `github.sha`; the expected commit input is checked
at the final gate and cannot select a different checkout. Its final gate rejects failed, cancelled,
skipped or missing Linux, macOS, coverage, containment or dependency jobs.
Only then can release builds start. Coverage uses the full-workspace 85% floor,
with an LCOV report required; source-PR or merge-group green checks cannot
substitute for this gate. Releases build native Linux x86_64/aarch64 and macOS aarch64
archives, audit third-party notices, upload assets, publish the GitHub Release,
and confirm that a tap update workflow actually started. Crates.io publication
is a separate authorized [`publish-crates.yml`](../../.github/workflows/publish-crates.yml) flow.

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
| Full macOS tests and instrumented coverage | 75 minutes |
| Coverage report / nightly issue update | 10 minutes each |
| Final full-validation gate | 5 minutes |
| Release invocation authorization | 35 minutes |

The nextest CI profile terminates an individual test after its configured
10-minute slow-test bound; job limits also catch hung builds or tool installs.
Build, packaging and publication jobs retain GitHub Actions' default job limit.
Timeouts and cancellations cannot satisfy the full-validation gate.

For local commands and prerequisites, see [DEVELOPMENT.md](../../DEVELOPMENT.md)
and the [workspace maintenance reference](workspace-maintenance-reference.md).
