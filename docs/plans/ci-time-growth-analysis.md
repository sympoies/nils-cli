# CI Time Growth Analysis

Refs #3278. Related: #3277 (release queue time), #2896 (test-thread
limits), #2693 (nightly full validation), #2717 (scoped dependency cache reuse).

Lifecycle: `transient-dev-record`. Remove this plan once the follow-up issues it
lists are closed or rejected.

## Summary

- Test count is not the main driver. Between 2026-10-14 and 2026-11-03 the
  full Linux suite grew from 9,237 to 10,362 tests (+12%), while its nextest
  wall time grew from 10.4 to 27.6 minutes (+165%). macOS grew from 20.2 to 41.6
  minutes (+106%) for +12% tests.
- Slow tests drive the growth. Tests taking at least 1 s doubled (309 to 615 on
  Linux) and now hold 86% of summed Linux test time. The 8,586 tests under 0.1 s
  take 2.6 minutes in total.
- One crate dominates. `nils-agent-session` added 940 of the 1,125 new tests
  and 75% (Linux) to 78% (macOS) of the added test time. Almost every scoped
  queue run validates `nils-agent-session` and `nils-agent-hook`, about 24 minutes.
- Both CI lanes run tests on 2 threads, and summed test time is 2.0 times the
  wall time on both platforms, so every added second of test time is about half
  a second of wall time.
- `full_validation / test_macos` is the release critical path: median 49.3
  minutes (week of 10-20) and 52.9 minutes (from 10-27), with a high of 66.3
  minutes. About 8 minutes of it repeats checks that other jobs already run on
  the same commit.
- The nightly Linux full validation has exceeded its 50-minute limit on 5 of the
  6 nights from 10-28 to 11-02. The isolation guard (10 to 12 minutes) runs
  after a checks step that now takes 34 to 37 minutes.

## Method And Sample

- Window: CI workflow (`ci.yml`) runs created 2026-10-13 to 2026-11-03.
  1,867 runs listed; job and step timings collected for the 1,439 runs that are
  not `push` events. Read-only `gh api` GET calls only.
- Durations below are for successful jobs (start to completion) unless marked.
  Weeks: W1 = 10-13..10-19, W2 = 10-20..10-26, W3 = 10-27..11-03.
- Workflow shape changed in W1 (#2685): before about 10-20, every pull request
  and queue entry ran a full macOS job. From W2 on, macOS full validation runs
  only for release queue entries and nightly. Trend comparisons use like jobs.
- JUnit: the `junit-nextest` and `junit-nextest-macos` artifacts of 12 full runs
  (24 reports) listed in the full-lane table.
- Validation scope: `merge-validation-linux` receipts (`validation.json`) of a
  random sample of 20 successful `test` jobs per week for queue and pull
  request events (120 sampled, 100 had a receipt).
- Phase split: job logs of release run 41605970286 (Linux job 125815965429,
  macOS job 125815965517).

## Measured Trend

### Job durations by event (successful jobs, minutes)

| Event and job | W1 median / p90 | W2 median / p90 | W3 median / p90 |
| --- | --- | --- | --- |
| Pull request `test` (all scopes) | 28.0 / 32.2 (n=279) | 21.8 / 31.8 (n=250) | 24.9 / 38.2 (n=243) |
| Queue, ordinary, `test` (all scopes) | 28.1 / 31.4 (n=153) | 22.0 / 32.8 (n=163) | 25.1 / 39.0 (n=122) |
| Queue, ordinary, `macos_clippy` | n/a | 3.9 / 5.3 (n=77) | 4.0 / 5.2 (n=122) |
| Queue, release, `full_validation / test` | n/a | 31.0 / 35.0 (n=20) | 37.9 / 41.2 (n=28) |
| Queue, release, `full_validation / test_macos` | n/a | 49.3 / 52.3 (n=18) | 52.9 / 60.0 (n=28) |
| Queue, release, required `test` (after `full_validation`) | n/a | 2.2 / 2.5 (n=18) | 3.1 / 3.5 (n=26) |
| Nightly, `full_validation / test_macos` | n/a | 49.2 / 57.4 (n=6) | 56.5 / 63.9 (n=7) |

Run wall time (start to last update, successful runs): ordinary queue entries
median 25.6 minutes in W2 and 29.9 in W3 (p90 36.7 and 44.4). Release queue
entries median 53.1 in W2 (n=18) and 58.1 in W3 (n=26), p90 56.4 and 62.9.

### Linux `test` job by validation scope (sampled receipts, minutes)

| Event | Scope | W1 | W2 | W3 |
| --- | --- | --- | --- | --- |
| Queue, ordinary | FULL | 29.9 (n=9) | 31.2 (n=7) | 37.8 (n=8) |
| Queue, ordinary | SCOPED | n/a | 21.4 (n=13) | 24.6 (n=12) |
| Pull request | FULL | 29.5 (n=15) | 30.3 (n=12) | 38.2 (n=9) |
| Pull request | SCOPED | n/a | 21.0 (n=7) | 24.1 (n=7) |

In W3, 18 of the 19 sampled SCOPED receipts list exactly `nils-agent-hook` and
`nils-agent-session` (for example 41597981661, 41395472365, 40852263935).
In W2 and W3, 15 of the 40 sampled queue entries ran the FULL scope.

### Full-lane test suite (JUnit)

| Date | Run | Linux tests | Linux nextest wall | Linux tests >= 1 s | macOS tests | macOS nextest wall | macOS tests >= 1 s |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 10-14 | 38548994946 | 9,237 | 10.4 | 309 | 8,977 | 20.2 | 398 |
| 10-16 | 38884018269 | 9,405 | 14.4 | 381 | 9,124 | 22.3 | 439 |
| 10-19 | 39241283283 | 9,395 | 15.9 | 440 | 9,119 | 25.0 | 511 |
| 10-20 | 39406256813 | 9,494 | 16.3 | 449 | 9,214 | 26.1 | 556 |
| 10-22 | 39766583894 | 9,678 | 22.0 | 499 | 9,397 | 32.9 | 620 |
| 10-24 | 40055094406 | 9,751 | 22.1 | 517 | 9,446 | 32.9 | 632 |
| 10-26 | 40310094879 | 9,851 | 22.9 | 534 | 9,544 | 34.8 | 666 |
| 10-28 | 40652537775 | 9,972 | 21.1 | 513 | 9,651 | 35.1 | 678 |
| 10-30 | 41002091637 | 10,085 | 21.7 | 530 | 9,764 | 36.2 | 695 |
| 10-31 | 41194028047 | 10,138 | 25.3 | 588 | 9,812 | 39.9 | 760 |
| 11-01 | 41303990468 | 10,205 | 25.7 | 584 | 9,879 | 40.5 | 766 |
| 11-03 | 41605970286 | 10,362 | 27.6 | 615 | 10,019 | 41.6 | 771 |

Wall times are minutes from the JUnit `testsuites` time. The first four rows
are nightly runs and the rest are release queue entries; all ran the whole
workspace on the same platforms.

Run 41605970286 by duration bucket:

| Bucket | Linux tests | Linux summed minutes | macOS tests | macOS summed minutes |
| --- | --- | --- | --- | --- |
| < 0.1 s | 8,586 | 2.6 | 7,036 | 3.1 |
| 0.1 to 1 s | 1,161 | 5.2 | 2,212 | 11.2 |
| 1 to 5 s | 429 | 15.1 | 545 | 19.6 |
| 5 to 10 s | 129 | 13.6 | 130 | 14.9 |
| 10 to 30 s | 48 | 12.4 | 78 | 22.6 |
| >= 30 s | 9 | 6.3 | 18 | 11.7 |

### Release lane phases (run 41605970286, minutes)

| Phase | Linux `full_validation / test` | macOS `full_validation / test_macos` |
| --- | --- | --- |
| Runner setup, toolchain, cache restore, installs | 0.8 | 1.7 |
| Repository audits and script tests | 4.5 | 5.2 |
| `cargo fmt` and `cargo clippy` | 1.7 | 2.8 |
| Test build (`nextest` / instrumented `llvm-cov`) | 3.9 | 5.4 |
| Test run | 27.6 | 41.6 |
| Doc tests (Linux) / LCOV export and coverage floor (macOS) | 0.2 | 1.8 |
| HTML coverage report | n/a | 1.6 |
| Total job | 38.9 | 60.1 |

### Nightly

The nightly Linux job runs the checks step, then a latency benchmark and the
isolation guard. Checks step: 26.7 minutes (40490609705), 33.9 (41303990468),
36.8 (41455015894). Isolation guard: 10.1, 12.3, and cancelled after 12.7
minutes. Runs 40680056142, 40866799853, 41048349156, 41194967805 and
41455015894 were cancelled at the 50-minute job limit (about 50.3 minutes), so
the gate failed. #2693 already records the first cause; this report adds the
phase trend.

## Drivers

### Slow tests, mostly in `nils-agent-session`

Change between run 38548994946 (10-14) and run 41605970286 (11-03), summed test
minutes per crate:

| Crate | Tests | Linux summed minutes | macOS summed minutes |
| --- | --- | --- | --- |
| `nils-agent-session` | 1,896 to 2,836 | 9.3 to 35.4 (+26.1) | 13.5 to 47.0 (+33.4) |
| `nils-forge-cli` | 1,948 to 2,239 | 1.7 to 5.8 (+4.1) | 3.3 to 8.8 (+5.4) |
| `nils-git-cli` | 474 to 503 | 2.0 to 4.2 (+2.2) | 3.4 to 6.2 (+2.7) |
| `nils-codex-cli` | 518 to 529 | 1.8 to 3.2 (+1.4) | 0.9 to 1.2 (+0.3) |
| All crates | 9,237 to 10,362 | 20.7 to 55.2 (+34.6) | 40.3 to 83.1 (+42.8) |

Inside `nils-agent-session`, `serve::tests` grew from 426 to 556 tests and from
1.4 to 8.7 summed Linux minutes; its `prompt_receipts_tests` (62 tests, 3.3
minutes) and `claude_mailbox` (45 tests, 2.1 minutes) are the largest groups.
New `guarded_send` and `codex_app_server` tests add several minutes more.

### macOS cost per test

The same suite takes 1.5 times as long on macOS (83.1 versus 55.2 summed
minutes in 41605970286). The macOS lane runs instrumented (`llvm-cov`) on a
smaller runner. Two groups are far slower on macOS than on Linux:

- `nils-macos-agent::integration` `backend::*`: 51 tests, 13.1 summed minutes on
  macOS versus 2.4 on Linux (about 5.5 times per test).
- `codex_app_server::proxy_transport` oversized-message tests: 41.4 and 25.7 s
  on macOS versus 2.1 and 1.2 s on Linux.

### Parallelism

`NILS_CLI_RUNNER_MAX=2` sets `NEXTEST_TEST_THREADS=2` on both lanes. Summed
test time divided by wall time is 2.0 on both platforms in every sampled run, so
the pool is saturated. #2896 owns raising this limit.

### Repeated work in the release queue

In a release queue entry, these run on the same commit:

- Repository audits and script tests run in both full-validation test jobs
  (4.5 and 5.2 minutes). The macOS lane already skips only the stale-test and
  completion audits.
- `cargo clippy` runs in `macos_clippy` (median 4.5 minutes in W3) and again in
  `test_macos` (2.8 minutes with fmt).
- `test_containment` and `cargo-deny` run at top level and inside
  `full_validation` (0.6 plus 2.8 minutes, and 0.2 twice). These cost runner
  time but are not on the critical path.
- The required `test` job runs only release checks, 2.2 to 3.6 minutes, after
  `full_validation` finishes (#3277 option 3).

### Release volume and late macOS failures

W2 and W3 had 49 release queue entries, 44 of them successful (about 3.4 a
day). Each uses about 39 Linux and 60 macOS runner minutes in the two
full-validation test jobs. Release entries used 1,710 of the 2,744 macOS runner
minutes in W3 (62%). Five release entries failed, and each failure cost a new
queue entry of about one hour:

- Real macOS-only defects that no earlier required check runs on macOS:
  39842789949 (dead code under macOS, compile error), 39859892752 (1 failed
  test) and 40828447533 (6 failed tests).
- Linker crash: 41590477216 (`ld` bus error, see #3277).
- `macos_clippy` cancelled after 15 minutes: 41087103953 (cause not inspected).

### Build and cache

- The release lane test builds take 3.9 (Linux) and 5.4 (macOS) minutes with a
  warm cache, about 10% of each job.
- Only `main` saves the `linux-test` and `macos-test` caches, which in practice
  means nightly and manual runs. A cancelled nightly Linux job skips its cache
  save (post step skipped in 41455015894), so no nightly Linux job could save
  the cache between 10-27 and 11-01. The effect on build time was not measured.
- The cache store held about 9.6 GB against the 10 GB limit when measured.
  104 CodeQL overlay entries took 1.4 GB; each Rust test cache is 0.6 to 1.0 GB.
- Checked and rejected: running the nightly only on unvalidated trees. Only 1 of
  12 nightlies from 10-22 had the same tree as a successful release entry
  (40182712577 and 40170806931).

## Options

The minutes are estimates derived from the measurements above. Savings on the
release path assume the other lane is not the longer one.

| # | Option | Estimated saving | Risk | Owner |
| --- | --- | --- | --- | --- |
| A | Validate the release on the PR head and accept the queue entry when its tree matches | Release queue about 58 to a few minutes when the base is unchanged | Medium: tree identity proof | #3277 option 1 |
| B | Run the required release-only `test` work beside `full_validation` | 2 to 3.6 minutes per release | Low | #3277 option 3 |
| C | Remove repeated work from `test_macos`: OS-independent audits and script tests, clippy already done by `macos_clippy`, HTML report outside release | About 8 minutes off the macOS critical path (60 to about 52) | Low: the same checks still run on the same commit; nightly keeps the HTML report | Input to #3277 option 2 |
| D | Split macOS nextest into 2 partitions and merge coverage before the floor check | Test phase 41.6 to about 21; macOS job about 60 to about 39 (about 31 with C). Linux, at 39 minutes, then becomes the critical path | Medium: coverage merge must give the same floor result; about 9 more macOS runner minutes per release | Input to #3277 option 2 |
| E | Raise test threads above 2 | Linux nextest 27.6 to about 18 to 21 minutes at 3 threads; similar share on macOS. Applies to every lane | Medium: flakes and memory, measured under #2896 | #2896 |
| F | Fix the slowest test clusters (candidate list below) | About 5 to 8 minutes Linux and 8 to 12 minutes macOS wall at 2 threads | Low to medium: assertions must stay | New issues 3 to 6 |
| G | Run the nightly isolation guard in its own job | Nightly Linux job about 50+ to about 38 minutes; nightly green again and cache saves resume | Low | New issue 1 |
| H | Fewer release queue entries (batch version bumps) | About 100 runner minutes and one hour of queue time per release avoided | Low technical risk; a release policy choice | Lead decision |
| I | Run scoped macOS tests for affected packages in the ordinary queue | Catches macOS-only defects before a release (3 of 49 release entries) | Adds macOS runner minutes to every code entry; reverses part of #2685 | Lead decision |

For the release queue, A removes the wait in the common case. C and D bound the
fallback to roughly the Linux lane (about 39 minutes), and E and F then lower
both lanes.

## Growth Control

### Budgets

- Per test: unit tests under 1 s and integration tests under 10 s on the Linux
  lane. A slower test must be listed with a reason and an owner.
- Per lane, at the current 2 threads: FULL Linux nextest wall at most 25
  minutes and macOS at most 40 minutes, measured from nightly JUnit. Both are
  already exceeded (27.6 and 41.6).
- Per pull request: added summed test time at most 30 s on Linux without a
  listed reason, measured against the base run's JUnit for the same packages.

### Checks

1. Slow-test ratchet (required after an advisory period): read the lane's JUnit
   and fail when a test not in a baseline file takes 10 s or more on Linux.
   The baseline starts from the current list and may only shrink, like
   `scripts/ci/file-size-baseline.tsv`. Gate only on new names to avoid noise
   from timing variance.
2. Timing report (advisory): the nightly publishes test count, nextest wall,
   build time and the count of tests of 1 s or more per platform, and flags a
   lane budget breach in the nightly tracking issue.
3. Consolidation review: a pull request that adds more than 25 tests to one
   module, or any test over 5 s, states in its body why existing tests do not
   cover the case.

### Criteria for merging or deleting tests

Delete or merge a test only when one of these holds and the remaining tests
still assert every fact it asserted:

1. Same fixture, same code path, and its assertions are a subset of another
   test: merge into one table-driven test with one fixture.
2. Its time is a fixed wait (sleep, deadline, staleness window): keep it and
   inject the clock or shorten the wait through configuration.
3. It covers a removed flag or contract: delete it with the removal.
4. The same fact is asserted at unit and CLI level: keep the unit test and one
   CLI smoke test per command.
5. Platform cost: a test of OS-independent logic that is over 5 times slower on
   macOS should use a smaller payload on macOS. Never drop a macOS-specific
   path from macOS.

### First candidate list (run 41605970286)

| Candidate | Linux s | macOS s | Criterion | Reason |
| --- | --- | --- | --- | --- |
| `nils-agent-session::integration` `lineage::*` (5), `origin::*` (2), `admission::*` (2) | 20.0 to 26.7 each, 3.2 min total | 21.2 to 28.2 each, 3.4 min total | 2, then 1 | Nine tests with nearly the same duration point to a shared fixed wait in the fake start path; several `lineage` tests start sessions to check separate facets of one start |
| `nils-forge-cli` `pr_review_handoff::retired_coordinator::deleted_producer::current_deleted_producer_recovers_after_staleness` | 61.8 | 62.8 | 2 | Slowest test; the name points to a staleness window |
| `codex_app_server::proxy_transport` oversized request and response tests | 2.1 and 1.2 | 41.4 and 25.7 | 5 | 20 times slower on macOS |
| `nils-macos-agent::integration` `backend::*` (51 tests) | 2.4 min total | 13.1 min total | 1 | Install, upgrade and rollback scenarios each stage releases; check for shared staged fixtures |
| `serve::tests::prompt_receipts_tests` and `serve::tests::claude_mailbox` | 3.3 and 2.1 min total | 4.3 and 2.8 min total | 1, 2 | Largest growth inside `serve::tests` |
| `remote_mailbox::two_root_http_federation_acceptance`, `activity_hook_ingress::auth_loss_real_hook_notifies_owner_once_and_reports_recovery`, `guarded_send` `history_command_bytes::runtime_command_input_bytes_are_unchanged` | 52.0, 60.3, 36.8 | 51.8, 30.8, 52.9 | 2 | Acceptance tests over 30 s; keep them and look for fixed waits |

## Recommendation

1. Let #3277 take A and B, and give it C and D as measured inputs for its
   option 2. This is the only route to a release queue well under an hour.
2. Open new issue 1 now: the nightly has been red from a time limit for a week,
   and its cache saves stop with it.
3. Fix the 20-second cluster (new issue 3) first among the slow tests: it is
   cheap to check and covers nine tests.
4. Add the slow-test ratchet (new issue 2) so the budget holds once the
   clusters are fixed.
5. Keep #2896 as the owner of thread limits; this report adds that both pools
   are saturated (sum/wall 2.0).
6. Lead decisions: release batching (H) and macOS tests in the ordinary queue (I).

## Proposed Issues

Not opened yet; the lead decides which to open. None of them duplicates
issues #3277, #2896, #2693 or #2717.

| # | Title | Priority | Size | Files | Acceptance |
| --- | --- | --- | --- | --- | --- |
| 1 | ci: run the nightly isolation guard in its own job | P1 | S | `.github/workflows/full-validation.yml`, `.github/scripts/full-validation-gate.cjs`, `scripts/ci/tests/full-validation-gate.test.cjs` | Nightly Linux test job under 45 minutes; the guard is a separate job that the gate requires when `isolation_guard` is true; the Linux cache saves on a green nightly; three green nightlies in a row. Refs #2693 |
| 2 | ci: slow-test ratchet and lane timing report | P2 | M | new `scripts/ci/slow-test-ratchet.py` with a baseline TSV and tests, `.github/workflows/ci.yml`, `.github/scripts/nightly-tracker.cjs` | Advisory first: a new test of 10 s or more on Linux is named in the job summary; the baseline only shrinks; nightly reports test count, nextest wall and tests of 1 s or more per platform and flags a lane budget breach |
| 3 | agent-session: remove the fixed wait from the fake-start integration fixtures | P1 | M | `crates/agent-session/tests/integration/lineage.rs`, `origin.rs`, `admission.rs` and their shared helpers | Each of the nine tests under 5 s on Linux in JUnit; their summed time under 0.5 minutes; every current assertion kept |
| 4 | agent-session: reduce `serve::tests` and `guarded_send` test time | P2 | M | `crates/agent-session/src/serve/` tests, `crates/agent-session/tests/guarded_send/` | `nils-agent-session` summed Linux test time from 35.4 to 25 minutes or less; scoped queue `test` job under 20 minutes |
| 5 | agent-session: make the proxy transport oversized-message tests fast on macOS | P2 | S | `crates/agent-session/src/codex_app_server/proxy_transport/tests.rs` | Both tests under 5 s on macOS with the same rejection and drop assertions |
| 6 | macos-agent: share staged fixtures in backend integration tests | P2 | M | `crates/macos-agent/tests/` backend integration tests | `backend::*` summed macOS time from 13.1 to 6 minutes or less; same scenarios covered |
| 7 | forge-cli: inject the clock in the deleted-producer staleness test | P3 | S | `crates/forge-cli/tests/` `pr_review_handoff` tests | The test runs under 5 s on both platforms and still proves recovery after staleness |
