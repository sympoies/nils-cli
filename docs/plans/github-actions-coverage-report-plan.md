# Plan: GitHub Actions code coverage report

## Overview
This plan adds a code coverage report to CI for this Rust workspace. The baseline deliverable is an LCOV + HTML report generated via `cargo llvm-cov`, uploaded as GitHub Actions artifacts, with a human-readable coverage summary visible in the workflow UI. Existing CI checks (fmt/clippy/tests/zsh completion + JUnit report) remain unchanged.

## Scope
- In scope:
  - Generate coverage on CI (Linux) for the Rust workspace tests and publish it as artifacts.
  - Surface a coverage percentage summary in GitHub Actions (job summary and/or PR comment).
  - Document how to reproduce the coverage report locally.
- Out of scope:
  - Modifying existing tests to change coverage.
  - Enforcing a coverage threshold as a required gate (optional Sprint).
  - Doctest coverage (doctests still run for correctness, but are not included in coverage initially).

## Assumptions (if any)
1. Using `cargo llvm-cov` is acceptable (preferred over `cargo-tarpaulin` for accuracy and future macOS support).
2. Coverage is computed on `ubuntu-latest` only (single job) to control CI time and complexity.
3. “Report” means both (a) downloadable artifacts (LCOV/HTML) and (b) a visible summary in the GitHub Actions UI.
4. For PRs from forks, we avoid workflows that require elevated permissions (PR commenting is optional and should degrade safely).

## Sprint 1: Coverage artifacts in CI (LCOV + HTML)
**Goal**: CI produces a code coverage report for workspace tests and uploads it as artifacts.
**Demo/Validation**:
- Command(s):
  - `cargo llvm-cov --workspace --all-features --lcov --output-path target/coverage/lcov.info`
  - `cargo llvm-cov --workspace --all-features --html --output-dir target/coverage/html`
- Verify:
  - `target/coverage/lcov.info` exists
  - `target/coverage/html/index.html` exists and renders in a browser when downloaded from CI

### Task 1.1: Document local coverage generation
- **Location**:
  - `DEVELOPMENT.md`
- **Description**: Add a “Coverage (optional)” section documenting how to install `cargo-llvm-cov`, which command(s) to run, where outputs are written, and the known limitation that doctests aren’t included initially.
- **Dependencies**: none
- **Complexity**: 2
- **Acceptance criteria**:
  - `DEVELOPMENT.md` includes copy-pastable commands to generate LCOV and HTML locally.
  - The documented commands produce `target/coverage/lcov.info` and `target/coverage/html/index.html`.
- **Validation**:
  - `rg -n "Coverage \\(optional\\)|llvm-cov" DEVELOPMENT.md`
  - `cargo llvm-cov --workspace --all-features --lcov --output-path target/coverage/lcov.info`

### Task 1.2: Add a dedicated coverage job to CI
- **Location**:
  - `.github/workflows/ci.yml`
- **Description**: Add a `coverage` job (Linux only) that checks out the repo, installs the Rust toolchain with `llvm-tools-preview`, installs `cargo-llvm-cov`, and generates both LCOV and HTML outputs under `target/coverage/`. Keep it independent from the existing `test` job so coverage can be tuned (or made optional) without affecting the main required checks.
- **Dependencies**: none
- **Complexity**: 5
- **Acceptance criteria**:
  - The CI workflow has a `coverage` job that runs on `pull_request` and `push`.
  - The job generates `target/coverage/lcov.info` and `target/coverage/html/index.html`.
  - If the coverage command fails, the job fails with clear logs (no silent success).
- **Validation**:
  - `git diff -- .github/workflows/ci.yml`

### Task 1.3: Upload coverage artifacts
- **Location**:
  - `.github/workflows/ci.yml`
- **Description**: Upload coverage artifacts using `actions/upload-artifact`: the raw `lcov.info` and the HTML directory. Run uploads with `if: always()` but only when outputs exist to avoid noisy failures on early-step errors.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 3
- **Acceptance criteria**:
  - A CI run contains a downloadable artifact with `lcov.info`.
  - A CI run contains a downloadable artifact with an HTML report (including `index.html`).
  - Upload steps do not fail the workflow when coverage outputs were not produced.
- **Validation**:
  - `git diff -- .github/workflows/ci.yml`

## Sprint 2: Coverage summary visible in GitHub Actions UI
**Goal**: A CI run visibly shows the overall coverage % in the GitHub Actions UI without downloading artifacts.
**Demo/Validation**:
- Command(s): Run CI on a PR.
- Verify: The `coverage` job includes a step summary with at least total line coverage percentage.

### Task 2.1: Generate and publish a Markdown coverage summary
- **Location**:
  - `.github/workflows/ci.yml`
- **Description**: Convert `lcov.info` into a short Markdown summary and write it to `$GITHUB_STEP_SUMMARY`. Prefer an off-the-shelf action that can read LCOV and emit Markdown; alternatively, add a small repo-local script that parses totals from LCOV (keep it minimal and well-tested if you take this path).
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - The workflow UI shows a coverage percentage in the `coverage` job summary.
  - The summary step runs even when tests fail, as long as `lcov.info` exists.
- **Validation**:
  - `git diff -- .github/workflows/ci.yml`

### Task 2.2 (Optional): Post a sticky PR comment with coverage summary
- **Location**:
  - `.github/workflows/ci.yml`
- **Description**: If desired, post (or update) a single sticky PR comment containing the Markdown summary. Ensure it is skipped for forked PRs (or runs without needing elevated permissions) to avoid permission failures.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 5
- **Acceptance criteria**:
  - On same-repo PRs, a single comment is created/updated with the latest coverage.
  - On fork PRs, the workflow does not fail due to missing permissions.
- **Validation**:
  - Open a same-repo PR and confirm comment updates across pushes.

## Sprint 3 (Optional): Coverage gates and badges
**Goal**: Make coverage a policy signal (threshold and/or badge) without adding excessive maintenance burden.
**Demo/Validation**:
- Command(s): `cargo llvm-cov --workspace --all-features --fail-under-lines <N>`
- Verify: CI fails when coverage drops below threshold (when enabled).

### Task 3.1 (Optional): Enforce a minimum line coverage threshold
- **Location**:
  - `.github/workflows/ci.yml`
- **Description**: Decide an initial line coverage threshold and enable it via `cargo llvm-cov` (e.g. `--fail-under-lines <N>`). Start with a non-blocking threshold (job allowed to fail) or run it only on `main` until the baseline is stable.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - The threshold value is documented in the workflow (and optionally in `DEVELOPMENT.md`).
  - When enabled as blocking, the job fails if coverage is below the threshold.
- **Validation**:
  - `git diff -- .github/workflows/ci.yml`

### Task 3.2 (Optional): Add a coverage badge
- **Location**:
  - `README.md`
- **Description**: If the repo chooses an external coverage service (Codecov/Coveralls) or a GitHub-native badge approach, add a badge to `README.md`. Prefer solutions that don’t require secret tokens for PRs.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `README.md` displays a working badge that updates automatically.
- **Validation**:
  - `git diff -- README.md`

## Testing Strategy
- Unit: none required (CI configuration and documentation changes only).
- Integration: run `cargo llvm-cov ...` locally on at least one crate and confirm outputs are generated and non-empty.
- E2E/manual: open a PR and confirm artifacts upload and the workflow UI summary render correctly.

## Risks & gotchas
- Coverage tools add CI time; keep it as a separate job and consider limiting to `push` on `main` if PR latency becomes an issue.
- Doctest coverage is not included by default (often requires nightly); document this explicitly to avoid confusion.
- Large workspaces can produce big HTML artifacts; set retention or upload only LCOV if artifact size becomes an issue.
- PR comment workflows can fail on forks due to token permission restrictions; ensure the workflow safely skips those cases.

## Rollback plan
- Remove the `coverage` job and related steps from `.github/workflows/ci.yml`.
- Remove the “Coverage (optional)” section from `DEVELOPMENT.md` and any badge changes from `README.md`.
